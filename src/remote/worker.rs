//! Remote worker service

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

use crate::executor::{Executor, LocalExecutor};
use crate::{hash_file, hash_to_hex, Action, Error};

use super::protocol::{ExecuteRequest, ExecuteResponse, ExecuteResult, Message, WorkerStatus};
use super::tls::TlsConfig;

/// Configuration for a remote worker
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Bind address
    pub bind_addr: SocketAddr,
    /// Maximum concurrent jobs
    pub max_jobs: usize,
    /// Worker identifier
    pub worker_id: String,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:9000".parse().unwrap(),
            max_jobs: 4,
            worker_id: "worker-default".to_string(),
        }
    }
}

/// Remote worker service
pub struct Worker {
    config: WorkerConfig,
    acceptor: TlsAcceptor,
    executor: LocalExecutor,
    active_jobs: AtomicUsize,
    job_semaphore: Arc<Semaphore>,
}

impl Worker {
    /// Create a new worker with the given configuration
    pub fn new(config: WorkerConfig, tls_config: TlsConfig) -> Result<Self, Error> {
        let acceptor = tls_config.server_acceptor()
            .map_err(|e| Error::Remote(format!("TLS config: {}", e)))?;
        
        Ok(Self {
            job_semaphore: Arc::new(Semaphore::new(config.max_jobs)),
            config,
            acceptor,
            executor: LocalExecutor::new(),
            active_jobs: AtomicUsize::new(0),
        })
    }

