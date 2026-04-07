//! Build statistics collection and analysis.

use std::collections::HashMap;
use std::time::Duration;

use crate::ActionId;

/// Statistics for a single action execution.
#[derive(Debug, Clone)]
pub struct ActionStats {
    /// Action identifier
    pub action_id: ActionId,
    /// Action name/description
    pub name: String,
    /// Time spent executing (wall clock)
    pub duration: Duration,
    /// Whether the action hit cache
    pub cache_hit: bool,
    /// Dependencies of this action
    pub dependencies: Vec<ActionId>,
    /// Outputs produced
    pub outputs: Vec<String>,
}

impl ActionStats {
    /// Create new action stats.
    pub fn new(action_id: ActionId, name: impl Into<String>) -> Self {
        Self {
            action_id,
            name: name.into(),
            duration: Duration::ZERO,
            cache_hit: false,
            dependencies: Vec::new(),
            outputs: Vec::new(),
        }
    }

    /// Set the duration.
    pub fn with_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    /// Mark as cache hit.
    pub fn with_cache_hit(mut self, hit: bool) -> Self {
        self.cache_hit = hit;
        self
    }

    /// Set dependencies.
    pub fn with_deps(mut self, deps: Vec<ActionId>) -> Self {
        self.dependencies = deps;
        self
    }

    /// Set outputs.
    pub fn with_outputs(mut self, outputs: Vec<String>) -> Self {
        self.outputs = outputs;
        self
    }
}

/// Cache statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Number of cache hits
    pub hits: usize,
    /// Number of cache misses
    pub misses: usize,
    /// Total bytes read from cache
    pub bytes_read: u64,
    /// Total bytes written to cache
    pub bytes_written: u64,
}

impl CacheStats {
    /// Calculate hit rate (0.0 to 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }

    /// Record a cache hit.
    pub fn record_hit(&mut self, bytes: u64) {
        self.hits += 1;
        self.bytes_read += bytes;
    }

    /// Record a cache miss.
    pub fn record_miss(&mut self, bytes_written: u64) {
        self.misses += 1;
        self.bytes_written += bytes_written;
    }
}

/// A sequence of actions forming a critical path.
#[derive(Debug, Clone)]
pub struct CriticalPath {
    /// Actions in the critical path (in execution order)
    pub actions: Vec<ActionId>,
    /// Total duration of the critical path
    pub total_duration: Duration,
}

impl CriticalPath {
    /// Create an empty critical path.
    pub fn empty() -> Self {
        Self {
            actions: Vec::new(),
            total_duration: Duration::ZERO,
        }
    }
}

/// Aggregate build statistics.
#[derive(Debug, Clone)]
pub struct BuildStats {
    /// Per-action statistics
    pub actions: HashMap<ActionId, ActionStats>,
    /// Cache statistics
    pub cache: CacheStats,
    /// Total build duration (wall clock)
    pub total_duration: Duration,
    /// Total execution time (sum of all actions)
    pub total_execution_time: Duration,
    /// Number of workers used
    pub worker_count: usize,
    /// Critical path
    pub critical_path: Option<CriticalPath>,
}

impl BuildStats {
    /// Create new build stats.
    pub fn new(worker_count: usize) -> Self {
        Self {
            actions: HashMap::new(),
            cache: CacheStats::default(),
            total_duration: Duration::ZERO,
            total_execution_time: Duration::ZERO,
            worker_count,
            critical_path: None,
        }
    }

    /// Record an action's execution.
    pub fn record_action(&mut self, stats: ActionStats) {
        self.total_execution_time += stats.duration;
        if stats.cache_hit {
            self.cache.hits += 1;
        } else {
            self.cache.misses += 1;
        }
        self.actions.insert(stats.action_id, stats);
    }

    /// Set total build duration.
    pub fn set_duration(&mut self, duration: Duration) {
        self.total_duration = duration;
    }

    /// Calculate parallelism efficiency (0.0 to 1.0).
    /// 1.0 means perfect parallelism, lower means sequential bottlenecks.
    pub fn parallelism_efficiency(&self) -> f64 {
        if self.total_duration.as_nanos() == 0 || self.worker_count == 0 {
            return 0.0;
        }
        let ideal_duration = self.total_execution_time.as_nanos() as f64 / self.worker_count as f64;
        let actual_duration = self.total_duration.as_nanos() as f64;
        if actual_duration == 0.0 {
            0.0
        } else {
            (ideal_duration / actual_duration).min(1.0)
        }
    }

    /// Compute the critical path through the build graph.
    pub fn compute_critical_path(&mut self) {
        // Find the action with the longest total path duration
        let mut longest_paths: HashMap<ActionId, (Duration, Vec<ActionId>)> = HashMap::new();

        // Topological sort by processing actions with no unprocessed dependencies first
        let mut processed: std::collections::HashSet<ActionId> = std::collections::HashSet::new();
        let mut to_process: Vec<ActionId> = self.actions.keys().copied().collect();

        while !to_process.is_empty() {
            let mut made_progress = false;
            to_process.retain(|&id| {
                let stats = match self.actions.get(&id) {
                    Some(s) => s,
                    None => return false,
                };

                // Check if all dependencies are processed
                let all_deps_processed = stats.dependencies.iter().all(|d| processed.contains(d));
                if !all_deps_processed {
                    return true; // Keep in queue
                }

                // Find the longest path to this node
                let mut max_path_duration = Duration::ZERO;
                let mut best_path = Vec::new();

                for dep_id in &stats.dependencies {
                    if let Some((dur, path)) = longest_paths.get(dep_id) {
                        if *dur > max_path_duration {
                            max_path_duration = *dur;
                            best_path = path.clone();
                        }
                    }
                }

                // Add this node to the path
                let total_duration = max_path_duration + stats.duration;
                best_path.push(id);
                longest_paths.insert(id, (total_duration, best_path));
                processed.insert(id);
                made_progress = true;
                false // Remove from queue
            });

            if !made_progress && !to_process.is_empty() {
                // Cycle detected or missing dependencies
                break;
            }
        }

        // Find the longest path overall
        let critical = longest_paths
            .into_iter()
            .max_by_key(|(_, (dur, _))| *dur)
            .map(|(_, (dur, path))| CriticalPath {
                actions: path,
                total_duration: dur,
            })
            .unwrap_or_else(CriticalPath::empty);

        self.critical_path = Some(critical);
    }

