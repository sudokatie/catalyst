//! Caching infrastructure for build results

mod action_cache;
mod cas;
mod hash;
mod metadata;

pub use action_cache::ActionCache;
pub use cas::CAS;
pub use hash::{hash_action, hash_bytes, hash_file, hash_to_hex, hex_to_hash, Hash, Hasher};
pub use metadata::MetadataStore;
