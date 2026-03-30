//! Catalyst - Build system with hermetic builds and content-addressed caching

pub mod error;
pub mod label;

// Re-export core types
pub use error::Error;
pub use label::Label;
