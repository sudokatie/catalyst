//! Error types for Catalyst

use std::path::PathBuf;

/// Main error type for Catalyst
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Parse error at {file}:{line}:{col}: {message}")]
    Parse {
        file: String,
        line: usize,
        col: usize,
        message: String,
    },

    #[error("Unknown target: {0}")]
    UnknownTarget(String),

    #[error("Dependency cycle detected: {}", .0.join(" -> "))]
    Cycle(Vec<String>),

    #[error("Action failed: {command} (exit code {exit_code})\n{stderr}")]
    ActionFailed {
        command: String,
        exit_code: i32,
        stderr: String,
    },

    #[error("Missing input: {0}")]
    MissingInput(PathBuf),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Invalid label: {0}")]
    InvalidLabel(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML error: {0}")]
    Toml(#[from] toml::de::Error),

    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display() {
        let err = Error::Parse {
            file: "BUILD".to_string(),
            line: 10,
            col: 5,
            message: "unexpected token".to_string(),
        };
        assert!(err.to_string().contains("BUILD:10:5"));
        assert!(err.to_string().contains("unexpected token"));
    }

    #[test]
    fn cycle_error() {
        let err = Error::Cycle(vec!["a".to_string(), "b".to_string(), "a".to_string()]);
        assert!(err.to_string().contains("a -> b -> a"));
    }

    #[test]
    fn action_failed_error() {
        let err = Error::ActionFailed {
            command: "rustc".to_string(),
            exit_code: 1,
            stderr: "error[E0001]: something wrong".to_string(),
        };
        assert!(err.to_string().contains("rustc"));
        assert!(err.to_string().contains("exit code 1"));
    }
}
