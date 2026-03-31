//! BUILD file parsing

mod ast;
mod build_parser;
mod lexer;

pub use ast::{Arg, BinOp, BuildFile, Expr, Statement};
pub use build_parser::{build_file_to_targets, is_known_rule, ParseError, Parser};
pub use lexer::{LexError, Lexer, Token};
