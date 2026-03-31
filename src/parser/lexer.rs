//! Lexer for BUILD files

use std::fmt;

/// Token types for BUILD files
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    /// Identifier (rule names, variable names)
    Ident(String),
    /// String literal
    String(String),
    /// Integer literal
    Int(i64),
    /// Boolean literal
    Bool(bool),
    /// (
    LParen,
    /// )
    RParen,
    /// [
    LBracket,
    /// ]
    RBracket,
    /// {
    LBrace,
    /// }
    RBrace,
    /// ,
    Comma,
    /// :
    Colon,
    /// =
    Equals,
    /// +
    Plus,
    /// Newline
    Newline,
    /// End of file
    Eof,
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Ident(s) => write!(f, "{s}"),
            Token::String(s) => write!(f, "\"{s}\""),
            Token::Int(i) => write!(f, "{i}"),
            Token::Bool(b) => write!(f, "{b}"),
            Token::LParen => write!(f, "("),
            Token::RParen => write!(f, ")"),
            Token::LBracket => write!(f, "["),
            Token::RBracket => write!(f, "]"),
            Token::LBrace => write!(f, "{{"),
            Token::RBrace => write!(f, "}}"),
            Token::Comma => write!(f, ","),
            Token::Colon => write!(f, ":"),
            Token::Equals => write!(f, "="),
            Token::Plus => write!(f, "+"),
            Token::Newline => write!(f, "\\n"),
            Token::Eof => write!(f, "EOF"),
        }
    }
}

/// Lexer error
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub message: String,
    pub line: usize,
    pub col: usize,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}", self.line, self.col, self.message)
    }
}

impl std::error::Error for LexError {}

/// Lexer for BUILD files
pub struct Lexer<'a> {
    input: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    pos: usize,
    line: usize,
    col: usize,
    peeked: Option<Token>,
}

