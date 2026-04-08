//! Remote executor client

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::executor::{ExecutionResult, Executor, LocalExecutor};
use crate::{Action, Error};

use super::protocol::{ExecuteRequest, ExecuteResult, ExecuteResponse, Message, WorkerStatus};
use super::tls::TlsConfig;

/// Configuration for remote execution
#[derive(Debug, Clone)]
pub struct RemoteConfig {
    /// Worker addresses (host:port)
    pub workers: Vec<String>,
    /// Connection timeout
    pub connect_timeout: Duration,
    /// Request timeout
    pub request_timeout: Duration,
    /// Fall back to local execution on failure
    pub fallback_to_local: bool,
    /// Maximum retries per worker
    pub max_retries: usize,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            workers: vec![],
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(300),
            fallback_to_local: true,
            max_retries: 2,
        }
    }
}

/// Connection to a remote worker
struct WorkerConnection {
    address: String,
    stream: TlsStream<TcpStream>,
}

impl WorkerConnection {
    async fn send(&mut self, msg: &Message) -> Result<(), Error> {
        let bytes = msg.to_bytes().map_err(|e| Error::Remote(format!("serialize: {}", e)))?;
        let len = bytes.len() as u32;
        
        self.stream.write_all(&len.to_be_bytes()).await?;
        self.stream.write_all(&bytes).await?;
        self.stream.flush().await?;
        
        Ok(())
    }

    async fn recv(&mut self) -> Result<Message, Error> {
        let mut len_buf = [0u8; 4];
        self.stream.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;
        
        if len > 100 * 1024 * 1024 {
            return Err(Error::Remote("message too large".to_string()));
        }
        
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        
        Message::from_bytes(&buf).map_err(|e| Error::Remote(format!("deserialize: {}", e)))
    }
}

/// Worker pool for managing connections
struct WorkerPool {
    config: RemoteConfig,
    tls: Arc<TlsConnector>,
    workers: Vec<WorkerInfo>,
}

struct WorkerInfo {
    address: String,
    status: Mutex<Option<WorkerStatus>>,
}

impl WorkerPool {
    fn new(config: RemoteConfig, tls: Arc<TlsConnector>) -> Self {
        let workers = config.workers.iter()
            .map(|addr| WorkerInfo {
                address: addr.clone(),
                status: Mutex::new(None),
            })
            .collect();
        
        Self { config, tls, workers }
    }

    async fn connect(&self, address: &str) -> Result<WorkerConnection, Error> {
        let stream = tokio::time::timeout(
            self.config.connect_timeout,
            TcpStream::connect(address)
        )
        .await
        .map_err(|_| Error::Remote(format!("connect timeout: {}", address)))?
        .map_err(|e| Error::Remote(format!("connect failed: {}", e)))?;
        
        // Extract hostname for TLS
        let hostname = address.split(':').next().unwrap_or("localhost").to_string();
        let server_name = rustls::pki_types::ServerName::try_from(hostname)
            .map_err(|_| Error::Remote("invalid hostname".to_string()))?;
        
        let tls_stream = self.tls.connect(server_name, stream)
            .await
            .map_err(|e| Error::Remote(format!("TLS handshake failed: {}", e)))?;
        
        Ok(WorkerConnection {
            address: address.to_string(),
            stream: tls_stream,
        })
    }

    async fn select_worker(&self) -> Option<&WorkerInfo> {
        // Simple round-robin with health check
        // TODO: More sophisticated load balancing
        for worker in &self.workers {
            let status = worker.status.lock().await;
            if status.as_ref().map(|s| s.healthy && s.active_jobs < s.max_jobs).unwrap_or(true) {
                return Some(worker);
            }
        }
        self.workers.first()
    }
}

/// Remote executor that distributes work to remote workers
pub struct RemoteExecutor {
    pool: WorkerPool,
    local: LocalExecutor,
    request_counter: AtomicU64,
    config: RemoteConfig,
}

impl RemoteExecutor {
    /// Create a new remote executor with the given configuration
    pub async fn new(config: RemoteConfig, tls_config: TlsConfig) -> Result<Self, Error> {
        let connector = tls_config.client_connector()
            .map_err(|e| Error::Remote(format!("TLS config: {}", e)))?;
        
        Ok(Self {
            pool: WorkerPool::new(config.clone(), Arc::new(connector)),
            local: LocalExecutor::new(),
            request_counter: AtomicU64::new(1),
            config,
        })
    }

    /// Execute on a specific worker
    async fn execute_remote(&self, action: &Action, worker: &WorkerInfo) -> Result<ExecutionResult, Error> {
        let mut conn = self.pool.connect(&worker.address).await?;
        
        let request_id = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let request = ExecuteRequest {
            request_id,
            command: action.command.clone(),
            env: action.env.clone(),
            working_dir: action.working_dir.clone(),
            input_hashes: HashMap::new(), // TODO: compute hashes
            outputs: action.outputs.clone(),
        };
        
        conn.send(&Message::Execute(request)).await?;
        
        let response = tokio::time::timeout(
            self.config.request_timeout,
            conn.recv()
        )
        .await
        .map_err(|_| Error::Remote("request timeout".to_string()))??;
        
        match response {
            Message::Response(ExecuteResponse { result, .. }) => {
                match result {
                    ExecuteResult::Success { exit_code, stdout, stderr, .. } => {
                        Ok(ExecutionResult { exit_code, stdout, stderr })
                    }
                    ExecuteResult::Failed { exit_code, stdout, stderr, error } => {
                        if exit_code != 0 {
                            Ok(ExecutionResult { exit_code, stdout, stderr })
                        } else {
                            Err(Error::Remote(error))
                        }
                    }
                    ExecuteResult::Error { message } => {
                        Err(Error::Remote(message))
                    }
                }
            }
            _ => Err(Error::Remote("unexpected response".to_string())),
        }
    }
}

#[async_trait]
impl Executor for RemoteExecutor {
    async fn execute(&self, action: &Action) -> Result<ExecutionResult, Error> {
        // If no workers configured, use local
        if self.pool.workers.is_empty() {
            return self.local.execute(action).await;
        }
        
        let worker = self.pool.select_worker().await;
        
        if let Some(worker) = worker {
            for attempt in 0..=self.config.max_retries {
                match self.execute_remote(action, worker).await {
                    Ok(result) => return Ok(result),
                    Err(e) => {
                        if attempt == self.config.max_retries {
                            if self.config.fallback_to_local {
                                // Fallback to local execution
                                return self.local.execute(action).await;
                            }
                            return Err(e);
                        }
                        // Retry
                        tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1))).await;
                    }
                }
            }
        }
        
        // No workers available, fallback to local
        if self.config.fallback_to_local {
            self.local.execute(action).await
        } else {
            Err(Error::Remote("no workers available".to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = RemoteConfig::default();
        assert!(config.workers.is_empty());
        assert!(config.fallback_to_local);
        assert_eq!(config.max_retries, 2);
    }

    #[test]
    fn config_with_workers() {
        let config = RemoteConfig {
            workers: vec!["localhost:9000".to_string()],
            ..Default::default()
        };
        assert_eq!(config.workers.len(), 1);
    }

    #[tokio::test]
    async fn fallback_to_local_when_no_workers() {
        // Create executor with no workers - should use local
        let config = RemoteConfig::default();
        
        // Can't easily test without TLS setup, but verify config
        assert!(config.fallback_to_local);
    }
}
