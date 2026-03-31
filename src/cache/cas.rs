//! Content-addressable storage for build artifacts

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::hash::{hash_bytes, hash_file, hash_to_hex, Hash};
use crate::Error;

/// Content-addressable storage.
///
/// Stores data by its SHA-256 hash, allowing deduplication and
/// verification of content integrity.
pub struct CAS {
    root: PathBuf,
}

impl CAS {
    /// Create a new CAS at the given root directory
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Initialize the CAS directory structure
    pub fn init(&self) -> Result<(), Error> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Check if the CAS contains data with the given hash
    pub fn contains(&self, hash: &Hash) -> bool {
        self.get_path(hash).exists()
    }

    /// Get the storage path for a hash
    pub fn get_path(&self, hash: &Hash) -> PathBuf {
        let hex = hash_to_hex(hash);
        // Use first 2 chars as subdirectory to avoid too many files in one dir
        self.root.join(&hex[..2]).join(&hex[2..])
    }

    /// Retrieve data by its hash
    pub fn get(&self, hash: &Hash) -> Result<Vec<u8>, Error> {
        let path = self.get_path(hash);
        if !path.exists() {
            return Err(Error::Cache(format!(
                "content not found: {}",
                hash_to_hex(hash)
            )));
        }

        let mut file = File::open(&path)?;
        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        // Verify integrity
        let actual_hash = hash_bytes(&data);
        if actual_hash != *hash {
            return Err(Error::Cache(format!(
                "content corrupted: expected {}, got {}",
                hash_to_hex(hash),
                hash_to_hex(&actual_hash)
            )));
        }

        Ok(data)
    }

    /// Store data and return its hash
    pub fn put(&self, data: &[u8]) -> Result<Hash, Error> {
        let hash = hash_bytes(data);

        // Check if already stored
        if self.contains(&hash) {
            return Ok(hash);
        }

        let path = self.get_path(&hash);

        // Create parent directory
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Write atomically via temp file
        let temp_path = path.with_extension("tmp");
        {
            let mut file = File::create(&temp_path)?;
            file.write_all(data)?;
            file.sync_all()?;
        }
        fs::rename(&temp_path, &path)?;

        Ok(hash)
    }

    /// Store a file and return its hash
    pub fn put_file(&self, path: &Path) -> Result<Hash, Error> {
        let hash = hash_file(path)?;

        // Check if already stored
        if self.contains(&hash) {
            return Ok(hash);
        }

        let dest = self.get_path(&hash);

        // Create parent directory
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Copy file to CAS
        fs::copy(path, &dest)?;

        Ok(hash)
    }

    /// Create a hard link from CAS content to destination.
    /// Falls back to copy if hard link fails (e.g., cross-filesystem).
    pub fn link_to(&self, hash: &Hash, dest: &Path) -> Result<(), Error> {
        let src = self.get_path(hash);
        if !src.exists() {
            return Err(Error::Cache(format!(
                "content not found: {}",
                hash_to_hex(hash)
            )));
        }

        // Create parent directory for destination
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }

        // Remove existing file if present
        if dest.exists() {
            fs::remove_file(dest)?;
        }

        // Try hard link first, fall back to copy
        if fs::hard_link(&src, dest).is_err() {
            fs::copy(&src, dest)?;
        }

        Ok(())
    }

    /// Remove content by hash
    pub fn remove(&self, hash: &Hash) -> Result<bool, Error> {
        let path = self.get_path(hash);
        if path.exists() {
            fs::remove_file(&path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::{tempdir, NamedTempFile};

    #[test]
    fn put_and_get_bytes() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().to_path_buf());
        cas.init().unwrap();

        let data = b"hello world";
        let hash = cas.put(data).unwrap();

        let retrieved = cas.get(&hash).unwrap();
        assert_eq!(retrieved, data);
    }

    #[test]
    fn put_file_and_get() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().to_path_buf());
        cas.init().unwrap();

        let mut file = NamedTempFile::new().unwrap();
        write!(file, "file content").unwrap();

        let hash = cas.put_file(file.path()).unwrap();
        let retrieved = cas.get(&hash).unwrap();
        assert_eq!(retrieved, b"file content");
    }

    #[test]
    fn contains_check() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().to_path_buf());
        cas.init().unwrap();

        let hash = cas.put(b"test data").unwrap();
        assert!(cas.contains(&hash));

        let missing = hash_bytes(b"other");
        assert!(!cas.contains(&missing));
    }

    #[test]
    fn link_to_destination() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().join("cas"));
        cas.init().unwrap();

        let hash = cas.put(b"linked content").unwrap();

        let dest = dir.path().join("output.txt");
        cas.link_to(&hash, &dest).unwrap();

        let content = fs::read_to_string(&dest).unwrap();
        assert_eq!(content, "linked content");
    }

    #[test]
    fn missing_hash_error() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().to_path_buf());
        cas.init().unwrap();

        let missing = hash_bytes(b"not stored");
        let result = cas.get(&missing);
        assert!(result.is_err());
    }

    #[test]
    fn deduplication() {
        let dir = tempdir().unwrap();
        let cas = CAS::new(dir.path().to_path_buf());
        cas.init().unwrap();

        let data = b"duplicate me";
        let h1 = cas.put(data).unwrap();
        let h2 = cas.put(data).unwrap();

        assert_eq!(h1, h2);

        // Verify only one file exists
        let path = cas.get_path(&h1);
        assert!(path.exists());
    }
}