    /// Run the worker, accepting connections until shutdown
    pub async fn run(self: Arc<Self>) -> Result<(), Error> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        
        loop {
            let (stream, addr) = listener.accept().await?;
            let worker = Arc::clone(&self);
            
            // Spawn connection handler
            tokio::spawn(async move {
                match worker.handle_connection(stream).await {
                    Ok(()) => {}
                    Err(e) => {
                        eprintln!("Connection error from {}: {}", addr, e);
                    }
                }
            });
        }
    }

    async fn handle_connection(&self, stream: TcpStream) -> Result<(), Error> {
        let mut tls_stream = self.acceptor.accept(stream).await
            .map_err(|e| Error::Remote(format!("TLS accept: {}", e)))?;
        
        loop {
            // Read message length
            let mut len_buf = [0u8; 4];
            match tls_stream.read_exact(&mut len_buf).await {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(()); // Client disconnected
                }
                Err(e) => return Err(e.into()),
            }
            
            let len = u32::from_be_bytes(len_buf) as usize;
            if len > 100 * 1024 * 1024 {
                return Err(Error::Remote("message too large".to_string()));
            }
            
            // Read message body
            let mut buf = vec![0u8; len];
            tls_stream.read_exact(&mut buf).await?;
            
            let msg = Message::from_bytes(&buf)
                .map_err(|e| Error::Remote(format!("deserialize: {}", e)))?;
            
            let response = self.handle_message(msg).await?;
            
            // Send response
            let response_bytes = response.to_bytes()
                .map_err(|e| Error::Remote(format!("serialize: {}", e)))?;
            let resp_len = response_bytes.len() as u32;
            
            tls_stream.write_all(&resp_len.to_be_bytes()).await?;
            tls_stream.write_all(&response_bytes).await?;
            tls_stream.flush().await?;
        }
    }

    async fn handle_message(&self, msg: Message) -> Result<Message, Error> {
        match msg {
            Message::Execute(request) => {
                // Acquire semaphore permit
                let _permit = self.job_semaphore.acquire().await
                    .map_err(|_| Error::Remote("semaphore closed".to_string()))?;
                
                self.active_jobs.fetch_add(1, Ordering::Relaxed);
                let result = self.execute_request(request.clone()).await;
                self.active_jobs.fetch_sub(1, Ordering::Relaxed);
                
                let response = ExecuteResponse {
                    request_id: request.request_id,
                    result,
                };
                
                Ok(Message::Response(response))
            }
            Message::StatusRequest => {
                let status = WorkerStatus {
                    worker_id: self.config.worker_id.clone(),
                    active_jobs: self.active_jobs.load(Ordering::Relaxed),
                    max_jobs: self.config.max_jobs,
                    healthy: true,
                };
                Ok(Message::Status(status))
            }
            Message::Ping => Ok(Message::Pong),
            _ => Err(Error::Remote("unexpected message type".to_string())),
        }
    }

    /// Compute SHA-256 hashes for all output files
    fn compute_output_hashes(outputs: &[PathBuf]) -> HashMap<PathBuf, String> {
        let mut hashes = HashMap::new();
        for output in outputs {
            if output.exists() {
                if let Ok(hash) = hash_file(output) {
                    hashes.insert(output.clone(), hash_to_hex(&hash));
                }
            }
        }
        hashes
    }

    async fn execute_request(&self, request: ExecuteRequest) -> ExecuteResult {
        let action = Action {
            id: request.request_id,
            command: request.command,
            inputs: vec![],
            outputs: request.outputs.clone(),
            env: request.env,
            working_dir: request.working_dir,
        };

        match self.executor.execute(&action).await {
            Ok(result) => {
                if result.success() {
                    // Compute hashes of all output files after execution
                    let output_hashes = Self::compute_output_hashes(&request.outputs);
                    ExecuteResult::Success {
                        exit_code: result.exit_code,
                        stdout: result.stdout,
                        stderr: result.stderr,
                        output_hashes,
                    }
                } else {
                    ExecuteResult::Failed {
                        exit_code: result.exit_code,
                        stdout: result.stdout,
                        stderr: result.stderr,
                        error: "command failed".to_string(),
                    }
                }
            }
            Err(e) => ExecuteResult::Error {
                message: e.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn default_worker_config() {
        let config = WorkerConfig::default();
        assert_eq!(config.max_jobs, 4);
        assert_eq!(config.worker_id, "worker-default");
    }

    #[test]
    fn custom_worker_config() {
        let config = WorkerConfig {
            bind_addr: "127.0.0.1:8080".parse().unwrap(),
            max_jobs: 8,
            worker_id: "my-worker".to_string(),
        };
        assert_eq!(config.max_jobs, 8);
        assert_eq!(config.worker_id, "my-worker");
    }

    // TASK 27 Tests: Output Hash Computation

    #[test]
    fn compute_output_hashes_with_existing_files() {
        let dir = tempdir().unwrap();
        let out1 = dir.path().join("out1.o");
        let out2 = dir.path().join("out2.o");

        std::fs::write(&out1, b"output content 1").unwrap();
        std::fs::write(&out2, b"output content 2").unwrap();

        let outputs = vec![out1.clone(), out2.clone()];
        let hashes = Worker::compute_output_hashes(&outputs);

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains_key(&out1));
        assert!(hashes.contains_key(&out2));
        // Verify hashes are 64-char hex strings (SHA-256)
        for hash in hashes.values() {
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn compute_output_hashes_skips_missing_files() {
        let outputs = vec![
            PathBuf::from("/nonexistent/out1.o"),
            PathBuf::from("/nonexistent/out2.o"),
        ];
        let hashes = Worker::compute_output_hashes(&outputs);
        assert!(hashes.is_empty());
    }

    #[test]
    fn compute_output_hashes_empty_outputs() {
        let outputs: Vec<PathBuf> = vec![];
        let hashes = Worker::compute_output_hashes(&outputs);
        assert!(hashes.is_empty());
    }

    #[test]
    fn compute_output_hashes_same_content_same_hash() {
        let dir = tempdir().unwrap();
        let out1 = dir.path().join("out1.o");
        let out2 = dir.path().join("out2.o");

        std::fs::write(&out1, b"identical content").unwrap();
        std::fs::write(&out2, b"identical content").unwrap();

        let outputs = vec![out1.clone(), out2.clone()];
        let hashes = Worker::compute_output_hashes(&outputs);

        let h1 = hashes.get(&out1).unwrap();
        let h2 = hashes.get(&out2).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn compute_output_hashes_different_content_different_hash() {
        let dir = tempdir().unwrap();
        let out1 = dir.path().join("out1.o");
        let out2 = dir.path().join("out2.o");

        std::fs::write(&out1, b"content A").unwrap();
        std::fs::write(&out2, b"content B").unwrap();

        let outputs = vec![out1.clone(), out2.clone()];
        let hashes = Worker::compute_output_hashes(&outputs);

        let h1 = hashes.get(&out1).unwrap();
        let h2 = hashes.get(&out2).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn compute_output_hashes_partial_files() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("exists.o");
        let missing = dir.path().join("missing.o");

        std::fs::write(&existing, b"content").unwrap();

        let outputs = vec![existing.clone(), missing.clone()];
        let hashes = Worker::compute_output_hashes(&outputs);

        assert_eq!(hashes.len(), 1);
        assert!(hashes.contains_key(&existing));
        assert!(!hashes.contains_key(&missing));
    }
}
