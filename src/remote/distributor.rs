//! Automatic work distribution across remote workers

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::{Action, Error};

use super::protocol::WorkerStatus;

/// Strategy for distributing work across workers
#[derive(Debug, Clone, Copy, Default)]
pub enum DistributionStrategy {
    /// Round-robin assignment
    #[default]
    RoundRobin,
    /// Least loaded worker first
    LeastLoaded,
    /// Random assignment
    Random,
}

/// Worker health and load information
#[derive(Debug, Clone)]
pub struct WorkerState {
    /// Worker address
    pub address: String,
    /// Last known status
    pub status: Option<WorkerStatus>,
    /// Pending jobs (assigned but not completed)
    pub pending_jobs: usize,
    /// Consecutive failures
    pub consecutive_failures: usize,
    /// Is worker healthy
    pub healthy: bool,
}

impl WorkerState {
    /// Create a new worker state
    pub fn new(address: String) -> Self {
        Self {
            address,
            status: None,
            pending_jobs: 0,
            consecutive_failures: 0,
            healthy: true,
        }
    }

    /// Effective load (pending + active)
    pub fn load(&self) -> usize {
        self.pending_jobs + self.status.as_ref().map(|s| s.active_jobs).unwrap_or(0)
    }

    /// Available capacity
    pub fn capacity(&self) -> usize {
        let max = self.status.as_ref().map(|s| s.max_jobs).unwrap_or(4);
        max.saturating_sub(self.load())
    }

    /// Mark a job as assigned
    pub fn job_assigned(&mut self) {
        self.pending_jobs += 1;
    }

    /// Mark a job as completed
    pub fn job_completed(&mut self, success: bool) {
        self.pending_jobs = self.pending_jobs.saturating_sub(1);
        if success {
            self.consecutive_failures = 0;
        } else {
            self.consecutive_failures += 1;
            if self.consecutive_failures >= 3 {
                self.healthy = false;
            }
        }
    }

    /// Update from worker status
    pub fn update_status(&mut self, status: WorkerStatus) {
        self.healthy = status.healthy;
        self.status = Some(status);
    }
}

/// Work distributor that assigns actions to workers
pub struct Distributor {
    /// Worker states
    workers: RwLock<Vec<WorkerState>>,
    /// Distribution strategy
    strategy: DistributionStrategy,
    /// Round-robin counter
    rr_counter: AtomicUsize,
}

impl Distributor {
    /// Create a new distributor
    pub fn new(addresses: Vec<String>, strategy: DistributionStrategy) -> Self {
        let workers = addresses.into_iter()
            .map(WorkerState::new)
            .collect();
        
        Self {
            workers: RwLock::new(workers),
            strategy,
            rr_counter: AtomicUsize::new(0),
        }
    }

    /// Select a worker for an action
    pub async fn select_worker(&self, _action: &Action) -> Option<String> {
        let workers = self.workers.read().await;
        
        if workers.is_empty() {
            return None;
        }

        // Filter healthy workers with capacity
        let available: Vec<_> = workers.iter()
            .enumerate()
            .filter(|(_, w)| w.healthy && w.capacity() > 0)
            .collect();

        if available.is_empty() {
            return None;
        }

        let selected = match self.strategy {
            DistributionStrategy::RoundRobin => {
                let idx = self.rr_counter.fetch_add(1, Ordering::Relaxed) % available.len();
                available[idx].1
            }
            DistributionStrategy::LeastLoaded => {
                available.iter()
                    .min_by_key(|(_, w)| w.load())
                    .map(|(_, w)| *w)?
            }
            DistributionStrategy::Random => {
                use std::time::{SystemTime, UNIX_EPOCH};
                let seed = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .subsec_nanos() as usize;
                let idx = seed % available.len();
                available[idx].1
            }
        };

        Some(selected.address.clone())
    }

