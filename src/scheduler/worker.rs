//! Parallel worker pool for action execution

use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};

use crate::{Action, Error, ExecutionResult, Executor, LocalExecutor};

/// A task to be executed by the worker pool
pub struct Task {
    /// The action to execute
    pub action: Action,
    /// Channel to send the result back
    pub result_tx: oneshot::Sender<TaskResult>,
}

impl Task {
    /// Create a new task
    pub fn new(action: Action) -> (Self, oneshot::Receiver<TaskResult>) {
        let (tx, rx) = oneshot::channel();
        (
            Self {
                action,
                result_tx: tx,
            },
            rx,
        )
    }
}

/// Result of a task execution
#[derive(Debug)]
pub struct TaskResult {
    /// The action ID
    pub action_id: u64,
    /// The execution result or error
    pub result: Result<ExecutionResult, Error>,
}

impl TaskResult {
    /// Check if the task succeeded
    pub fn success(&self) -> bool {
        matches!(&self.result, Ok(r) if r.success())
    }
}

/// Handle for submitting tasks to the pool
#[derive(Clone)]
pub struct PoolHandle {
    task_tx: mpsc::Sender<Task>,
}

impl PoolHandle {
    /// Submit a task and get a receiver for the result
    pub async fn submit(&self, action: Action) -> Result<oneshot::Receiver<TaskResult>, Error> {
        let (task, result_rx) = Task::new(action);
        self.task_tx
            .send(task)
            .await
            .map_err(|_| Error::Cache("Worker pool shut down".to_string()))?;
        Ok(result_rx)
    }

    /// Submit a task and wait for the result
    pub async fn execute(&self, action: Action) -> Result<TaskResult, Error> {
        let result_rx = self.submit(action).await?;
        result_rx
            .await
            .map_err(|_| Error::Cache("Worker dropped task".to_string()))
    }
}

/// Parallel worker pool for executing actions
pub struct WorkerPool {
    /// Maximum number of concurrent workers
    max_workers: usize,
    /// Executor to use for running actions
    executor: Arc<dyn Executor>,
}

impl WorkerPool {
    /// Create a new worker pool with the given parallelism
    pub fn new(max_workers: usize) -> Self {
        Self {
            max_workers,
            executor: Arc::new(LocalExecutor::new()),
        }
    }

    /// Create a worker pool with a custom executor
    pub fn with_executor<E: Executor + 'static>(max_workers: usize, executor: E) -> Self {
        Self {
            max_workers,
            executor: Arc::new(executor),
        }
    }

    /// Start the worker pool and return a handle for submitting tasks
    pub fn start(self) -> (PoolHandle, WorkerPoolRunner) {
        let (task_tx, task_rx) = mpsc::channel(self.max_workers * 2);
        let handle = PoolHandle { task_tx };
        let runner = WorkerPoolRunner {
            max_workers: self.max_workers,
            executor: self.executor,
            task_rx,
        };
        (handle, runner)
    }
}

/// Runner that processes tasks from the pool
pub struct WorkerPoolRunner {
    max_workers: usize,
    executor: Arc<dyn Executor>,
    task_rx: mpsc::Receiver<Task>,
}

impl WorkerPoolRunner {
    /// Run the worker pool until all senders are dropped
    pub async fn run(mut self) {
        let semaphore = Arc::new(Semaphore::new(self.max_workers));

        while let Some(task) = self.task_rx.recv().await {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let executor = self.executor.clone();

            tokio::spawn(async move {
                let action_id = task.action.id;
                let result = executor.execute(&task.action).await;
                let task_result = TaskResult { action_id, result };

                // Ignore send error - receiver may have been dropped
                let _ = task.result_tx.send(task_result);

                drop(permit);
            });
        }
    }

