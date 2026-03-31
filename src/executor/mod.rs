//! Action execution infrastructure

mod local;

pub use local::{execute_sync, ExecutionResult, Executor, LocalExecutor};
