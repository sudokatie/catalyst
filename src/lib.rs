//! Catalyst - Build system with hermetic builds and content-addressed caching

pub mod action;
pub mod error;
pub mod label;
pub mod parser;
pub mod target;

// Re-export core types
pub use action::{Action, ActionId, ActionKey, ActionResult};
pub use error::Error;
pub use label::Label;
pub use parser::{Arg, BinOp, BuildFile, Expr, Statement, Parser, ParseError, Lexer, Token, LexError};
pub use target::{Target, Value};
