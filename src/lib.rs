//! Catalyst - Build system with hermetic builds and content-addressed caching

pub mod action;
pub mod cache;
pub mod error;
pub mod executor;
pub mod graph;
pub mod label;
pub mod parser;
pub mod target;

// Re-export core types
pub use action::{Action, ActionId, ActionKey, ActionResult};
pub use cache::{
    hash_action, hash_bytes, hash_file, hash_to_hex, hex_to_hash, ActionCache, Hash, Hasher,
    MetadataStore, CAS,
};
pub use error::Error;
pub use executor::{execute_sync, ExecutionResult, Executor, LocalExecutor};
pub use graph::{
    ActionNode, FileKind, FileNode, Graph, Node, NodeId, QueryEngine, Resolver, TargetNode,
};
pub use label::Label;
pub use parser::{
    build_file_to_targets, is_known_rule, Arg, BinOp, BuildFile, Expr, LexError, Lexer,
    ParseError, Parser, Statement, Token,
};
pub use target::{Target, Value};
