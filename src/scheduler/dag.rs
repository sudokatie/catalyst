//! DAG-based scheduler for executing actions in dependency order

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;

use super::worker::{PoolHandle, WorkerPool};
use crate::{Action, ActionId, Error, ExecutionResult};

/// Result of a build operation
#[derive(Debug, Default)]
pub struct BuildResult {
    /// Number of actions executed
    pub executed: usize,
    /// Number of actions that succeeded
    pub succeeded: usize,
    /// Number of actions that failed
    pub failed: usize,
    /// Number of actions skipped (due to dependency failure)
    pub skipped: usize,
    /// Action results keyed by action ID
    pub results: HashMap<ActionId, Result<ExecutionResult, String>>,
}

impl BuildResult {
    /// Check if the build succeeded (no failures)
    pub fn success(&self) -> bool {
        self.failed == 0
    }

    /// Get total number of actions
    pub fn total(&self) -> usize {
        self.executed + self.skipped
    }
}

/// An action with its dependencies
#[derive(Debug, Clone)]
pub struct ScheduledAction {
    /// The action to execute
    pub action: Action,
    /// IDs of actions this depends on
    pub dependencies: Vec<ActionId>,
}

impl ScheduledAction {
    /// Create a new scheduled action
    pub fn new(action: Action) -> Self {
        Self {
            action,
            dependencies: Vec::new(),
        }
    }

    /// Add a dependency
    pub fn with_dep(mut self, dep: ActionId) -> Self {
        self.dependencies.push(dep);
        self
    }

    /// Add multiple dependencies
    pub fn with_deps(mut self, deps: impl IntoIterator<Item = ActionId>) -> Self {
        self.dependencies.extend(deps);
        self
    }
}

/// DAG-based scheduler that respects dependencies
pub struct Scheduler {
    /// Actions to execute
    actions: HashMap<ActionId, ScheduledAction>,
    /// Worker pool handle
    pool_handle: PoolHandle,
    /// Maximum parallelism
    max_parallel: usize,
}

impl Scheduler {
    /// Create a new scheduler with the given parallelism
    pub fn new(max_parallel: usize) -> (Self, super::WorkerPoolRunner) {
        let pool = WorkerPool::new(max_parallel);
        let (handle, runner) = pool.start();
        (
            Self {
                actions: HashMap::new(),
                pool_handle: handle,
                max_parallel,
            },
            runner,
        )
    }

    /// Create a scheduler with an existing pool handle
    pub fn with_pool(pool_handle: PoolHandle, max_parallel: usize) -> Self {
        Self {
            actions: HashMap::new(),
            pool_handle,
            max_parallel,
        }
    }

    /// Add an action to the schedule
    pub fn add(&mut self, scheduled: ScheduledAction) {
        self.actions.insert(scheduled.action.id, scheduled);
    }

    /// Add multiple actions
    pub fn add_all(&mut self, actions: impl IntoIterator<Item = ScheduledAction>) {
        for action in actions {
            self.add(action);
        }
    }

    /// Execute all actions in dependency order
    pub async fn execute(self) -> Result<BuildResult, Error> {
        let mut result = BuildResult::default();

        if self.actions.is_empty() {
            return Ok(result);
        }

        // Build dependency graph
        let mut dependents: HashMap<ActionId, Vec<ActionId>> = HashMap::new();
        let mut dep_counts: HashMap<ActionId, usize> = HashMap::new();

        for (id, scheduled) in &self.actions {
            dep_counts.insert(*id, scheduled.dependencies.len());
            for dep in &scheduled.dependencies {
                dependents.entry(*dep).or_default().push(*id);
            }
        }

        // Find initially ready actions (no dependencies)
        let mut ready: Vec<ActionId> = dep_counts
            .iter()
            .filter(|(_, count)| **count == 0)
            .map(|(id, _)| *id)
            .collect();

        // Shared state for tracking completion
        let completed: Arc<Mutex<HashSet<ActionId>>> = Arc::new(Mutex::new(HashSet::new()));
        let failed: Arc<Mutex<HashSet<ActionId>>> = Arc::new(Mutex::new(HashSet::new()));

        // Process actions in waves
        while !ready.is_empty() || result.executed + result.skipped < self.actions.len() {
            if ready.is_empty() {
                // All remaining actions have unsatisfied deps - circular dependency or failure cascade
                break;
            }

            // Launch all ready actions
            let mut pending = Vec::new();
            for action_id in ready.drain(..) {
                // Check if any dependency failed
                let should_skip = {
                    let failed_set = failed.lock().await;
                    let scheduled = &self.actions[&action_id];
                    scheduled.dependencies.iter().any(|d| failed_set.contains(d))
                };

                if should_skip {
                    result.skipped += 1;
                    result.results.insert(
                        action_id,
                        Err("Skipped due to dependency failure".to_string()),
                    );

                    // Mark as failed so dependents are also skipped
                    failed.lock().await.insert(action_id);
                    completed.lock().await.insert(action_id);

                    // Update dependents
                    if let Some(deps) = dependents.get(&action_id) {
                        for dep_id in deps {
                            if let Some(count) = dep_counts.get_mut(dep_id) {
                                *count = count.saturating_sub(1);
                            }
                        }
                    }
                } else {
                    let action = self.actions[&action_id].action.clone();
                    let rx = self.pool_handle.submit(action).await?;
                    pending.push((action_id, rx));
                }
            }

            // Wait for all pending to complete
            for (action_id, rx) in pending {
                let task_result = rx
                    .await
                    .map_err(|_| Error::Cache("Worker dropped task".to_string()))?;

                result.executed += 1;

                if task_result.success() {
                    result.succeeded += 1;
                    result
                        .results
                        .insert(action_id, Ok(task_result.result.unwrap()));
                } else {
                    result.failed += 1;
                    let err_msg = match task_result.result {
                        Ok(r) => format!("Exit code: {}", r.exit_code),
                        Err(e) => e.to_string(),
                    };
                    result.results.insert(action_id, Err(err_msg));
                    failed.lock().await.insert(action_id);
                }

                completed.lock().await.insert(action_id);

                // Update ready queue with newly unblocked actions
                if let Some(dependent_ids) = dependents.get(&action_id) {
                    for dep_id in dependent_ids {
                        if let Some(count) = dep_counts.get_mut(dep_id) {
                            *count = count.saturating_sub(1);
                            if *count == 0 {
                                ready.push(*dep_id);
                            }
                        }
                    }
                }
            }
        }

        // Mark any remaining actions as skipped
        let completed_set = completed.lock().await;
        for id in self.actions.keys() {
            if !completed_set.contains(id) {
                result.skipped += 1;
                result
                    .results
                    .insert(*id, Err("Skipped due to unmet dependencies".to_string()));
            }
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_action(id: ActionId, command: Vec<&str>) -> Action {
        let cmd: Vec<String> = command.into_iter().map(|s| s.to_string()).collect();
        Action::with_id(id, cmd)
    }

    #[tokio::test]
    async fn execute_single_action() {
        let (mut scheduler, runner) = Scheduler::new(4);
        tokio::spawn(runner.run());

        let action = test_action(1, vec!["echo", "hello"]);
        scheduler.add(ScheduledAction::new(action));

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.executed, 1);
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 0);
        assert!(result.success());
    }

