//! Action result cache for build caching

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

use super::hash::{hash_to_hex, hex_to_hash};
use crate::{ActionKey, ActionResult, Error};

/// Cache for action execution results.
///
/// Stores action results keyed by their action hash, enabling
/// incremental builds by reusing previous results.
pub struct ActionCache {
    root: PathBuf,
}

impl ActionCache {
    /// Create a new action cache at the given root directory
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Initialize the cache directory structure
    pub fn init(&self) -> Result<(), Error> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Get the storage path for an action key
    fn get_path(&self, key: &ActionKey) -> PathBuf {
        let hex = hash_to_hex(key);
        // Use first 2 chars as subdirectory
        self.root.join(&hex[..2]).join(format!("{}.json", &hex[2..]))
    }

    /// Retrieve a cached action result
    pub fn get(&self, key: &ActionKey) -> Option<ActionResult> {
        let path = self.get_path(key);
        if !path.exists() {
            return None;
        }

        let mut file = File::open(&path).ok()?;
        let mut contents = String::new();
        file.read_to_string(&mut contents).ok()?;

        let cached: CachedResult = serde_json::from_str(&contents).ok()?;
        Some(cached.into())
    }

    /// Store an action result
    pub fn put(&self, key: &ActionKey, result: &ActionResult) -> Result<(), Error> {
        let path = self.get_path(key);

        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let cached = CachedResult::from(result);
        let json = serde_json::to_string_pretty(&cached)?;

        // Write atomically
        let temp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(json.as_bytes())?;
            file.sync_all()?;
        }
        fs::rename(&temp_path, &path)?;

        Ok(())
    }

    /// Check if a result is cached
    pub fn contains(&self, key: &ActionKey) -> bool {
        self.get_path(key).exists()
    }

    /// Remove a cached result
    pub fn remove(&self, key: &ActionKey) -> Result<bool, Error> {
        let path = self.get_path(key);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

/// Serializable form of ActionResult
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedResult {
    exit_code: i32,
    output_hashes: Vec<(String, String)>, // path -> hash as hex
    stdout: String,
    stderr: String,
    duration_ms: u64,
}

impl From<&ActionResult> for CachedResult {
    fn from(r: &ActionResult) -> Self {
        Self {
            exit_code: r.exit_code,
            output_hashes: r
                .output_hashes
                .iter()
                .map(|(p, h)| (p.to_string_lossy().to_string(), hash_to_hex(h)))
                .collect(),
            stdout: r.stdout.clone(),
            stderr: r.stderr.clone(),
            duration_ms: r.duration.as_millis() as u64,
        }
    }
}

impl From<CachedResult> for ActionResult {
    fn from(c: CachedResult) -> Self {
        let mut result = ActionResult::success(std::time::Duration::from_millis(c.duration_ms));
        result.exit_code = c.exit_code;
        result.stdout = c.stdout;
        result.stderr = c.stderr;
        result.output_hashes = c
            .output_hashes
            .into_iter()
            .filter_map(|(p, h)| {
                hex_to_hash(&h).map(|hash| (PathBuf::from(p), hash))
            })
            .collect();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Duration;
    use tempfile::tempdir;

    fn make_test_result() -> ActionResult {
        let mut result = ActionResult::success(Duration::from_millis(150));
        result.stdout = "build output".to_string();
        result.stderr = "warning: unused".to_string();
        result.output_hashes.insert(
            PathBuf::from("out.o"),
            [0u8; 32],
        );
        result
    }

    #[test]
    fn store_and_retrieve() {
        let dir = tempdir().unwrap();
        let cache = ActionCache::new(dir.path().to_path_buf());
        cache.init().unwrap();

        let key: ActionKey = [1u8; 32];
        let result = make_test_result();

        cache.put(&key, &result).unwrap();
        let retrieved = cache.get(&key).unwrap();

        assert_eq!(retrieved.exit_code, result.exit_code);
        assert_eq!(retrieved.stdout, result.stdout);
        assert_eq!(retrieved.stderr, result.stderr);
    }

    #[test]
    fn miss_returns_none() {
        let dir = tempdir().unwrap();
        let cache = ActionCache::new(dir.path().to_path_buf());
        cache.init().unwrap();

        let missing: ActionKey = [99u8; 32];
        assert!(cache.get(&missing).is_none());
    }

    #[test]
    fn result_includes_exit_code() {
        let dir = tempdir().unwrap();
        let cache = ActionCache::new(dir.path().to_path_buf());
        cache.init().unwrap();

        let key: ActionKey = [2u8; 32];
        let result = ActionResult::failure(
            1,
            "compile error".to_string(),
            Duration::from_secs(1),
        );

        cache.put(&key, &result).unwrap();
        let retrieved = cache.get(&key).unwrap();

        assert_eq!(retrieved.exit_code, 1);
        assert_eq!(retrieved.stderr, "compile error");
        assert!(!retrieved.is_success());
    }

    #[test]
    fn result_includes_output_hashes() {
        let dir = tempdir().unwrap();
        let cache = ActionCache::new(dir.path().to_path_buf());
        cache.init().unwrap();

        let key: ActionKey = [3u8; 32];
        let mut result = ActionResult::success(Duration::from_millis(50));
        result.output_hashes.insert(PathBuf::from("a.o"), [10u8; 32]);
        result.output_hashes.insert(PathBuf::from("b.o"), [20u8; 32]);

        cache.put(&key, &result).unwrap();
        let retrieved = cache.get(&key).unwrap();

        assert_eq!(retrieved.output_hashes.len(), 2);
        assert_eq!(retrieved.output_hashes.get(&PathBuf::from("a.o")), Some(&[10u8; 32]));
    }

    #[test]
    fn contains_check() {
        let dir = tempdir().unwrap();
        let cache = ActionCache::new(dir.path().to_path_buf());
        cache.init().unwrap();

        let key: ActionKey = [4u8; 32];
        assert!(!cache.contains(&key));

        cache.put(&key, &make_test_result()).unwrap();
        assert!(cache.contains(&key));
    }
}