    /// Get actions sorted by duration (slowest first).
    pub fn slowest_actions(&self, limit: usize) -> Vec<&ActionStats> {
        let mut actions: Vec<_> = self.actions.values().collect();
        actions.sort_by(|a, b| b.duration.cmp(&a.duration));
        actions.truncate(limit);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_stats_hit_rate() {
        let mut cache = CacheStats::default();
        assert_eq!(cache.hit_rate(), 0.0);

        cache.record_hit(100);
        cache.record_hit(100);
        cache.record_miss(50);
        assert!((cache.hit_rate() - 0.666).abs() < 0.01);
    }

    #[test]
    fn test_cache_stats_record() {
        let mut cache = CacheStats::default();
        cache.record_hit(100);
        cache.record_miss(200);

        assert_eq!(cache.hits, 1);
        assert_eq!(cache.misses, 1);
        assert_eq!(cache.bytes_read, 100);
        assert_eq!(cache.bytes_written, 200);
    }

    #[test]
    fn test_action_stats_builder() {
        let stats = ActionStats::new(1, "test action")
            .with_duration(Duration::from_secs(5))
            .with_cache_hit(true)
            .with_deps(vec![0])
            .with_outputs(vec!["out.txt".to_string()]);

        assert_eq!(stats.action_id, 1);
        assert_eq!(stats.name, "test action");
        assert_eq!(stats.duration, Duration::from_secs(5));
        assert!(stats.cache_hit);
        assert_eq!(stats.dependencies, vec![0]);
        assert_eq!(stats.outputs, vec!["out.txt"]);
    }

    #[test]
    fn test_build_stats_record() {
        let mut stats = BuildStats::new(4);
        stats.record_action(
            ActionStats::new(0, "a0").with_duration(Duration::from_secs(1))
        );
        stats.record_action(
            ActionStats::new(1, "a1")
                .with_duration(Duration::from_secs(2))
                .with_cache_hit(true)
        );

        assert_eq!(stats.actions.len(), 2);
        assert_eq!(stats.total_execution_time, Duration::from_secs(3));
        assert_eq!(stats.cache.hits, 1);
        assert_eq!(stats.cache.misses, 1);
    }

    #[test]
    fn test_build_stats_slowest() {
        let mut stats = BuildStats::new(2);
        stats.record_action(ActionStats::new(0, "fast").with_duration(Duration::from_millis(100)));
        stats.record_action(ActionStats::new(1, "slow").with_duration(Duration::from_secs(5)));
        stats.record_action(ActionStats::new(2, "medium").with_duration(Duration::from_secs(1)));

        let slowest = stats.slowest_actions(2);
        assert_eq!(slowest.len(), 2);
        assert_eq!(slowest[0].name, "slow");
        assert_eq!(slowest[1].name, "medium");
    }

    #[test]
    fn test_critical_path_linear() {
        let mut stats = BuildStats::new(1);
        stats.record_action(
            ActionStats::new(0, "a0")
                .with_duration(Duration::from_secs(1))
                .with_deps(vec![])
        );
        stats.record_action(
            ActionStats::new(1, "a1")
                .with_duration(Duration::from_secs(2))
                .with_deps(vec![0])
        );
        stats.record_action(
            ActionStats::new(2, "a2")
                .with_duration(Duration::from_secs(3))
                .with_deps(vec![1])
        );

        stats.compute_critical_path();
        let path = stats.critical_path.unwrap();

        assert_eq!(path.actions, vec![0, 1, 2]);
        assert_eq!(path.total_duration, Duration::from_secs(6));
    }

    #[test]
    fn test_critical_path_parallel() {
        let mut stats = BuildStats::new(2);
        // Two parallel paths: 0->1 (total 3s) and 2->3 (total 7s)
        stats.record_action(
            ActionStats::new(0, "a0")
                .with_duration(Duration::from_secs(1))
                .with_deps(vec![])
        );
        stats.record_action(
            ActionStats::new(1, "a1")
                .with_duration(Duration::from_secs(2))
                .with_deps(vec![0])
        );
        stats.record_action(
            ActionStats::new(2, "a2")
                .with_duration(Duration::from_secs(3))
                .with_deps(vec![])
        );
        stats.record_action(
            ActionStats::new(3, "a3")
                .with_duration(Duration::from_secs(4))
                .with_deps(vec![2])
        );

        stats.compute_critical_path();
        let path = stats.critical_path.unwrap();

        assert_eq!(path.actions, vec![2, 3]);
        assert_eq!(path.total_duration, Duration::from_secs(7));
    }

    #[test]
    fn test_parallelism_efficiency() {
        let mut stats = BuildStats::new(4);
        stats.total_execution_time = Duration::from_secs(16); // 16s of work
        stats.total_duration = Duration::from_secs(8); // Done in 8s with 4 workers

        // Ideal would be 16/4 = 4s, actual is 8s, so efficiency = 4/8 = 0.5
        let eff = stats.parallelism_efficiency();
        assert!((eff - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_parallelism_efficiency_zero() {
        let stats = BuildStats::new(0);
        assert_eq!(stats.parallelism_efficiency(), 0.0);
    }
}