    #[tokio::test]
    async fn execute_in_dependency_order() {
        let (mut scheduler, runner) = Scheduler::new(1); // Force sequential
        tokio::spawn(runner.run());

        // Action 2 depends on action 1
        let action1 = test_action(1, vec!["echo", "first"]);
        let action2 = test_action(2, vec!["echo", "second"]);

        scheduler.add(ScheduledAction::new(action1));
        scheduler.add(ScheduledAction::new(action2).with_dep(1));

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.executed, 2);
        assert_eq!(result.succeeded, 2);
        assert!(result.success());
    }

    #[tokio::test]
    async fn independent_actions_can_run_parallel() {
        let (mut scheduler, runner) = Scheduler::new(4);
        tokio::spawn(runner.run());

        // Four independent actions
        for i in 1..=4 {
            let action = test_action(i, vec!["echo", &format!("action{}", i)]);
            scheduler.add(ScheduledAction::new(action));
        }

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.executed, 4);
        assert_eq!(result.succeeded, 4);
    }

    #[tokio::test]
    async fn track_completed_actions() {
        let (mut scheduler, runner) = Scheduler::new(2);
        tokio::spawn(runner.run());

        scheduler.add(ScheduledAction::new(test_action(1, vec!["true"])));
        scheduler.add(ScheduledAction::new(test_action(2, vec!["false"])));

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.total(), 2);
        assert_eq!(result.succeeded, 1);
        assert_eq!(result.failed, 1);
        assert!(!result.success());

        // Check individual results
        assert!(result.results.get(&1).unwrap().is_ok());
        assert!(result.results.get(&2).unwrap().is_err());
    }

    #[tokio::test]
    async fn skip_on_dependency_failure() {
        let (mut scheduler, runner) = Scheduler::new(2);
        tokio::spawn(runner.run());

        // Action 2 depends on action 1 which fails
        let action1 = test_action(1, vec!["false"]); // Fails
        let action2 = test_action(2, vec!["echo", "should not run"]);

        scheduler.add(ScheduledAction::new(action1));
        scheduler.add(ScheduledAction::new(action2).with_dep(1));

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.executed, 1);
        assert_eq!(result.failed, 1);
        assert_eq!(result.skipped, 1);
    }

    #[tokio::test]
    async fn diamond_dependency() {
        let (mut scheduler, runner) = Scheduler::new(4);
        tokio::spawn(runner.run());

        //     1
        //    / \
        //   2   3
        //    \ /
        //     4
        scheduler.add(ScheduledAction::new(test_action(1, vec!["echo", "1"])));
        scheduler.add(ScheduledAction::new(test_action(2, vec!["echo", "2"])).with_dep(1));
        scheduler.add(ScheduledAction::new(test_action(3, vec!["echo", "3"])).with_dep(1));
        scheduler.add(
            ScheduledAction::new(test_action(4, vec!["echo", "4"]))
                .with_dep(2)
                .with_dep(3),
        );

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.executed, 4);
        assert_eq!(result.succeeded, 4);
        assert!(result.success());
    }

    #[tokio::test]
    async fn empty_schedule() {
        let (scheduler, runner) = Scheduler::new(2);
        tokio::spawn(runner.run());

        let result = scheduler.execute().await.unwrap();

        assert_eq!(result.total(), 0);
        assert!(result.success());
    }
}
