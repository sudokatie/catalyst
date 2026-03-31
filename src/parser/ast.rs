//! AST types for BUILD files

/// Expression node
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// String literal
    String(String),
    /// Integer literal
    Int(i64),
    /// Boolean literal
    Bool(bool),
    /// Identifier reference
    Ident(String),
    /// List expression [a, b, c]
    List(Vec<Expr>),
    /// Dictionary expression {a: b, c: d}
    Dict(Vec<(String, Expr)>),
    /// Function call: func(args)
    Call {
        func: String,
        args: Vec<Arg>,
    },
    /// Binary operation
    BinOp {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

/// Function argument (positional or keyword)
#[derive(Debug, Clone, PartialEq)]
pub struct Arg {
    /// Keyword argument name (None for positional)
    pub name: Option<String>,
    /// Argument value
    pub value: Expr,
}

impl Arg {
    /// Create a positional argument
    pub fn positional(value: Expr) -> Self {
        Self { name: None, value }
    }

    /// Create a keyword argument
    pub fn keyword(name: impl Into<String>, value: Expr) -> Self {
        Self {
            name: Some(name.into()),
            value,
        }
    }

    /// Check if this is a keyword argument
    pub fn is_keyword(&self) -> bool {
        self.name.is_some()
    }
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    /// Addition (+)
    Add,
    /// Equality (==)
    Eq,
    /// Logical and
    And,
    /// Logical or
    Or,
}

/// A parsed BUILD file
#[derive(Debug, Clone, PartialEq)]
pub struct BuildFile {
    /// Top-level statements
    pub statements: Vec<Statement>,
}

impl BuildFile {
    /// Create a new empty BUILD file
    pub fn new() -> Self {
        Self {
            statements: Vec::new(),
        }
    }

    /// Add a statement
    pub fn add_statement(&mut self, stmt: Statement) {
        self.statements.push(stmt);
    }

    /// Get all function calls (rule invocations)
    pub fn calls(&self) -> impl Iterator<Item = (&str, &[Arg])> {
        self.statements.iter().filter_map(|stmt| {
            if let Statement::Expr(Expr::Call { func, args }) = stmt {
                Some((func.as_str(), args.as_slice()))
            } else {
                None
            }
        })
    }
}

impl Default for BuildFile {
    fn default() -> Self {
        Self::new()
    }
}

/// Top-level statement
#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    /// Variable assignment: NAME = value
    Assignment { name: String, value: Expr },
    /// Expression statement (usually a function call)
    Expr(Expr),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_string_expr() {
        let expr = Expr::String("hello".to_string());
        assert_eq!(expr, Expr::String("hello".to_string()));
    }

    #[test]
    fn create_list_expr() {
        let expr = Expr::List(vec![
            Expr::String("a".to_string()),
            Expr::String("b".to_string()),
        ]);
        if let Expr::List(items) = expr {
            assert_eq!(items.len(), 2);
        } else {
            panic!("expected list");
        }
    }

    #[test]
    fn create_call_expr() {
        let expr = Expr::Call {
            func: "rust_binary".to_string(),
            args: vec![
                Arg::keyword("name", Expr::String("myapp".to_string())),
                Arg::keyword("srcs", Expr::List(vec![Expr::String("main.rs".to_string())])),
            ],
        };
        if let Expr::Call { func, args } = expr {
            assert_eq!(func, "rust_binary");
            assert_eq!(args.len(), 2);
            assert!(args[0].is_keyword());
        } else {
            panic!("expected call");
        }
    }

    #[test]
    fn create_binop_expr() {
        let expr = Expr::BinOp {
            op: BinOp::Add,
            left: Box::new(Expr::List(vec![Expr::String("a".to_string())])),
            right: Box::new(Expr::List(vec![Expr::String("b".to_string())])),
        };
        if let Expr::BinOp { op, .. } = expr {
            assert_eq!(op, BinOp::Add);
        } else {
            panic!("expected binop");
        }
    }

    #[test]
    fn positional_arg() {
        let arg = Arg::positional(Expr::Int(42));
        assert!(!arg.is_keyword());
        assert_eq!(arg.value, Expr::Int(42));
    }

    #[test]
    fn keyword_arg() {
        let arg = Arg::keyword("name", Expr::String("value".to_string()));
        assert!(arg.is_keyword());
        assert_eq!(arg.name, Some("name".to_string()));
    }

    #[test]
    fn build_file_calls() {
        let mut bf = BuildFile::new();
        bf.add_statement(Statement::Assignment {
            name: "DEPS".to_string(),
            value: Expr::List(vec![]),
        });
        bf.add_statement(Statement::Expr(Expr::Call {
            func: "rust_binary".to_string(),
            args: vec![Arg::keyword("name", Expr::String("app".to_string()))],
        }));
        bf.add_statement(Statement::Expr(Expr::Call {
            func: "rust_library".to_string(),
            args: vec![],
        }));

        let calls: Vec<_> = bf.calls().collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "rust_binary");
        assert_eq!(calls[1].0, "rust_library");
    }

    #[test]
    fn assignment_statement() {
        let stmt = Statement::Assignment {
            name: "COMMON_DEPS".to_string(),
            value: Expr::List(vec![Expr::String(":util".to_string())]),
        };
        if let Statement::Assignment { name, value } = stmt {
            assert_eq!(name, "COMMON_DEPS");
            assert!(matches!(value, Expr::List(_)));
        } else {
            panic!("expected assignment");
        }
    }
}
