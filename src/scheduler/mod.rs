//! Build scheduling and parallel execution

mod worker;

pub use worker::{PoolHandle, Task, TaskResult, WorkerPool, WorkerPoolRunner};
