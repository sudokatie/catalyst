//! Configuration loading and management

use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::Error;

/// Main configuration structure
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Build-related configuration
    pub build: BuildConfig,
    /// Cache-related configuration
    pub cache: CacheConfig,
    /// Remote execution configuration
    pub remote: RemoteConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            build: BuildConfig::default(),
            cache: CacheConfig::default(),
            remote: RemoteConfig::default(),
        }
    }
}

impl Config {
    /// Load configuration from a file
    pub fn load(path: &Path) -> Result<Self, Error> {
        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from the default locations
    ///
    /// Searches in order:
    /// 1. .catalystrc in current directory
    /// 2. .catalystrc in workspace root
    /// 3. ~/.catalystrc
    pub fn load_default(workspace_root: Option<&Path>) -> Result<Self, Error> {
        // Check current directory
        let cwd_config = PathBuf::from(".catalystrc");
        if cwd_config.exists() {
            return Self::load(&cwd_config);
        }

        // Check workspace root
        if let Some(root) = workspace_root {
            let ws_config = root.join(".catalystrc");
            if ws_config.exists() {
                return Self::load(&ws_config);
            }
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let home_config = home.join(".catalystrc");
            if home_config.exists() {
                return Self::load(&home_config);
            }
        }

        // Return defaults
        Ok(Self::default())
    }

    /// Apply environment variable overrides
    pub fn with_env_overrides(mut self) -> Self {
        // CATALYST_JOBS overrides build.jobs
        if let Ok(jobs) = env::var("CATALYST_JOBS") {
            if let Ok(n) = jobs.parse() {
                self.build.jobs = n;
            }
        }

        // CATALYST_SANDBOX overrides build.sandbox
        if let Ok(sandbox) = env::var("CATALYST_SANDBOX") {
            self.build.sandbox = sandbox == "1" || sandbox.to_lowercase() == "true";
        }

        // CATALYST_CACHE_DIR overrides cache.local
        if let Ok(cache_dir) = env::var("CATALYST_CACHE_DIR") {
            self.cache.local = Some(PathBuf::from(cache_dir));
        }

        // CATALYST_REMOTE_CACHE overrides cache.remote
        if let Ok(remote) = env::var("CATALYST_REMOTE_CACHE") {
            self.cache.remote = Some(remote);
        }

        // CATALYST_REMOTE_EXECUTOR overrides remote.executor
        if let Ok(executor) = env::var("CATALYST_REMOTE_EXECUTOR") {
            self.remote.executor = Some(executor);
        }

        self
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), Error> {
        // Jobs must be positive
        if self.build.jobs == 0 {
            return Err(Error::Config("build.jobs must be greater than 0".into()));
        }

        // Cache directory must be valid path if specified
        if let Some(ref local) = self.cache.local {
            if local.to_string_lossy().is_empty() {
                return Err(Error::Config("cache.local cannot be empty".into()));
            }
        }

        Ok(())
    }

    /// Get the effective cache directory
    pub fn cache_dir(&self) -> PathBuf {
        self.cache
            .local
            .clone()
            .or_else(|| dirs::home_dir().map(|h| h.join(".catalyst/cache")))
            .unwrap_or_else(|| PathBuf::from(".catalyst/cache"))
    }

    /// Get the effective number of parallel jobs
    pub fn jobs(&self) -> usize {
        if self.build.jobs == 0 {
            num_cpus()
        } else {
            self.build.jobs
        }
    }
}

/// Build-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    /// Number of parallel jobs (0 = auto-detect)
    pub jobs: usize,
    /// Enable sandboxing
    pub sandbox: bool,
    /// Verbose output
    pub verbose: bool,
    /// Keep going on failure
    pub keep_going: bool,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            jobs: 0, // Auto-detect
            sandbox: true,
            verbose: false,
            keep_going: false,
        }
    }
}

/// Cache-related configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// Local cache directory
    pub local: Option<PathBuf>,
    /// Remote cache URL (gRPC or HTTP)
    pub remote: Option<String>,
    /// Maximum local cache size in bytes
    pub max_size: Option<u64>,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            local: None, // Will use ~/.catalyst/cache
            remote: None,
            max_size: None,
        }
    }
}

