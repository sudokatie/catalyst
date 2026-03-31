//! BUILD file parser

use super::ast::{Arg, BinOp, BuildFile, Expr, Statement};
use super::lexer::{LexError, Lexer, Token};
use crate::{Error, Label, Target, Value};
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

/// Parser error
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for ParseError {}

impl From<LexError> for ParseError {
    fn from(e: LexError) -> Self {
        Self {
            message: e.message,
            line: e.line,
            col: e.col,
        }
    }
}

/// BUILD file parser
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current: Token,
    line: usize,
    col: usize,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the given input
    pub fn new(input: &'a str) -> Result<Self, ParseError> {
        let mut lexer = Lexer::new(input);
        let line = lexer.line();
        let col = lexer.col();
        let current = lexer.next_token()?;
        
        Ok(Self {
            lexer,
            current,
            line,
            col,
        })
    }

    /// Parse the entire BUILD file
    pub fn parse(&mut self) -> Result<BuildFile, ParseError> {
        let mut bf = BuildFile::new();
        
        // Skip leading newlines
        self.skip_newlines()?;

        while !self.is_at_end() {
            let stmt = self.parse_statement()?;
            bf.add_statement(stmt);
            self.skip_newlines()?;
        }

        Ok(bf)
    }

    fn advance(&mut self) -> Result<Token, ParseError> {
        self.line = self.lexer.line();
        self.col = self.lexer.col();
        let prev = std::mem::replace(&mut self.current, self.lexer.next_token()?);
        Ok(prev)
    }

    fn is_at_end(&self) -> bool {
        matches!(self.current, Token::Eof)
    }

    fn skip_newlines(&mut self) -> Result<(), ParseError> {
        while matches!(self.current, Token::Newline) {
            self.advance()?;
        }
        Ok(())
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        if &self.current == expected {
            self.advance()?;
            Ok(())
        } else {
            Err(ParseError {
                message: format!("expected {expected}, got {}", self.current),
                line: self.line,
                col: self.col,
            })
        }
    }

    fn parse_statement(&mut self) -> Result<Statement, ParseError> {
        // Check for assignment: IDENT = expr
        if let Token::Ident(name) = &self.current {
            let name = name.clone();
            self.advance()?;
            
            if matches!(self.current, Token::Equals) {
                self.advance()?;
                let value = self.parse_expr()?;
                return Ok(Statement::Assignment { name, value });
            } else if matches!(self.current, Token::LParen) {
                // Function call
                let call = self.parse_call(name)?;
                return Ok(Statement::Expr(call));
            } else {
                // Just an identifier expression (rare but valid)
                return Ok(Statement::Expr(Expr::Ident(name)));
            }
        }

        // Otherwise parse as expression
        let expr = self.parse_expr()?;
        Ok(Statement::Expr(expr))
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or_expr()
    }

    fn parse_or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_and_expr()?;
        
        while let Token::Ident(op) = &self.current {
            if op == "or" {
                self.advance()?;
                let right = self.parse_and_expr()?;
                left = Expr::BinOp {
                    op: BinOp::Or,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        
        Ok(left)
    }

    fn parse_and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_add_expr()?;
        
        while let Token::Ident(op) = &self.current {
            if op == "and" {
                self.advance()?;
                let right = self.parse_add_expr()?;
                left = Expr::BinOp {
                    op: BinOp::And,
                    left: Box::new(left),
                    right: Box::new(right),
                };
            } else {
                break;
            }
        }
        
        Ok(left)
    }

    fn parse_add_expr(&mut self) -> Result<Expr, ParseError> {
        let mut left = self.parse_primary()?;
        
        while matches!(self.current, Token::Plus) {
            self.advance()?;
            let right = self.parse_primary()?;
            left = Expr::BinOp {
                op: BinOp::Add,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        
        Ok(left)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        match &self.current {
            Token::String(s) => {
                let s = s.clone();
                self.advance()?;
                Ok(Expr::String(s))
            }
            Token::Int(i) => {
                let i = *i;
                self.advance()?;
                Ok(Expr::Int(i))
            }
            Token::Bool(b) => {
                let b = *b;
                self.advance()?;
                Ok(Expr::Bool(b))
            }
            Token::Ident(name) => {
                let name = name.clone();
                self.advance()?;
                
                if matches!(self.current, Token::LParen) {
                    self.parse_call(name)
                } else {
                    Ok(Expr::Ident(name))
                }
            }
            Token::LBracket => self.parse_list(),
            Token::LBrace => self.parse_dict(),
            Token::LParen => {
                self.advance()?;
                let expr = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(expr)
            }
            _ => Err(ParseError {
                message: format!("unexpected token: {}", self.current),
                line: self.line,
                col: self.col,
            }),
        }
    }

    fn parse_call(&mut self, func: String) -> Result<Expr, ParseError> {
        self.expect(&Token::LParen)?;
        self.skip_newlines()?;
        
        let mut args = Vec::new();
        
        while !matches!(self.current, Token::RParen | Token::Eof) {
            let arg = self.parse_arg()?;
            args.push(arg);
            
            self.skip_newlines()?;
            
            if matches!(self.current, Token::Comma) {
                self.advance()?;
                self.skip_newlines()?;
            } else {
                break;
            }
        }
        
        self.expect(&Token::RParen)?;
        
        Ok(Expr::Call { func, args })
    }

    fn parse_arg(&mut self) -> Result<Arg, ParseError> {
        // Check for keyword argument: name = value
        if let Token::Ident(name) = &self.current {
            let name = name.clone();
            let _saved_line = self.line;
            let _saved_col = self.col;
            self.advance()?;
            
            if matches!(self.current, Token::Equals) {
                self.advance()?;
                let value = self.parse_expr()?;
                return Ok(Arg::keyword(name, value));
            } else if matches!(self.current, Token::LParen) {
                // It's a function call, backtrack
                let call = self.parse_call(name)?;
                return Ok(Arg::positional(call));
            } else {
                // Positional identifier or error
                // The identifier was already consumed, so we need to return it
                return Ok(Arg::positional(Expr::Ident(name)));
            }
        }
        
        // Positional argument
        let value = self.parse_expr()?;
        Ok(Arg::positional(value))
    }

    fn parse_list(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::LBracket)?;
        self.skip_newlines()?;
        
        let mut items = Vec::new();
        
        while !matches!(self.current, Token::RBracket | Token::Eof) {
            let item = self.parse_expr()?;
            items.push(item);
            
            self.skip_newlines()?;
            
            if matches!(self.current, Token::Comma) {
                self.advance()?;
                self.skip_newlines()?;
            } else {
                break;
            }
        }
        
        self.expect(&Token::RBracket)?;
        
        Ok(Expr::List(items))
    }

    fn parse_dict(&mut self) -> Result<Expr, ParseError> {
        self.expect(&Token::LBrace)?;
        self.skip_newlines()?;
        
        let mut entries = Vec::new();
        
        while !matches!(self.current, Token::RBrace | Token::Eof) {
            // Key must be a string
            let key = match &self.current {
                Token::String(s) => {
                    let s = s.clone();
                    self.advance()?;
                    s
                }
                _ => {
                    return Err(ParseError {
                        message: "dict key must be a string".to_string(),
                        line: self.line,
                        col: self.col,
                    });
                }
            };
            
            self.expect(&Token::Colon)?;
            let value = self.parse_expr()?;
            entries.push((key, value));
            
            self.skip_newlines()?;
            
            if matches!(self.current, Token::Comma) {
                self.advance()?;
                self.skip_newlines()?;
            } else {
                break;
            }
        }
        
        self.expect(&Token::RBrace)?;
        
        Ok(Expr::Dict(entries))
    }
}

/// Known rule types for validation
const KNOWN_RULES: &[&str] = &[
    "rust_binary",
    "rust_library",
    "rust_test",
    "cc_binary",
    "cc_library",
    "cc_test",
    "genrule",
    "filegroup",
    "test_suite",
    "alias",
    "exports_files",
];

/// Check if a rule type is known
pub fn is_known_rule(name: &str) -> bool {
    KNOWN_RULES.contains(&name)
}

/// Convert a parsed BUILD file to Target objects.
///
/// Takes the parsed AST and the package path, returns a list of targets
/// defined in the file.
pub fn build_file_to_targets(bf: &BuildFile, pkg: &str) -> Result<Vec<Target>, Error> {
    // Build variable environment from assignments
    let mut env: HashMap<String, Expr> = HashMap::new();
    
    for stmt in &bf.statements {
        if let Statement::Assignment { name, value } = stmt {
            env.insert(name.clone(), value.clone());
        }
    }
    
    let mut targets = Vec::new();
    
    for (rule_type, args) in bf.calls() {
        // Skip non-rule calls like glob(), select(), etc.
        if !is_known_rule(rule_type) {
            continue;
        }
        
        let target = call_to_target(rule_type, args, pkg, &env)?;
        targets.push(target);
    }
    
    Ok(targets)
}

/// Convert a single rule call to a Target
fn call_to_target(
    rule_type: &str,
    args: &[Arg],
    pkg: &str,
    env: &HashMap<String, Expr>,
) -> Result<Target, Error> {
    // Extract name (required for all rules except exports_files)
    let name = find_string_arg(args, "name", env)?
        .ok_or_else(|| Error::MissingAttribute {
            rule: rule_type.to_string(),
            attr: "name".to_string(),
        })?;
    
    let label = Label::new(pkg, &name);
    let mut target = Target::new(label, rule_type);
    
    // Extract srcs
    if let Some(srcs) = find_string_list_arg(args, "srcs", env)? {
        for src in srcs {
            target.add_src(PathBuf::from(src));
        }
    }
    
    // Extract deps - convert to Labels
    if let Some(deps) = find_string_list_arg(args, "deps", env)? {
        for dep in deps {
            let dep_label = Label::parse(&dep)?.resolve(pkg);
            target.add_dep(dep_label);
        }
    }
    
    // Extract outs (for genrule)
    if let Some(outs) = find_string_list_arg(args, "outs", env)? {
        for out in outs {
            target.add_out(PathBuf::from(out));
        }
    }
    
    // Store remaining attributes
    for arg in args {
        if let Some(attr_name) = &arg.name {
            // Skip already-processed attributes
            if matches!(attr_name.as_str(), "name" | "srcs" | "deps" | "outs") {
                continue;
            }
            
            let value = expr_to_value(&arg.value, env)?;
            target.set_attr(attr_name, value);
        }
    }
    
    Ok(target)
}

/// Find a string-valued keyword argument
fn find_string_arg(
    args: &[Arg],
    name: &str,
    env: &HashMap<String, Expr>,
) -> Result<Option<String>, Error> {
    for arg in args {
        if arg.name.as_deref() == Some(name) {
            return match eval_expr(&arg.value, env)? {
                Expr::String(s) => Ok(Some(s)),
                other => Err(Error::InvalidAttributeType {
                    attr: name.to_string(),
                    expected: "string".to_string(),
                    got: expr_type_name(&other),
                }),
            };
        }
    }
    Ok(None)
}

/// Find a string-list-valued keyword argument
fn find_string_list_arg(
    args: &[Arg],
    name: &str,
    env: &HashMap<String, Expr>,
) -> Result<Option<Vec<String>>, Error> {
    for arg in args {
        if arg.name.as_deref() == Some(name) {
            return match eval_expr(&arg.value, env)? {
                Expr::List(items) => {
                    let mut strings = Vec::new();
                    for item in items {
                        match item {
                            Expr::String(s) => strings.push(s),
                            other => {
                                return Err(Error::InvalidAttributeType {
                                    attr: name.to_string(),
                                    expected: "list of strings".to_string(),
                                    got: format!("list containing {}", expr_type_name(&other)),
                                });
                            }
                        }
                    }
                    Ok(Some(strings))
                }
                other => Err(Error::InvalidAttributeType {
                    attr: name.to_string(),
                    expected: "list".to_string(),
                    got: expr_type_name(&other),
                }),
            };
        }
    }
    Ok(None)
}

/// Evaluate an expression, resolving variable references
fn eval_expr(expr: &Expr, env: &HashMap<String, Expr>) -> Result<Expr, Error> {
    match expr {
        Expr::Ident(name) => {
            env.get(name)
                .cloned()
                .ok_or_else(|| Error::UndefinedVariable(name.clone()))
        }
        Expr::BinOp { op: BinOp::Add, left, right } => {
            let left_val = eval_expr(left, env)?;
            let right_val = eval_expr(right, env)?;
            
            match (left_val, right_val) {
                (Expr::List(mut l), Expr::List(r)) => {
                    l.extend(r);
                    Ok(Expr::List(l))
                }
                (Expr::String(mut l), Expr::String(r)) => {
                    l.push_str(&r);
                    Ok(Expr::String(l))
                }
                _ => Err(Error::InvalidAttributeType {
                    attr: "+".to_string(),
                    expected: "matching list or string types".to_string(),
                    got: "incompatible types".to_string(),
                }),
            }
        }
        Expr::List(items) => {
            let mut evaled = Vec::new();
            for item in items {
                evaled.push(eval_expr(item, env)?);
            }
            Ok(Expr::List(evaled))
        }
        // Primitives and other expressions pass through
        _ => Ok(expr.clone()),
    }
}

/// Convert an Expr to a Value for storage in Target.attrs
fn expr_to_value(expr: &Expr, env: &HashMap<String, Expr>) -> Result<Value, Error> {
    let evaled = eval_expr(expr, env)?;
    
    match evaled {
        Expr::String(s) => Ok(Value::String(s)),
        Expr::Int(i) => Ok(Value::Int(i)),
        Expr::Bool(b) => Ok(Value::Bool(b)),
        Expr::List(items) => {
            let values: Result<Vec<_>, _> = items
                .iter()
                .map(|item| expr_to_value(item, env))
                .collect();
            Ok(Value::List(values?))
        }
        Expr::Dict(entries) => {
            let mut map = HashMap::new();
            for (k, v) in entries {
                map.insert(k, expr_to_value(&v, env)?);
            }
            Ok(Value::Dict(map))
        }
        _ => Ok(Value::String(format!("{:?}", evaled))),
    }
}

/// Get a type name for error messages
fn expr_type_name(expr: &Expr) -> String {
    match expr {
        Expr::String(_) => "string".to_string(),
        Expr::Int(_) => "int".to_string(),
        Expr::Bool(_) => "bool".to_string(),
        Expr::Ident(_) => "identifier".to_string(),
        Expr::List(_) => "list".to_string(),
        Expr::Dict(_) => "dict".to_string(),
        Expr::Call { .. } => "call".to_string(),
        Expr::BinOp { .. } => "expression".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_function_call() {
        let input = r#"foo(name = "bar")"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        assert_eq!(bf.statements.len(), 1);
        if let Statement::Expr(Expr::Call { func, args }) = &bf.statements[0] {
            assert_eq!(func, "foo");
            assert_eq!(args.len(), 1);
            assert_eq!(args[0].name, Some("name".to_string()));
        } else {
            panic!("expected call");
        }
    }

    #[test]
    fn parse_list() {
        let input = r#"["a", "b", "c"]"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        if let Statement::Expr(Expr::List(items)) = &bf.statements[0] {
            assert_eq!(items.len(), 3);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn parse_nested_calls() {
        let input = r#"
rust_binary(
    name = "myapp",
    srcs = ["src/main.rs"],
    deps = [":mylib"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        assert_eq!(bf.statements.len(), 1);
        let calls: Vec<_> = bf.calls().collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "rust_binary");
        assert_eq!(calls[0].1.len(), 3);
    }

    #[test]
    fn parse_assignment() {
        let input = r#"COMMON_DEPS = [":util", ":logging"]"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        if let Statement::Assignment { name, value } = &bf.statements[0] {
            assert_eq!(name, "COMMON_DEPS");
            if let Expr::List(items) = value {
                assert_eq!(items.len(), 2);
            } else {
                panic!("expected list value");
            }
        } else {
            panic!("expected assignment");
        }
    }

    #[test]
    fn parse_list_concatenation() {
        let input = r#"srcs = ["a.rs"] + ["b.rs"]"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        if let Statement::Assignment { value, .. } = &bf.statements[0] {
            if let Expr::BinOp { op: BinOp::Add, .. } = value {
                // OK
            } else {
                panic!("expected binop");
            }
        } else {
            panic!("expected assignment");
        }
    }

    #[test]
    fn parse_dict() {
        let input = r#"{"key": "value", "num": 42}"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        if let Statement::Expr(Expr::Dict(entries)) = &bf.statements[0] {
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[0].0, "key");
            assert_eq!(entries[1].0, "num");
        } else {
            panic!("expected dict");
        }
    }

    #[test]
    fn parse_multiple_statements() {
        let input = r#"
DEPS = [":common"]

rust_library(
    name = "mylib",
    deps = DEPS,
)

rust_binary(
    name = "myapp",
    deps = [":mylib"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        assert_eq!(bf.statements.len(), 3);
        
        // First is assignment
        assert!(matches!(bf.statements[0], Statement::Assignment { .. }));
        
        // Last two are calls
        let calls: Vec<_> = bf.calls().collect();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn parse_error_on_invalid_syntax() {
        let input = r#"foo(name =)"#;
        let mut parser = Parser::new(input).unwrap();
        let result = parser.parse();
        assert!(result.is_err());
    }

    #[test]
    fn parse_glob_call() {
        let input = r#"srcs = glob(["src/**/*.rs"])"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        
        if let Statement::Assignment { value, .. } = &bf.statements[0] {
            if let Expr::Call { func, .. } = value {
                assert_eq!(func, "glob");
            } else {
                panic!("expected call");
            }
        } else {
            panic!("expected assignment");
        }
    }

    // ===== build_file_to_targets tests =====

    #[test]
    fn convert_rust_binary_to_target() {
        let input = r#"
rust_binary(
    name = "myapp",
    srcs = ["src/main.rs", "src/lib.rs"],
    deps = [":mylib", "//other:dep"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        let targets = build_file_to_targets(&bf, "pkg").unwrap();
        
        assert_eq!(targets.len(), 1);
        let t = &targets[0];
        assert_eq!(t.label.package, "pkg");
        assert_eq!(t.label.name, "myapp");
        assert_eq!(t.rule_type, "rust_binary");
        assert_eq!(t.srcs.len(), 2);
        assert_eq!(t.srcs[0], PathBuf::from("src/main.rs"));
        assert_eq!(t.deps.len(), 2);
        assert_eq!(t.deps[0].package, "pkg");  // Resolved relative
        assert_eq!(t.deps[0].name, "mylib");
        assert_eq!(t.deps[1].package, "other");  // Absolute unchanged
        assert_eq!(t.deps[1].name, "dep");
    }

    #[test]
    fn convert_genrule_to_target() {
        let input = r#"
genrule(
    name = "generate_config",
    srcs = ["config.in"],
    outs = ["config.json"],
    cmd = "process $< > $@",
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        let targets = build_file_to_targets(&bf, "tools").unwrap();
        
        assert_eq!(targets.len(), 1);
        let t = &targets[0];
        assert_eq!(t.rule_type, "genrule");
        assert_eq!(t.outs.len(), 1);
        assert_eq!(t.outs[0], PathBuf::from("config.json"));
        assert_eq!(t.get_attr("cmd").unwrap().as_str(), Some("process $< > $@"));
    }

    #[test]
    fn handle_deps_as_labels() {
        let input = r#"
rust_library(
    name = "mylib",
    deps = [":local", "//abs/path:target"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        let targets = build_file_to_targets(&bf, "my/pkg").unwrap();
        
        assert_eq!(targets[0].deps.len(), 2);
        // Relative resolved
        assert_eq!(targets[0].deps[0].absolute(), "//my/pkg:local");
        // Absolute kept
        assert_eq!(targets[0].deps[1].absolute(), "//abs/path:target");
    }

    #[test]
    fn handle_srcs_as_pathbuf() {
        let input = r#"
rust_library(
    name = "lib",
    srcs = ["src/a.rs", "src/b.rs", "src/sub/c.rs"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        let targets = build_file_to_targets(&bf, "pkg").unwrap();
        
        assert_eq!(targets[0].srcs.len(), 3);
        assert_eq!(targets[0].srcs[2], PathBuf::from("src/sub/c.rs"));
    }

    #[test]
    fn error_on_missing_name() {
        let input = r#"
rust_binary(
    srcs = ["main.rs"],
)
"#;
        let mut parser = Parser::new(input).unwrap();
        let bf = parser.parse().unwrap();
        let result = build_file_to_targets(&bf, "pkg");
        
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("name"));
    }
}