    /// Mark a job as assigned to a worker
    pub async fn job_assigned(&self, address: &str) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.iter_mut().find(|w| w.address == address) {
            worker.job_assigned();
        }
    }

    /// Mark a job as completed on a worker
    pub async fn job_completed(&self, address: &str, success: bool) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.iter_mut().find(|w| w.address == address) {
            worker.job_completed(success);
        }
    }

    /// Update worker status
    pub async fn update_status(&self, address: &str, status: WorkerStatus) {
        let mut workers = self.workers.write().await;
        if let Some(worker) = workers.iter_mut().find(|w| w.address == address) {
            worker.update_status(status);
        }
    }

    /// Get all healthy workers
    pub async fn healthy_workers(&self) -> Vec<String> {
        let workers = self.workers.read().await;
        workers.iter()
            .filter(|w| w.healthy)
            .map(|w| w.address.clone())
            .collect()
    }

    /// Get worker count
    pub async fn worker_count(&self) -> usize {
        self.workers.read().await.len()
    }

    /// Get healthy worker count
    pub async fn healthy_count(&self) -> usize {
        self.workers.read().await.iter().filter(|w| w.healthy).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_action() -> Action {
        Action::new(vec!["echo".to_string(), "test".to_string()])
    }

    #[tokio::test]
    async fn empty_distributor_returns_none() {
        let dist = Distributor::new(vec![], DistributionStrategy::RoundRobin);
        let action = test_action();
        assert!(dist.select_worker(&action).await.is_none());
    }

    #[tokio::test]
    async fn single_worker_selected() {
        let dist = Distributor::new(
            vec!["worker1:9000".to_string()],
            DistributionStrategy::RoundRobin
        );
        let action = test_action();
        let selected = dist.select_worker(&action).await;
        assert_eq!(selected, Some("worker1:9000".to_string()));
    }

    #[tokio::test]
    async fn round_robin_cycles() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
                "worker3:9000".to_string(),
            ],
            DistributionStrategy::RoundRobin
        );
        let action = test_action();
        
        let w1 = dist.select_worker(&action).await.unwrap();
        let w2 = dist.select_worker(&action).await.unwrap();
        let w3 = dist.select_worker(&action).await.unwrap();
        let w4 = dist.select_worker(&action).await.unwrap();
        
        // Should cycle through workers
        assert_eq!(w1, "worker1:9000");
        assert_eq!(w2, "worker2:9000");
        assert_eq!(w3, "worker3:9000");
        assert_eq!(w4, "worker1:9000");
    }

    #[tokio::test]
    async fn least_loaded_selects_lightest() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
            ],
            DistributionStrategy::LeastLoaded
        );
        
        // Assign jobs to worker1
        dist.job_assigned("worker1:9000").await;
        dist.job_assigned("worker1:9000").await;
        
        let action = test_action();
        let selected = dist.select_worker(&action).await;
        
        // Should select worker2 (less loaded)
        assert_eq!(selected, Some("worker2:9000".to_string()));
    }

    #[tokio::test]
    async fn unhealthy_worker_skipped() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
            ],
            DistributionStrategy::RoundRobin
        );
        
        // Mark worker1 as unhealthy (3 consecutive failures)
        dist.job_completed("worker1:9000", false).await;
        dist.job_completed("worker1:9000", false).await;
        dist.job_completed("worker1:9000", false).await;
        
        let action = test_action();
        let selected = dist.select_worker(&action).await;
        
        // Should only select worker2
        assert_eq!(selected, Some("worker2:9000".to_string()));
    }

    #[tokio::test]
    async fn worker_count() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
            ],
            DistributionStrategy::RoundRobin
        );
        
        assert_eq!(dist.worker_count().await, 2);
    }

    #[tokio::test]
    async fn healthy_count() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
            ],
            DistributionStrategy::RoundRobin
        );
        
        assert_eq!(dist.healthy_count().await, 2);
        
        // Mark worker1 unhealthy
        dist.job_completed("worker1:9000", false).await;
        dist.job_completed("worker1:9000", false).await;
        dist.job_completed("worker1:9000", false).await;
        
        assert_eq!(dist.healthy_count().await, 1);
    }

    #[tokio::test]
    async fn healthy_workers_list() {
        let dist = Distributor::new(
            vec![
                "worker1:9000".to_string(),
                "worker2:9000".to_string(),
            ],
            DistributionStrategy::RoundRobin
        );
        
        let healthy = dist.healthy_workers().await;
        assert_eq!(healthy.len(), 2);
    }

    #[tokio::test]
    async fn job_lifecycle() {
        let dist = Distributor::new(
            vec!["worker1:9000".to_string()],
            DistributionStrategy::RoundRobin
        );
        
        // Assign job
        dist.job_assigned("worker1:9000").await;
        
        // Complete job successfully
        dist.job_completed("worker1:9000", true).await;
        
        // Worker should still be healthy
        assert_eq!(dist.healthy_count().await, 1);
    }

    #[tokio::test]
    async fn status_update() {
        let dist = Distributor::new(
            vec!["worker1:9000".to_string()],
            DistributionStrategy::RoundRobin
        );
        
        let status = WorkerStatus {
            worker_id: "worker1".to_string(),
            active_jobs: 2,
            max_jobs: 4,
            healthy: true,
        };
        
        dist.update_status("worker1:9000", status).await;
        
        // Worker should reflect the status
        assert_eq!(dist.healthy_count().await, 1);
    }
}