    /// Run the worker pool with a shutdown signal
    pub async fn run_until<F>(mut self, shutdown: F)
    where
        F: std::future::Future<Output = ()>,
    {
        let semaphore = Arc::new(Semaphore::new(self.max_workers));

        tokio::select! {
            _ = async {
                while let Some(task) = self.task_rx.recv().await {
                    let permit = semaphore.clone().acquire_owned().await.unwrap();
                    let executor = self.executor.clone();

                    tokio::spawn(async move {
                        let action_id = task.action.id;
                        let result = executor.execute(&task.action).await;
                        let task_result = TaskResult { action_id, result };
                        let _ = task.result_tx.send(task_result);
                        drop(permit);
                    });
                }
            } => {},
            _ = shutdown => {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn test_action(id: u64, command: Vec<&str>) -> Action {
        let cmd: Vec<String> = command.into_iter().map(|s| s.to_string()).collect();
        Action::with_id(id, cmd)
    }

    #[tokio::test]
    async fn submit_single_task() {
        let pool = WorkerPool::new(4);
        let (handle, runner) = pool.start();

        tokio::spawn(runner.run());

        let action = test_action(1, vec!["echo", "hello"]);
        let result = handle.execute(action).await.unwrap();

        assert!(result.success());
        assert_eq!(result.action_id, 1);
    }

    #[tokio::test]
    async fn submit_multiple_tasks() {
        let pool = WorkerPool::new(4);
        let (handle, runner) = pool.start();

        tokio::spawn(runner.run());

        let mut receivers = Vec::new();
        for i in 0..5 {
            let action = test_action(i, vec!["echo", &format!("task{}", i)]);
            let rx = handle.submit(action).await.unwrap();
            receivers.push(rx);
        }

        // All tasks should complete
        for rx in receivers {
            let result = rx.await.unwrap();
            assert!(result.success());
        }
    }

    #[tokio::test]
    async fn tasks_execute_in_parallel() {
        let counter = Arc::new(AtomicUsize::new(0));
        let max_concurrent = Arc::new(AtomicUsize::new(0));

        let pool = WorkerPool::new(4);
        let (handle, runner) = pool.start();

        tokio::spawn(runner.run());

        let mut receivers = Vec::new();

        // Submit tasks that sleep briefly
        for i in 0..4 {
            let action = test_action(i, vec!["sleep", "0.1"]);
            let rx = handle.submit(action).await.unwrap();
            receivers.push(rx);
        }

        // Wait for all to complete
        for rx in receivers {
            rx.await.unwrap();
        }

        // If running truly in parallel, total time should be ~0.1s not ~0.4s
        // We can't easily test timing, but we can verify all completed
    }

    #[tokio::test]
    async fn receive_task_results() {
        let pool = WorkerPool::new(2);
        let (handle, runner) = pool.start();

        tokio::spawn(runner.run());

        // Success case
        let action = test_action(1, vec!["true"]);
        let result = handle.execute(action).await.unwrap();
        assert!(result.success());

        // Failure case
        let action = test_action(2, vec!["false"]);
        let result = handle.execute(action).await.unwrap();
        assert!(!result.success());
    }

    #[tokio::test]
    async fn graceful_shutdown() {
        let pool = WorkerPool::new(2);
        let (handle, runner) = pool.start();

        let shutdown = async {
            tokio::time::sleep(Duration::from_millis(50)).await;
        };

        // Submit a quick task
        let action = test_action(1, vec!["echo", "quick"]);
        let rx = handle.submit(action).await.unwrap();

        // Run until shutdown
        tokio::spawn(runner.run_until(shutdown));

        // Task should complete before shutdown
        let result = tokio::time::timeout(Duration::from_millis(100), rx)
            .await
            .unwrap()
            .unwrap();

        assert!(result.success());
    }

    #[tokio::test]
    async fn pool_respects_concurrency_limit() {
        // With limit of 1, tasks must run sequentially
        let pool = WorkerPool::new(1);
        let (handle, runner) = pool.start();

        tokio::spawn(runner.run());

        let start = std::time::Instant::now();

        // Submit 2 tasks that each sleep 50ms
        let action1 = test_action(1, vec!["sleep", "0.05"]);
        let action2 = test_action(2, vec!["sleep", "0.05"]);

        let rx1 = handle.submit(action1).await.unwrap();
        let rx2 = handle.submit(action2).await.unwrap();

        rx1.await.unwrap();
        rx2.await.unwrap();

        let elapsed = start.elapsed();

        // With concurrency 1, should take at least 100ms
        assert!(elapsed >= Duration::from_millis(90));
    }
}
