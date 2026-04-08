//! Remote execution infrastructure
//!
//! Distributes build actions across remote worker machines using mTLS.
//!
//! # Architecture
//!
//! - **Client (RemoteExecutor)**: Implements `Executor` trait, connects to workers
//! - **Worker**: Service that accepts connections and executes actions locally
//! - **Distributor**: Automatic work distribution with load balancing
//! - **Protocol**: Binary messages for execute requests, responses, status
//! - **TLS**: mTLS configuration for secure communication
//!
//! # Example
//!
//! ```ignore
//! // Client side
//! let config = RemoteConfig {
//!     workers: vec!["worker1:9000".into(), "worker2:9000".into()],
//!     fallback_to_local: true,
//!     ..Default::default()
//! };
//! let tls = TlsConfig::load("client.pem", "client-key.pem", "ca.pem")?;
//! let executor = RemoteExecutor::new(config, tls).await?;
//!
//! // Worker side
//! let config = WorkerConfig { max_jobs: 4, ..Default::default() };
//! let tls = TlsConfig::load("worker.pem", "worker-key.pem", "ca.pem")?;
//! let worker = Worker::new(config, tls)?;
//! worker.run().await?;
//! ```

mod client;
mod distributor;
mod protocol;
mod tls;
mod worker;

pub use client::{RemoteConfig, RemoteExecutor};
pub use distributor::{DistributionStrategy, Distributor, WorkerState};
pub use protocol::{ExecuteRequest, ExecuteResponse, ExecuteResult, Message, WorkerStatus};
pub use tls::{TlsConfig, TlsError};
pub use worker::{Worker, WorkerConfig};
