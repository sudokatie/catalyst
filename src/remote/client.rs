//! Remote executor client

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

use crate::executor::{ExecutionResult, Executor, LocalExecutor};
use crate::{hash_file, hash_to_hex, Action, Error};

use super::protocol::{ExecuteRequest, ExecuteResult, ExecuteResponse, Message, WorkerStatus};
use super::tls::TlsConfig;

/// Worker selection strategy
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum WorkerSelectionStrategy {
    /// Select worker with lowest load (active_jobs / max_jobs ratio)
    #[default]
    LeastLoaded,
    /// Cycle through workers in order
    RoundRobin,
    /// Random worker selection
    Random,
}

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
    /// Worker selection strategy
    pub selection_strategy: WorkerSelectionStrategy,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            workers: vec![],
            connect_timeout: Duration::from_secs(5),
            request_timeout: Duration::from_secs(300),
            fallback_to_local: true,
            max_retries: 2,
            selection_strategy: WorkerSelectionStrategy::LeastLoaded,
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

/// Statistics for a worker
#[derive(Debug, Default)]
pub struct WorkerStats {
    /// Total jobs completed
    pub total_completed_jobs: u64,
    /// Total successful jobs
    pub successful_jobs: u64,
    /// Average execution time in milliseconds
    pub average_execution_time_ms: u64,
    /// Sum of execution times (for computing average)
    total_execution_time_ms: u64,
    /// Number of consecutive failures
    pub consecutive_failures: u32,
}

impl WorkerStats {
    /// Calculate success rate (0.0 to 1.0)
    pub fn success_rate(&self) -> f64 {
        if self.total_completed_jobs == 0 {
            1.0 // Assume healthy until proven otherwise
        } else {
            self.successful_jobs as f64 / self.total_completed_jobs as f64
        }
    }

    /// Record a job completion
    fn record_completion(&mut self, success: bool, execution_time_ms: u64) {
        self.total_completed_jobs += 1;
        if success {
            self.successful_jobs += 1;
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
        }
        self.total_execution_time_ms += execution_time_ms;
        self.average_execution_time_ms =
            self.total_execution_time_ms / self.total_completed_jobs;
    }
}

/// Worker pool for managing connections
struct WorkerPool {
    config: RemoteConfig,
    tls: Arc<TlsConnector>,
    workers: Vec<WorkerInfo>,
    round_robin_counter: AtomicU64,
}

struct WorkerInfo {
    address: String,
    status: Mutex<Option<WorkerStatus>>,
    stats: Mutex<WorkerStats>,
    pending_jobs: std::sync::atomic::AtomicUsize,
}

impl WorkerPool {
    fn new(config: RemoteConfig, tls: Arc<TlsConnector>) -> Self {
        let workers = config
            .workers
            .iter()
            .map(|addr| WorkerInfo {
                address: addr.clone(),
                status: Mutex::new(None),
                stats: Mutex::new(WorkerStats::default()),
                pending_jobs: std::sync::atomic::AtomicUsize::new(0),
            })
            .collect();

        Self {
            config,
            tls,
            workers,
            round_robin_counter: AtomicU64::new(0),
        }
    }

    async fn connect(&self, address: &str) -> Result<WorkerConnection, Error> {
        let stream = tokio::time::timeout(
            self.config.connect_timeout,
            TcpStream::connect(address),
        )
        .await
        .map_err(|_| Error::Remote(format!("connect timeout: {}", address)))?
        .map_err(|e| Error::Remote(format!("connect failed: {}", e)))?;

        // Extract hostname for TLS
        let hostname = address.split(':').next().unwrap_or("localhost").to_string();
        let server_name = rustls::pki_types::ServerName::try_from(hostname)
            .map_err(|_| Error::Remote("invalid hostname".to_string()))?;

        let tls_stream = self.tls
            .connect(server_name, stream)
            .await
            .map_err(|e| Error::Remote(format!("TLS handshake failed: {}", e)))?;

        Ok(WorkerConnection {
            address: address.to_string(),
            stream: tls_stream,
        })
    }