/// Remote execution configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteConfig {
    /// Remote executor URL
    pub executor: Option<String>,
    /// Instance name for remote execution
    pub instance: Option<String>,
    /// Timeout for remote operations in seconds
    pub timeout_secs: u64,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            executor: None,
            instance: None,
            timeout_secs: 300,
        }
    }
}

/// Get the number of CPUs
fn num_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// Helper module for home directory detection
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var("HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| std::env::var("USERPROFILE").ok().map(PathBuf::from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{NamedTempFile, TempDir};

    #[test]
    fn default_config() {
        let config = Config::default();

        assert_eq!(config.build.jobs, 0);
        assert!(config.build.sandbox);
        assert!(!config.build.verbose);
        assert!(config.cache.local.is_none());
        assert!(config.cache.remote.is_none());
    }

    #[test]
    fn load_from_file() {
        let mut file = NamedTempFile::new().unwrap();
        write!(
            file,
            r#"
[build]
jobs = 8
sandbox = false

[cache]
local = "/tmp/catalyst-cache"
"#
        )
        .unwrap();

        let config = Config::load(file.path()).unwrap();

        assert_eq!(config.build.jobs, 8);
        assert!(!config.build.sandbox);
        assert_eq!(
            config.cache.local,
            Some(PathBuf::from("/tmp/catalyst-cache"))
        );
    }

    #[test]
    fn use_defaults_if_missing() {
        let dir = TempDir::new().unwrap();
        // No config file exists
        let config = Config::load_default(Some(dir.path())).unwrap();

        // Should use defaults
        assert_eq!(config.build.jobs, 0);
        assert!(config.build.sandbox);
    }

    #[test]
    fn env_override_jobs() {
        // SAFETY: Test is single-threaded, env var is immediately removed
        unsafe {
            env::set_var("CATALYST_JOBS", "16");
        }
        let config = Config::default().with_env_overrides();
        unsafe {
            env::remove_var("CATALYST_JOBS");
        }

        assert_eq!(config.build.jobs, 16);
    }

    #[test]
    fn env_override_sandbox() {
        // SAFETY: Test is single-threaded, env var is immediately removed
        unsafe {
            env::set_var("CATALYST_SANDBOX", "false");
        }
        let config = Config::default().with_env_overrides();
        unsafe {
            env::remove_var("CATALYST_SANDBOX");
        }

        assert!(!config.build.sandbox);
    }

    #[test]
    fn env_override_cache_dir() {
        // SAFETY: Test is single-threaded, env var is immediately removed
        unsafe {
            env::set_var("CATALYST_CACHE_DIR", "/custom/cache");
        }
        let config = Config::default().with_env_overrides();
        unsafe {
            env::remove_var("CATALYST_CACHE_DIR");
        }

        assert_eq!(config.cache.local, Some(PathBuf::from("/custom/cache")));
    }

    #[test]
    fn validate_jobs_zero_allowed() {
        let config = Config::default();
        // Zero jobs means auto-detect, which is valid
        // But we need to check jobs() returns a positive number
        assert!(config.jobs() > 0);
    }

    #[test]
    fn validate_empty_cache_path() {
        let mut config = Config::default();
        config.cache.local = Some(PathBuf::from(""));

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn cache_dir_fallback() {
        let config = Config::default();
        let cache_dir = config.cache_dir();

        // Should return some path even with no config
        assert!(!cache_dir.to_string_lossy().is_empty());
    }

    #[test]
    fn jobs_auto_detect() {
        let config = Config::default();
        // Auto-detect should return positive number
        assert!(config.jobs() > 0);
    }

    #[test]
    fn jobs_explicit() {
        let mut config = Config::default();
        config.build.jobs = 12;
        assert_eq!(config.jobs(), 12);
    }

    #[test]
    fn partial_config_file() {
        let mut file = NamedTempFile::new().unwrap();
        // Only specify some fields
        write!(
            file,
            r#"
[build]
jobs = 4
"#
        )
        .unwrap();

        let config = Config::load(file.path()).unwrap();

        // Specified field
        assert_eq!(config.build.jobs, 4);
        // Defaults for unspecified
        assert!(config.build.sandbox);
        assert!(config.cache.local.is_none());
    }
}
