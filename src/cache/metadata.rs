//! Metadata store for file hash caching using SQLite

use rusqlite::{params, Connection, Result as SqlResult};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::Hash;
use crate::Error;

/// SQLite-backed metadata store for caching file hashes
///
/// This avoids re-hashing files that haven't changed by storing
/// file hashes along with their modification times. If the mtime
/// matches, we can return the cached hash instead of re-reading
/// the file.
pub struct MetadataStore {
    conn: Connection,
}

impl MetadataStore {
    /// Create or open a metadata store at the given path
    pub fn new(path: &Path) -> Result<Self, Error> {
        let conn = Connection::open(path)?;
        
        conn.execute(
            "CREATE TABLE IF NOT EXISTS file_hashes (
                path TEXT PRIMARY KEY,
                hash BLOB NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                last_accessed INTEGER NOT NULL
            )",
            [],
        )?;

        // Index for GC queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_last_accessed ON file_hashes(last_accessed)",
            [],
        )?;

        Ok(Self { conn })
    }

    /// Create an in-memory metadata store (for testing)
    pub fn in_memory() -> Result<Self, Error> {
        let conn = Connection::open_in_memory()?;
        
        conn.execute(
            "CREATE TABLE file_hashes (
                path TEXT PRIMARY KEY,
                hash BLOB NOT NULL,
                mtime_secs INTEGER NOT NULL,
                mtime_nanos INTEGER NOT NULL,
                last_accessed INTEGER NOT NULL
            )",
            [],
        )?;

        conn.execute(
            "CREATE INDEX idx_last_accessed ON file_hashes(last_accessed)",
            [],
        )?;

        Ok(Self { conn })
    }

    /// Store a file hash with its modification time
    pub fn store(&self, path: &Path, hash: &Hash, mtime: SystemTime) -> Result<(), Error> {
        let path_str = path.to_string_lossy();
        let duration = mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let mtime_secs = duration.as_secs() as i64;
        let mtime_nanos = duration.subsec_nanos() as i64;
        let now = current_timestamp();

        self.conn.execute(
            "INSERT OR REPLACE INTO file_hashes (path, hash, mtime_secs, mtime_nanos, last_accessed)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![path_str, hash.as_slice(), mtime_secs, mtime_nanos, now],
        )?;

        Ok(())
    }

    /// Get a cached hash if the file's mtime matches
    ///
    /// Returns None if:
    /// - No cached entry exists for this path
    /// - The cached mtime doesn't match the current mtime
    pub fn get(&self, path: &Path, current_mtime: SystemTime) -> Result<Option<Hash>, Error> {
        let path_str = path.to_string_lossy();
        let duration = current_mtime
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let current_secs = duration.as_secs() as i64;
        let current_nanos = duration.subsec_nanos() as i64;

        let result: SqlResult<(Vec<u8>, i64, i64)> = self.conn.query_row(
            "SELECT hash, mtime_secs, mtime_nanos FROM file_hashes WHERE path = ?1",
            params![path_str],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        );

        match result {
            Ok((hash_bytes, stored_secs, stored_nanos)) => {
                // Check if mtime matches
                if stored_secs == current_secs && stored_nanos == current_nanos {
                    // Update last_accessed timestamp
                    let now = current_timestamp();
                    let _ = self.conn.execute(
                        "UPDATE file_hashes SET last_accessed = ?1 WHERE path = ?2",
                        params![now, path_str],
                    );

                    // Convert to Hash
                    if hash_bytes.len() == 32 {
                        let mut hash = [0u8; 32];
                        hash.copy_from_slice(&hash_bytes);
                        Ok(Some(hash))
                    } else {
                        Ok(None)
                    }
                } else {
                    // mtime differs, cache is stale
                    Ok(None)
                }
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Remove entries that haven't been accessed since the given time
    ///
    /// Returns the number of entries removed
    pub fn gc(&self, older_than: SystemTime) -> Result<usize, Error> {
        let duration = older_than
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        let cutoff = duration.as_secs() as i64;

        let removed = self.conn.execute(
            "DELETE FROM file_hashes WHERE last_accessed < ?1",
            params![cutoff],
        )?;

        Ok(removed)
    }

    /// Remove a specific entry
    pub fn remove(&self, path: &Path) -> Result<bool, Error> {
        let path_str = path.to_string_lossy();
        let removed = self.conn.execute(
            "DELETE FROM file_hashes WHERE path = ?1",
            params![path_str],
        )?;
        Ok(removed > 0)
    }

    /// Get the number of entries in the store
    pub fn len(&self) -> Result<usize, Error> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM file_hashes",
            [],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    /// Check if the store is empty
    pub fn is_empty(&self) -> Result<bool, Error> {
        Ok(self.len()? == 0)
    }

    /// Get all paths in the store (for testing/debugging)
    pub fn all_paths(&self) -> Result<Vec<PathBuf>, Error> {
        let mut stmt = self.conn.prepare("SELECT path FROM file_hashes")?;
        let paths = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                Ok(PathBuf::from(path))
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(paths)
    }
}

/// Get current timestamp as seconds since epoch
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    fn test_hash() -> Hash {
        [1u8; 32]
    }

    fn mtime_from_secs(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn store_and_retrieve_matching_mtime() {
        let store = MetadataStore::in_memory().unwrap();
        let path = Path::new("/some/file.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1234567890);

        store.store(path, &hash, mtime).unwrap();
        let result = store.get(path, mtime).unwrap();

        assert_eq!(result, Some(hash));
    }

    #[test]
    fn returns_none_when_mtime_differs() {
        let store = MetadataStore::in_memory().unwrap();
        let path = Path::new("/some/file.rs");
        let hash = test_hash();
        let stored_mtime = mtime_from_secs(1000);
        let current_mtime = mtime_from_secs(2000);

        store.store(path, &hash, stored_mtime).unwrap();
        let result = store.get(path, current_mtime).unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn returns_none_when_not_cached() {
        let store = MetadataStore::in_memory().unwrap();
        let path = Path::new("/nonexistent/file.rs");
        let mtime = mtime_from_secs(1000);

        let result = store.get(path, mtime).unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn gc_removes_old_entries() {
        let store = MetadataStore::in_memory().unwrap();
        
        let path1 = Path::new("/old/file.rs");
        let path2 = Path::new("/new/file.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1000);

        // Store both entries
        store.store(path1, &hash, mtime).unwrap();
        store.store(path2, &hash, mtime).unwrap();

        assert_eq!(store.len().unwrap(), 2);

        // GC with cutoff in the future removes all
        let future = SystemTime::now() + Duration::from_secs(3600);
        let removed = store.gc(future).unwrap();

        assert_eq!(removed, 2);
        assert!(store.is_empty().unwrap());
    }

    #[test]
    fn gc_preserves_recent_entries() {
        let store = MetadataStore::in_memory().unwrap();
        
        let path = Path::new("/recent/file.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1000);

        store.store(path, &hash, mtime).unwrap();

        // GC with cutoff in the past removes nothing
        let past = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        let removed = store.gc(past).unwrap();

        assert_eq!(removed, 0);
        assert_eq!(store.len().unwrap(), 1);
    }

    #[test]
    fn updates_last_accessed_on_get() {
        let store = MetadataStore::in_memory().unwrap();
        
        let path = Path::new("/file.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1000);

        store.store(path, &hash, mtime).unwrap();
        
        // Access the entry
        let _ = store.get(path, mtime).unwrap();

        // Entry should still exist after a GC with recent cutoff
        let recent = SystemTime::now() - Duration::from_secs(1);
        let removed = store.gc(recent).unwrap();

        assert_eq!(removed, 0);
    }

    #[test]
    fn remove_entry() {
        let store = MetadataStore::in_memory().unwrap();
        
        let path = Path::new("/file.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1000);

        store.store(path, &hash, mtime).unwrap();
        assert_eq!(store.len().unwrap(), 1);

        let removed = store.remove(path).unwrap();
        assert!(removed);
        assert!(store.is_empty().unwrap());
    }

    #[test]
    fn all_paths_returns_stored_paths() {
        let store = MetadataStore::in_memory().unwrap();
        
        let path1 = Path::new("/a.rs");
        let path2 = Path::new("/b.rs");
        let hash = test_hash();
        let mtime = mtime_from_secs(1000);

        store.store(path1, &hash, mtime).unwrap();
        store.store(path2, &hash, mtime).unwrap();

        let paths = store.all_paths().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&PathBuf::from("/a.rs")));
        assert!(paths.contains(&PathBuf::from("/b.rs")));
    }

    #[test]
    fn works_with_real_file() {
        let store = MetadataStore::in_memory().unwrap();
        
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "test content").unwrap();
        
        let metadata = file.as_file().metadata().unwrap();
        let mtime = metadata.modified().unwrap();
        let hash = test_hash();

        store.store(file.path(), &hash, mtime).unwrap();
        let result = store.get(file.path(), mtime).unwrap();

        assert_eq!(result, Some(hash));
    }

    #[test]
    fn nanosecond_precision() {
        let store = MetadataStore::in_memory().unwrap();
        let path = Path::new("/file.rs");
        let hash = test_hash();
        
        // Two mtimes with same seconds but different nanoseconds
        let mtime1 = SystemTime::UNIX_EPOCH + Duration::new(1000, 100);
        let mtime2 = SystemTime::UNIX_EPOCH + Duration::new(1000, 200);

        store.store(path, &hash, mtime1).unwrap();
        
        // Same seconds, different nanos should not match
        let result = store.get(path, mtime2).unwrap();
        assert_eq!(result, None);

        // Exact match should work
        let result = store.get(path, mtime1).unwrap();
        assert_eq!(result, Some(hash));
    }
}