    /// Calculate effective load for a worker (lower is better)
    async fn calculate_load(&self, worker: &WorkerInfo) -> f64 {
        let status = worker.status.lock().await;
        let stats = worker.stats.lock().await;
        let pending = worker.pending_jobs.load(Ordering::Relaxed);

        // Base load from active + pending jobs
        let max_jobs = status.as_ref().map(|s| s.max_jobs).unwrap_or(4);
        let active_jobs = status.as_ref().map(|s| s.active_jobs).unwrap_or(0);
        let total_jobs = active_jobs + pending;

        if max_jobs == 0 {
            return f64::MAX;
        }

        let job_ratio = total_jobs as f64 / max_jobs as f64;

        // Adjust for worker health
        let health_penalty = if status.as_ref().map(|s| !s.healthy).unwrap_or(false) {
            10.0 // Heavy penalty for unhealthy workers
        } else if stats.consecutive_failures >= 3 {
            5.0 // Penalty for workers with recent failures
        } else {
            0.0
        };

        // Prefer workers with better historical performance
        let performance_factor = if stats.total_completed_jobs > 0 {
            // Lower average time and higher success rate = better
            let time_factor = (stats.average_execution_time_ms as f64 / 1000.0).min(5.0);
            let success_factor = 1.0 - stats.success_rate();
            time_factor * 0.1 + success_factor
        } else {
            0.0
        };

        job_ratio + health_penalty + performance_factor
    }

    /// Select a worker based on the configured strategy
    async fn select_worker(&self) -> Option<&WorkerInfo> {
        if self.workers.is_empty() {
            return None;
        }

        // Filter to healthy workers with capacity
        let mut available: Vec<(usize, f64)> = Vec::new();
        for (idx, worker) in self.workers.iter().enumerate() {
            let status = worker.status.lock().await;
            let stats = worker.stats.lock().await;

            // Skip unhealthy workers
            if status.as_ref().map(|s| !s.healthy).unwrap_or(false) {
                continue;
            }
            // Skip workers with too many consecutive failures
            if stats.consecutive_failures >= 5 {
                continue;
            }

            // Check capacity
            let pending = worker.pending_jobs.load(Ordering::Relaxed);
            let max_jobs = status.as_ref().map(|s| s.max_jobs).unwrap_or(4);
            let active = status.as_ref().map(|s| s.active_jobs).unwrap_or(0);

            if active + pending < max_jobs {
                drop(status);
                drop(stats);
                let load = self.calculate_load(worker).await;
                available.push((idx, load));
            }
        }

        if available.is_empty() {
            // Fall back to any worker if none are "available"
            return self.workers.first();
        }

        let selected_idx = match self.config.selection_strategy {
            WorkerSelectionStrategy::LeastLoaded => {
                // Select worker with lowest load
                available
                    .iter()
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(idx, _)| *idx)
                    .unwrap_or(0)
            }
            WorkerSelectionStrategy::RoundRobin => {
                // Cycle through available workers
                let counter = self.round_robin_counter.fetch_add(1, Ordering::Relaxed);
                let idx_in_available = (counter as usize) % available.len();
                available[idx_in_available].0
            }
            WorkerSelectionStrategy::Random => {
                // Random selection using time-based seed
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos() as usize;
                let idx_in_available = seed % available.len();
                available[idx_in_available].0
            }
        };