impl<'a> Lexer<'a> {
    /// Create a new lexer for the given input
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            chars: input.char_indices().peekable(),
            pos: 0,
            line: 1,
            col: 1,
            peeked: None,
        }
    }

    /// Current line number (1-indexed)
    pub fn line(&self) -> usize {
        self.line
    }

    /// Current column number (1-indexed)
    pub fn col(&self) -> usize {
        self.col
    }

    /// Peek at the next token without consuming it
    pub fn peek(&mut self) -> Result<&Token, LexError> {
        if self.peeked.is_none() {
            self.peeked = Some(self.next_token()?);
        }
        Ok(self.peeked.as_ref().unwrap())
    }

    /// Get the next token
    pub fn next_token(&mut self) -> Result<Token, LexError> {
        if let Some(token) = self.peeked.take() {
            return Ok(token);
        }

        self.skip_whitespace_and_comments();

        let Some(&(pos, ch)) = self.chars.peek() else {
            return Ok(Token::Eof);
        };

        self.pos = pos;

        match ch {
            '(' => {
                self.advance();
                Ok(Token::LParen)
            }
            ')' => {
                self.advance();
                Ok(Token::RParen)
            }
            '[' => {
                self.advance();
                Ok(Token::LBracket)
            }
            ']' => {
                self.advance();
                Ok(Token::RBracket)
            }
            '{' => {
                self.advance();
                Ok(Token::LBrace)
            }
            '}' => {
                self.advance();
                Ok(Token::RBrace)
            }
            ',' => {
                self.advance();
                Ok(Token::Comma)
            }
            ':' => {
                self.advance();
                Ok(Token::Colon)
            }
            '=' => {
                self.advance();
                Ok(Token::Equals)
            }
            '+' => {
                self.advance();
                Ok(Token::Plus)
            }
            '\n' => {
                self.advance();
                self.line += 1;
                self.col = 1;
                Ok(Token::Newline)
            }
            '"' | '\'' => self.read_string(ch),
            '0'..='9' => self.read_number(),
            c if c.is_alphabetic() || c == '_' => self.read_identifier(),
            c => Err(LexError {
                message: format!("unexpected character: {c}"),
                line: self.line,
                col: self.col,
            }),
        }
    }

    fn advance(&mut self) -> Option<(usize, char)> {
        let result = self.chars.next();
        if result.is_some() {
            self.col += 1;
        }
        result
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.chars.peek() {
                Some(&(_, ' ' | '\t' | '\r')) => {
                    self.advance();
                }
                Some(&(_, '#')) => {
                    // Skip comment until end of line
                    while let Some(&(_, ch)) = self.chars.peek() {
                        if ch == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                _ => break,
            }
        }
    }

    fn read_string(&mut self, quote: char) -> Result<Token, LexError> {
        self.advance(); // consume opening quote
        let mut value = String::new();

        loop {
            match self.chars.peek() {
                None => {
                    return Err(LexError {
                        message: "unterminated string".to_string(),
                        line: self.line,
                        col: self.col,
                    });
                }
                Some(&(_, c)) if c == quote => {
                    self.advance();
                    break;
                }
                Some(&(_, '\\')) => {
                    self.advance();
                    match self.chars.peek() {
                        Some(&(_, 'n')) => {
                            self.advance();
                            value.push('\n');
                        }
                        Some(&(_, 't')) => {
                            self.advance();
                            value.push('\t');
                        }
                        Some(&(_, '\\')) => {
                            self.advance();
                            value.push('\\');
                        }
                        Some(&(_, c)) if c == quote => {
                            self.advance();
                            value.push(c);
                        }
                        _ => {
                            return Err(LexError {
                                message: "invalid escape sequence".to_string(),
                                line: self.line,
                                col: self.col,
                            });
                        }
                    }
                }
                Some(&(_, '\n')) => {
                    return Err(LexError {
                        message: "unterminated string".to_string(),
                        line: self.line,
                        col: self.col,
                    });
                }
                Some(&(_, c)) => {
                    self.advance();
                    value.push(c);
                }
            }
        }

        Ok(Token::String(value))
    }

    fn read_number(&mut self) -> Result<Token, LexError> {
        let start = self.pos;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch.is_ascii_digit() {
                self.pos = pos;
                self.advance();
            } else {
                break;
            }
        }

        let end = self.chars.peek().map(|&(p, _)| p).unwrap_or(self.input.len());
        let num_str = &self.input[start..end];
        
        let value = num_str.parse::<i64>().map_err(|_| LexError {
            message: format!("invalid number: {num_str}"),
            line: self.line,
            col: self.col,
        })?;

        Ok(Token::Int(value))
    }

    fn read_identifier(&mut self) -> Result<Token, LexError> {
        let start = self.pos;

        while let Some(&(pos, ch)) = self.chars.peek() {
            if ch.is_alphanumeric() || ch == '_' {
                self.pos = pos;
                self.advance();
            } else {
                break;
            }
        }

        let end = self.chars.peek().map(|&(p, _)| p).unwrap_or(self.input.len());
        let ident = &self.input[start..end];

        // Check for keywords
        match ident {
            "True" | "true" => Ok(Token::Bool(true)),
            "False" | "false" => Ok(Token::Bool(false)),
            _ => Ok(Token::Ident(ident.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_identifiers() {
        let mut lexer = Lexer::new("foo bar_baz _private");
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("foo".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("bar_baz".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("_private".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::Eof);
    }

    #[test]
    fn tokenize_strings_double_quotes() {
        let mut lexer = Lexer::new(r#""hello" "world""#);
        assert_eq!(lexer.next_token().unwrap(), Token::String("hello".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::String("world".to_string()));
    }

    #[test]
    fn tokenize_strings_single_quotes() {
        let mut lexer = Lexer::new("'hello' 'world'");
        assert_eq!(lexer.next_token().unwrap(), Token::String("hello".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::String("world".to_string()));
    }

    #[test]
    fn tokenize_strings_with_escapes() {
        let mut lexer = Lexer::new(r#""hello\nworld" "tab\there""#);
        assert_eq!(lexer.next_token().unwrap(), Token::String("hello\nworld".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::String("tab\there".to_string()));
    }

    #[test]
    fn tokenize_integers() {
        let mut lexer = Lexer::new("123 456 0");
        assert_eq!(lexer.next_token().unwrap(), Token::Int(123));
        assert_eq!(lexer.next_token().unwrap(), Token::Int(456));
        assert_eq!(lexer.next_token().unwrap(), Token::Int(0));
    }

    #[test]
    fn tokenize_booleans() {
        let mut lexer = Lexer::new("True False true false");
        assert_eq!(lexer.next_token().unwrap(), Token::Bool(true));
        assert_eq!(lexer.next_token().unwrap(), Token::Bool(false));
        assert_eq!(lexer.next_token().unwrap(), Token::Bool(true));
        assert_eq!(lexer.next_token().unwrap(), Token::Bool(false));
    }

    #[test]
    fn tokenize_punctuation() {
        let mut lexer = Lexer::new("( ) [ ] { } , : = +");
        assert_eq!(lexer.next_token().unwrap(), Token::LParen);
        assert_eq!(lexer.next_token().unwrap(), Token::RParen);
        assert_eq!(lexer.next_token().unwrap(), Token::LBracket);
        assert_eq!(lexer.next_token().unwrap(), Token::RBracket);
        assert_eq!(lexer.next_token().unwrap(), Token::LBrace);
        assert_eq!(lexer.next_token().unwrap(), Token::RBrace);
        assert_eq!(lexer.next_token().unwrap(), Token::Comma);
        assert_eq!(lexer.next_token().unwrap(), Token::Colon);
        assert_eq!(lexer.next_token().unwrap(), Token::Equals);
        assert_eq!(lexer.next_token().unwrap(), Token::Plus);
    }

    #[test]
    fn skip_comments() {
        let mut lexer = Lexer::new("foo # this is a comment\nbar");
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("foo".to_string()));
        assert_eq!(lexer.next_token().unwrap(), Token::Newline);
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("bar".to_string()));
    }

    #[test]
    fn track_line_column() {
        let mut lexer = Lexer::new("foo\nbar");
        assert_eq!(lexer.line(), 1);
        
        lexer.next_token().unwrap(); // foo
        lexer.next_token().unwrap(); // newline
        
        assert_eq!(lexer.line(), 2);
    }

    #[test]
    fn peek_token() {
        let mut lexer = Lexer::new("foo bar");
        
        assert_eq!(lexer.peek().unwrap(), &Token::Ident("foo".to_string()));
        assert_eq!(lexer.peek().unwrap(), &Token::Ident("foo".to_string())); // peek again
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("foo".to_string())); // consume
        assert_eq!(lexer.next_token().unwrap(), Token::Ident("bar".to_string()));
    }

    #[test]
    fn error_on_unterminated_string() {
        let mut lexer = Lexer::new("\"unterminated");
        let err = lexer.next_token().unwrap_err();
        assert!(err.message.contains("unterminated"));
    }

    #[test]
    fn tokenize_build_file() {
        let input = r#"
rust_binary(
    name = "myapp",
    srcs = ["src/main.rs"],
    deps = [":mylib"],
)
"#;
        let mut lexer = Lexer::new(input);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next_token().unwrap();
            if token == Token::Eof {
                break;
            }
            tokens.push(token);
        }
        
        // Check key tokens are present
        assert!(tokens.contains(&Token::Ident("rust_binary".to_string())));
        assert!(tokens.contains(&Token::Ident("name".to_string())));
        assert!(tokens.contains(&Token::String("myapp".to_string())));
        assert!(tokens.contains(&Token::Ident("srcs".to_string())));
    }
}
