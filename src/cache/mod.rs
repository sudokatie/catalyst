//! Caching infrastructure for build results

mod cas;
mod hash;

pub use cas::CAS;
pub use hash::{hash_action, hash_bytes, hash_file, hash_to_hex, hex_to_hash, Hash, Hasher};