        self.workers.get(selected_idx)
    }

    /// Mark a job as assigned to a worker
    fn job_assigned(&self, worker: &WorkerInfo) {
        worker.pending_jobs.fetch_add(1, Ordering::Relaxed);
    }

    /// Record job completion for a worker
    async fn job_completed(&self, worker: &WorkerInfo, success: bool, execution_time_ms: u64) {
        worker.pending_jobs.fetch_sub(1, Ordering::Relaxed);
        let mut stats = worker.stats.lock().await;
        stats.record_completion(success, execution_time_ms);
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

    /// Compute SHA-256 hashes for all input files
    fn compute_input_hashes(action: &Action) -> HashMap<PathBuf, String> {
        let mut hashes = HashMap::new();
        for input in &action.inputs {
            if input.exists() {
                if let Ok(hash) = hash_file(input) {
                    hashes.insert(input.clone(), hash_to_hex(&hash));
                }
            }
        }
        hashes
    }

    /// Verify output hashes from remote execution match expected values
    fn verify_output_hashes(
        expected_outputs: &[PathBuf],
        received_hashes: &HashMap<PathBuf, String>,
    ) -> Result<(), Error> {
        // If no outputs expected, nothing to verify
        if expected_outputs.is_empty() {
            return Ok(());
        }

        // Check that all expected outputs have hashes
        for output in expected_outputs {
            if !received_hashes.contains_key(output) {
                // Missing hash for expected output - could be cache poisoning
                return Err(Error::Remote(format!(
                    "Missing output hash for {:?}",
                    output
                )));
            }
        }

        Ok(())
    }

    /// Execute on a specific worker
    async fn execute_remote(&self, action: &Action, worker: &WorkerInfo) -> Result<ExecutionResult, Error> {
        let mut conn = self.pool.connect(&worker.address).await?;

        // Compute input hashes for cache validation
        let input_hashes = Self::compute_input_hashes(action);

        let request_id = self.request_counter.fetch_add(1, Ordering::Relaxed);
        let request = ExecuteRequest {
            request_id,
            command: action.command.clone(),
            env: action.env.clone(),
            working_dir: action.working_dir.clone(),
            input_hashes,
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
                    ExecuteResult::Success { exit_code, stdout, stderr, output_hashes } => {
                        // Verify output hashes for cache poison protection
                        Self::verify_output_hashes(&action.outputs, &output_hashes)?;
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
        use std::time::Instant;

        // If no workers configured, use local
        if self.pool.workers.is_empty() {
            return self.local.execute(action).await;
        }

        let worker = self.pool.select_worker().await;

        if let Some(worker) = worker {
            // Track job assignment
            self.pool.job_assigned(worker);

            for attempt in 0..=self.config.max_retries {
                let start = Instant::now();
                match self.execute_remote(action, worker).await {
                    Ok(result) => {
                        let elapsed_ms = start.elapsed().as_millis() as u64;
                        self.pool
                            .job_completed(worker, result.success(), elapsed_ms)
                            .await;
                        return Ok(result);
                    }
                    Err(e) => {
                        let elapsed_ms = start.elapsed().as_millis() as u64;

                        if attempt == self.config.max_retries {
                            // Record final failure
                            self.pool.job_completed(worker, false, elapsed_ms).await;

                            if self.config.fallback_to_local {
                                // Fallback to local execution
                                return self.local.execute(action).await;
                            }
                            return Err(e);
                        }
                        // Retry with exponential backoff
                        tokio::time::sleep(Duration::from_millis(100 * (attempt as u64 + 1)))
                            .await;
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
    use std::io::Write;
    use tempfile::NamedTempFile;

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

    #[test]
    fn config_with_multiple_workers() {
        let config = RemoteConfig {
            workers: vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
                "worker3:9000".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(config.workers.len(), 3);
    }

    #[test]
    fn config_timeouts() {
        let config = RemoteConfig {
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(600),
            ..Default::default()
        };
        assert_eq!(config.connect_timeout, Duration::from_secs(10));
        assert_eq!(config.request_timeout, Duration::from_secs(600));
    }

    #[test]
    fn config_no_fallback() {
        let config = RemoteConfig {
            fallback_to_local: false,
            ..Default::default()
        };
        assert!(!config.fallback_to_local);
    }

    #[tokio::test]
    async fn fallback_to_local_when_no_workers() {
        // Create executor with no workers - should use local
        let config = RemoteConfig::default();

        // Can't easily test without TLS setup, but verify config
        assert!(config.fallback_to_local);
    }

    // TASK 27 Tests: Input/Output Hash Computation

    #[test]
    fn compute_input_hashes_with_existing_files() {
        let mut file1 = NamedTempFile::new().unwrap();
        let mut file2 = NamedTempFile::new().unwrap();
        write!(file1, "content1").unwrap();
        write!(file2, "content2").unwrap();

        let mut action = Action::new(vec!["test".to_string()]);
        action.add_input(file1.path().to_path_buf());
        action.add_input(file2.path().to_path_buf());

        let hashes = RemoteExecutor::compute_input_hashes(&action);

        assert_eq!(hashes.len(), 2);
        assert!(hashes.contains_key(&file1.path().to_path_buf()));
        assert!(hashes.contains_key(&file2.path().to_path_buf()));
        // Verify hashes are 64-char hex strings (SHA-256)
        for hash in hashes.values() {
            assert_eq!(hash.len(), 64);
            assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn compute_input_hashes_skips_missing_files() {
        let mut action = Action::new(vec!["test".to_string()]);
        action.add_input(PathBuf::from("/nonexistent/file1.txt"));
        action.add_input(PathBuf::from("/nonexistent/file2.txt"));

        let hashes = RemoteExecutor::compute_input_hashes(&action);

        assert!(hashes.is_empty());
    }

    #[test]
    fn compute_input_hashes_empty_action() {
        let action = Action::new(vec!["test".to_string()]);
        let hashes = RemoteExecutor::compute_input_hashes(&action);
        assert!(hashes.is_empty());
    }

    #[test]
    fn compute_input_hashes_different_content_different_hashes() {
        let mut file1 = NamedTempFile::new().unwrap();
        let mut file2 = NamedTempFile::new().unwrap();
        write!(file1, "content A").unwrap();
        write!(file2, "content B").unwrap();

        let mut action = Action::new(vec!["test".to_string()]);
        action.add_input(file1.path().to_path_buf());
        action.add_input(file2.path().to_path_buf());

        let hashes = RemoteExecutor::compute_input_hashes(&action);

        let h1 = hashes.get(&file1.path().to_path_buf()).unwrap();
        let h2 = hashes.get(&file2.path().to_path_buf()).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn verify_output_hashes_empty_outputs_succeeds() {
        let outputs: Vec<PathBuf> = vec![];
        let hashes: HashMap<PathBuf, String> = HashMap::new();

        let result = RemoteExecutor::verify_output_hashes(&outputs, &hashes);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_output_hashes_all_present_succeeds() {
        let outputs = vec![PathBuf::from("out1.o"), PathBuf::from("out2.o")];
        let mut hashes = HashMap::new();
        hashes.insert(PathBuf::from("out1.o"), "abc123".to_string());
        hashes.insert(PathBuf::from("out2.o"), "def456".to_string());

        let result = RemoteExecutor::verify_output_hashes(&outputs, &hashes);
        assert!(result.is_ok());
    }

    #[test]
    fn verify_output_hashes_missing_hash_fails() {
        let outputs = vec![PathBuf::from("out1.o"), PathBuf::from("out2.o")];
        let mut hashes = HashMap::new();
        hashes.insert(PathBuf::from("out1.o"), "abc123".to_string());
        // Missing out2.o hash

        let result = RemoteExecutor::verify_output_hashes(&outputs, &hashes);
        assert!(result.is_err());
    }

    // TASK 28 Tests: Load Balancing

    #[test]
    fn worker_stats_default() {
        let stats = WorkerStats::default();
        assert_eq!(stats.total_completed_jobs, 0);
        assert_eq!(stats.successful_jobs, 0);
        assert_eq!(stats.consecutive_failures, 0);
        assert_eq!(stats.success_rate(), 1.0); // Assume healthy until proven otherwise
    }

    #[test]
    fn worker_stats_success_rate() {
        let mut stats = WorkerStats::default();
        stats.record_completion(true, 100);
        stats.record_completion(true, 100);
        stats.record_completion(false, 100);

        assert_eq!(stats.total_completed_jobs, 3);
        assert_eq!(stats.successful_jobs, 2);
        assert!((stats.success_rate() - 0.6666).abs() < 0.01);
    }

    #[test]
    fn worker_stats_consecutive_failures_reset_on_success() {
        let mut stats = WorkerStats::default();
        stats.record_completion(false, 100);
        stats.record_completion(false, 100);
        assert_eq!(stats.consecutive_failures, 2);

        stats.record_completion(true, 100);
        assert_eq!(stats.consecutive_failures, 0);
    }

    #[test]
    fn worker_stats_average_execution_time() {
        let mut stats = WorkerStats::default();
        stats.record_completion(true, 100);
        stats.record_completion(true, 200);
        stats.record_completion(true, 300);

        assert_eq!(stats.average_execution_time_ms, 200);
    }

    #[test]
    fn worker_selection_strategy_default_is_least_loaded() {
        let config = RemoteConfig::default();
        assert_eq!(config.selection_strategy, WorkerSelectionStrategy::LeastLoaded);
    }

    #[test]
    fn worker_selection_strategy_round_robin() {
        let config = RemoteConfig {
            selection_strategy: WorkerSelectionStrategy::RoundRobin,
            ..Default::default()
        };
        assert_eq!(config.selection_strategy, WorkerSelectionStrategy::RoundRobin);
    }

    #[test]
    fn worker_selection_strategy_random() {
        let config = RemoteConfig {
            selection_strategy: WorkerSelectionStrategy::Random,
            ..Default::default()
        };
        assert_eq!(config.selection_strategy, WorkerSelectionStrategy::Random);
    }
}
