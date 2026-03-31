//! Caching infrastructure for build results

mod hash;

pub use hash::{hash_action, hash_bytes, hash_file, hash_to_hex, hex_to_hash, Hash, Hasher};
