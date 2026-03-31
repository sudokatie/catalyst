//! Content hashing utilities for build caching

use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use crate::{Action, Error};

/// SHA-256 hash (32 bytes)
pub type Hash = [u8; 32];

/// Incremental hasher for combining multiple inputs
pub struct Hasher {
    inner: Sha256,
}

impl Hasher {
    /// Create a new hasher
    pub fn new() -> Self {
        Self {
            inner: Sha256::new(),
        }
    }

    /// Update the hash with additional data
    pub fn update(&mut self, data: &[u8]) {
        self.inner.update(data);
    }

    /// Update the hash with file contents
    pub fn update_file(&mut self, path: &Path) -> Result<(), Error> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut buffer = [0u8; 8192];

        loop {
            let bytes_read = reader.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            self.inner.update(&buffer[..bytes_read]);
        }

        Ok(())
    }

    /// Finalize and return the hash
    pub fn finalize(self) -> Hash {
        let result = self.inner.finalize();
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&result);
        hash
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

/// Hash a byte slice
pub fn hash_bytes(data: &[u8]) -> Hash {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

/// Hash a file's contents
pub fn hash_file(path: &Path) -> Result<Hash, Error> {
    let mut hasher = Hasher::new();
    hasher.update_file(path)?;
    Ok(hasher.finalize())
}

/// Hash an action for cache key generation.
///
/// The hash includes:
/// - Command (each argument)
/// - Input file paths (sorted)
/// - Output file paths (sorted)
/// - Environment variables (sorted by key)
/// - Working directory
///
/// Note: This does NOT hash input file contents - that should be done
/// separately when computing the full cache key.
pub fn hash_action(action: &Action) -> Hash {
    let mut hasher = Hasher::new();

    // Hash the command
    hasher.update(b"cmd:");
    for arg in &action.command {
        hasher.update(arg.as_bytes());
        hasher.update(b"\0");
    }

    // Hash input paths (sorted for determinism)
    hasher.update(b"inputs:");
    let mut inputs: Vec<_> = action.inputs.iter().collect();
    inputs.sort();
    for input in inputs {
        hasher.update(input.to_string_lossy().as_bytes());
        hasher.update(b"\0");
    }

    // Hash output paths (sorted for determinism)
    hasher.update(b"outputs:");
    let mut outputs: Vec<_> = action.outputs.iter().collect();
    outputs.sort();
    for output in outputs {
        hasher.update(output.to_string_lossy().as_bytes());
        hasher.update(b"\0");
    }

    // Hash environment (sorted for determinism)
    hasher.update(b"env:");
    let mut env: Vec<_> = action.env.iter().collect();
    env.sort_by_key(|(k, _)| *k);
    for (key, value) in env {
        hasher.update(key.as_bytes());
        hasher.update(b"=");
        hasher.update(value.as_bytes());
        hasher.update(b"\0");
    }

    // Hash working directory
    hasher.update(b"cwd:");
    hasher.update(action.working_dir.to_string_lossy().as_bytes());

    hasher.finalize()
}

/// Convert a hash to a hex string
pub fn hash_to_hex(hash: &Hash) -> String {
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Parse a hex string to a hash
pub fn hex_to_hash(hex: &str) -> Option<Hash> {
    if hex.len() != 64 {
        return None;
    }

    let mut hash = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        hash[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn hash_bytes_consistent() {
        let data = b"hello world";
        let h1 = hash_bytes(data);
        let h2 = hash_bytes(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_bytes_different_content() {
        let h1 = hash_bytes(b"hello");
        let h2 = hash_bytes(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_file_consistent() {
        let mut file = NamedTempFile::new().unwrap();
        write!(file, "test content").unwrap();

        let h1 = hash_file(file.path()).unwrap();
        let h2 = hash_file(file.path()).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_file_different_content() {
        let mut f1 = NamedTempFile::new().unwrap();
        let mut f2 = NamedTempFile::new().unwrap();
        write!(f1, "content A").unwrap();
        write!(f2, "content B").unwrap();

        let h1 = hash_file(f1.path()).unwrap();
        let h2 = hash_file(f2.path()).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_action_includes_command() {
        let mut a1 = Action::new(vec!["rustc".to_string()]);
        let mut a2 = Action::new(vec!["gcc".to_string()]);
        
        // Reset IDs to be the same (ID shouldn't affect hash)
        a1 = Action::with_id(1, a1.command);
        a2 = Action::with_id(1, a2.command);

        let h1 = hash_action(&a1);
        let h2 = hash_action(&a2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_action_includes_inputs() {
        let mut a1 = Action::with_id(1, vec!["rustc".to_string()]);
        let mut a2 = Action::with_id(1, vec!["rustc".to_string()]);

        a1.add_input("src/main.rs".into());
        a2.add_input("src/lib.rs".into());

        let h1 = hash_action(&a1);
        let h2 = hash_action(&a2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_action_includes_env() {
        let mut a1 = Action::with_id(1, vec!["rustc".to_string()]);
        let mut a2 = Action::with_id(1, vec!["rustc".to_string()]);

        a1.set_env("RUSTFLAGS", "-O");
        a2.set_env("RUSTFLAGS", "-g");

        let h1 = hash_action(&a1);
        let h2 = hash_action(&a2);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_action_deterministic() {
        let mut action = Action::with_id(1, vec!["rustc".to_string(), "-o".to_string(), "out".to_string()]);
        action.add_input("a.rs".into());
        action.add_input("b.rs".into());
        action.set_env("OPT", "3");

        let h1 = hash_action(&action);
        let h2 = hash_action(&action);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_hex_roundtrip() {
        let original = hash_bytes(b"test data");
        let hex = hash_to_hex(&original);
        let parsed = hex_to_hash(&hex).unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn hex_format() {
        let hash = hash_bytes(b"hello");
        let hex = hash_to_hex(&hash);
        assert_eq!(hex.len(), 64);
        assert!(hex.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn invalid_hex_length() {
        assert!(hex_to_hash("abc").is_none());
        assert!(hex_to_hash("").is_none());
    }
}
