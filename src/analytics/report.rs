//! HTML report generation for build analytics.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use super::stats::BuildStats;

/// Configuration for report generation.
#[derive(Debug, Clone)]
pub struct ReportConfig {
    /// Title for the report
    pub title: String,
    /// Number of slowest actions to show
    pub top_slow_count: usize,
    /// Include critical path analysis
    pub include_critical_path: bool,
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            title: "Build Analytics Report".to_string(),
            top_slow_count: 10,
            include_critical_path: true,
        }
    }
}

impl ReportConfig {
    /// Create a new report config with custom title.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }
}

/// HTML report generator.
pub struct HtmlReport {
    config: ReportConfig,
}

impl HtmlReport {
    /// Create a new HTML report generator.
    pub fn new(config: ReportConfig) -> Self {
        Self { config }
    }

    /// Generate HTML report from build stats.
    pub fn generate(&self, stats: &BuildStats) -> String {
        let mut html = String::new();

        // HTML header
        html.push_str(&format!(r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{}</title>
    <style>
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; margin: 40px; background: #f5f5f5; }}
        .container {{ max-width: 1200px; margin: 0 auto; }}
        h1 {{ color: #333; border-bottom: 2px solid #4a9eff; padding-bottom: 10px; }}
        h2 {{ color: #555; margin-top: 30px; }}
        .card {{ background: white; border-radius: 8px; padding: 20px; margin: 15px 0; box-shadow: 0 2px 4px rgba(0,0,0,0.1); }}
        .stats-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 15px; }}
        .stat {{ text-align: center; padding: 15px; }}
        .stat-value {{ font-size: 2em; font-weight: bold; color: #4a9eff; }}
        .stat-label {{ color: #666; margin-top: 5px; }}
        table {{ width: 100%; border-collapse: collapse; }}
        th, td {{ padding: 12px; text-align: left; border-bottom: 1px solid #ddd; }}
        th {{ background: #f8f9fa; font-weight: 600; }}
        tr:hover {{ background: #f8f9fa; }}
        .duration {{ font-family: monospace; }}
        .cache-hit {{ color: #28a745; }}
        .cache-miss {{ color: #dc3545; }}
        .progress-bar {{ width: 100%; height: 20px; background: #e9ecef; border-radius: 4px; overflow: hidden; }}
        .progress-fill {{ height: 100%; background: #4a9eff; transition: width 0.3s; }}
        .critical-path {{ background: #fff3cd; border-left: 4px solid #ffc107; }}
    </style>
</head>
<body>
    <div class="container">
        <h1>{}</h1>
"#, self.config.title, self.config.title));

        // Summary section
        html.push_str(r#"
        <div class="card">
            <h2>Summary</h2>
            <div class="stats-grid">
"#);

        html.push_str(&format!(r#"
                <div class="stat">
                    <div class="stat-value">{}</div>
                    <div class="stat-label">Total Actions</div>
                </div>
"#, stats.actions.len()));

        html.push_str(&format!(r#"
                <div class="stat">
                    <div class="stat-value">{}</div>
                    <div class="stat-label">Total Duration</div>
                </div>
"#, format_duration(stats.total_duration)));

        html.push_str(&format!(r#"
                <div class="stat">
                    <div class="stat-value">{:.1}%</div>
                    <div class="stat-label">Cache Hit Rate</div>
                </div>
"#, stats.cache.hit_rate() * 100.0));

        html.push_str(&format!(r#"
                <div class="stat">
                    <div class="stat-value">{:.1}%</div>
                    <div class="stat-label">Parallelism Efficiency</div>
                </div>
"#, stats.parallelism_efficiency() * 100.0));

        html.push_str(r#"
            </div>
        </div>
"#);

        // Cache statistics
        html.push_str(r#"
        <div class="card">
            <h2>Cache Statistics</h2>
            <div class="stats-grid">
"#);
        html.push_str(&format!(r#"
                <div class="stat">
                    <div class="stat-value cache-hit">{}</div>
                    <div class="stat-label">Cache Hits</div>
                </div>
                <div class="stat">
                    <div class="stat-value cache-miss">{}</div>
                    <div class="stat-label">Cache Misses</div>
                </div>
"#, stats.cache.hits, stats.cache.misses));

        // Hit rate progress bar
        let hit_rate_pct = stats.cache.hit_rate() * 100.0;
        html.push_str(&format!(r#"
            </div>
            <div style="margin-top: 15px;">
                <div class="progress-bar">
                    <div class="progress-fill" style="width: {:.1}%;"></div>
                </div>
                <div style="text-align: center; margin-top: 5px; color: #666;">
                    {:.1}% hit rate
                </div>
            </div>
        </div>
"#, hit_rate_pct, hit_rate_pct));

        // Critical path section
        if self.config.include_critical_path {
            if let Some(ref path) = stats.critical_path {
                html.push_str(r#"
        <div class="card critical-path">
            <h2>Critical Path</h2>
            <p>The longest dependency chain determining minimum build time.</p>
"#);
                html.push_str(&format!(r#"
            <p><strong>Total Duration:</strong> <span class="duration">{}</span></p>
            <p><strong>Actions:</strong> {}</p>
            <table>
                <tr><th>#</th><th>Action</th><th>Duration</th></tr>
"#, format_duration(path.total_duration), path.actions.len()));

                for (i, action_id) in path.actions.iter().enumerate() {
                    if let Some(action) = stats.actions.get(action_id) {
                        html.push_str(&format!(r#"
                <tr>
                    <td>{}</td>
                    <td>{}</td>
                    <td class="duration">{}</td>
                </tr>
"#, i + 1, html_escape(&action.name), format_duration(action.duration)));
                    }
                }

                html.push_str(r#"
            </table>
        </div>
"#);
            }
        }

        // Slowest actions
        html.push_str(&format!(r#"
        <div class="card">
            <h2>Slowest Actions (Top {})</h2>
            <table>
                <tr>
                    <th>Action</th>
                    <th>Duration</th>
                    <th>Cache</th>
                </tr>
"#, self.config.top_slow_count));

        for action in stats.slowest_actions(self.config.top_slow_count) {
            let cache_status = if action.cache_hit {
                r#"<span class="cache-hit">HIT</span>"#
            } else {
                r#"<span class="cache-miss">MISS</span>"#
            };
            html.push_str(&format!(r#"
                <tr>
                    <td>{}</td>
                    <td class="duration">{}</td>
                    <td>{}</td>
                </tr>
"#, html_escape(&action.name), format_duration(action.duration), cache_status));
        }

        html.push_str(r#"
            </table>
        </div>
"#);

        // Footer
        html.push_str(r#"
    </div>
</body>
</html>
"#);

        html
    }

    /// Write report to a file.
    pub fn write_to_file(&self, stats: &BuildStats, path: &Path) -> std::io::Result<()> {
        let html = self.generate(stats);
        let mut file = std::fs::File::create(path)?;
        file.write_all(html.as_bytes())
    }
}

/// Format duration as human-readable string.
fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs_f64();
    if total_secs < 1.0 {
        format!("{:.0}ms", d.as_millis())
    } else if total_secs < 60.0 {
        format!("{:.2}s", total_secs)
    } else {
        let mins = (total_secs / 60.0).floor() as u64;
        let secs = total_secs % 60.0;
        format!("{}m {:.1}s", mins, secs)
    }
}

/// Escape HTML special characters.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analytics::stats::ActionStats;

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration(Duration::from_millis(50)), "50ms");
        assert_eq!(format_duration(Duration::from_millis(999)), "999ms");
    }

    #[test]
    fn test_format_duration_secs() {
        assert_eq!(format_duration(Duration::from_secs(5)), "5.00s");
        assert_eq!(format_duration(Duration::from_millis(1500)), "1.50s");
    }

    #[test]
    fn test_format_duration_mins() {
        assert_eq!(format_duration(Duration::from_secs(90)), "1m 30.0s");
        assert_eq!(format_duration(Duration::from_secs(185)), "3m 5.0s");
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("<script>"), "&lt;script&gt;");
        assert_eq!(html_escape("a & b"), "a &amp; b");
        assert_eq!(html_escape(r#""quoted""#), "&quot;quoted&quot;");
    }

    #[test]
    fn test_report_generation() {
        let mut stats = BuildStats::new(4);
        stats.total_duration = Duration::from_secs(10);
        stats.record_action(
            ActionStats::new(0, "compile main.rs")
                .with_duration(Duration::from_secs(5))
                .with_cache_hit(false)
        );
        stats.record_action(
            ActionStats::new(1, "compile lib.rs")
                .with_duration(Duration::from_secs(3))
                .with_cache_hit(true)
        );
        stats.compute_critical_path();

        let report = HtmlReport::new(ReportConfig::default());
        let html = report.generate(&stats);

        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Build Analytics Report"));
        assert!(html.contains("compile main.rs"));
        assert!(html.contains("compile lib.rs"));
        assert!(html.contains("Cache Hit Rate"));
        assert!(html.contains("Critical Path"));
    }

    #[test]
    fn test_report_config() {
        let config = ReportConfig::default()
            .with_title("My Build");
        assert_eq!(config.title, "My Build");
        assert_eq!(config.top_slow_count, 10);
    }

    #[test]
    fn test_empty_report() {
        let stats = BuildStats::new(1);
        let report = HtmlReport::new(ReportConfig::default());
        let html = report.generate(&stats);

        assert!(html.contains("0"));  // Total actions
        assert!(html.contains("0ms")); // Duration
    }
}
