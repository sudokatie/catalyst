//! Build analytics for timing, cache stats, and critical path analysis.

mod report;
mod stats;

pub use report::{HtmlReport, ReportConfig};
pub use stats::{ActionStats, BuildStats, CacheStats, CriticalPath};
