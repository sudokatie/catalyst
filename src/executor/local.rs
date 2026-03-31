//! Local command executor

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;

use crate::{Action, Error};

/// Result of executing an action
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Exit code (0 = success)
    pub exit_code: i32,
    /// Captured stdout
    pub stdout: Vec<u8>,
    /// Captured stderr
    pub stderr: Vec<u8>,
}

impl ExecutionResult {
    /// Check if execution was successful
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }

    /// Get stdout as string (lossy UTF-8 conversion)
    pub fn stdout_str(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }

    /// Get stderr as string (lossy UTF-8 conversion)
    pub fn stderr_str(&self) -> String {
        String::from_utf8_lossy(&self.stderr).into_owned()
    }
}

/// Trait for action executors
#[async_trait]
pub trait Executor: Send + Sync {
    /// Execute an action and return the result
    async fn execute(&self, action: &Action) -> Result<ExecutionResult, Error>;
}

/// Local executor that runs commands on the current machine
pub struct LocalExecutor {
    /// Additional environment variables to set
    extra_env: HashMap<String, String>,
}

impl LocalExecutor {
    /// Create a new local executor
    pub fn new() -> Self {
        Self {
            extra_env: HashMap::new(),
        }
    }

    /// Add an environment variable for all executed commands
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_env.insert(key.into(), value.into());
        self
    }

    /// Set multiple environment variables
    pub fn with_envs<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (k, v) in vars {
            self.extra_env.insert(k.into(), v.into());
        }
        self
    }
}

impl Default for LocalExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Executor for LocalExecutor {
    async fn execute(&self, action: &Action) -> Result<ExecutionResult, Error> {
        if action.command.is_empty() {
            return Err(Error::ActionFailed {
                command: "<empty>".to_string(),
                exit_code: 1,
                stderr: "No command specified".to_string(),
            });
        }

        let program = &action.command[0];
        let args = &action.command[1..];

        let mut cmd = Command::new(program);
        cmd.args(args);

        // Set working directory
        cmd.current_dir(&action.working_dir);

        // Set environment variables from action
        for (key, value) in &action.env {
            cmd.env(key, value);
        }

        // Set extra environment variables from executor
        for (key, value) in &self.extra_env {
            cmd.env(key, value);
        }

        // Capture stdout and stderr
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                Error::ActionFailed {
                    command: program.clone(),
                    exit_code: 127,
                    stderr: format!("Command not found: {}", program),
                }
            } else {
                Error::Io(e)
            }
        })?;

        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ExecutionResult {
            exit_code,
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Execute an action synchronously (blocking)
pub fn execute_sync(action: &Action) -> Result<ExecutionResult, Error> {
    let rt = tokio::runtime::Runtime::new().map_err(|e| Error::Io(e.into()))?;
    let executor = LocalExecutor::new();
    rt.block_on(executor.execute(action))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_action(command: Vec<&str>) -> Action {
        let cmd: Vec<String> = command.into_iter().map(|s| s.to_string()).collect();
        Action::new(cmd)
    }

    #[tokio::test]
    async fn execute_echo() {
        let executor = LocalExecutor::new();
        let action = test_action(vec!["echo", "hello world"]);

        let result = executor.execute(&action).await.unwrap();

        assert!(result.success());
        assert!(result.stdout_str().contains("hello world"));
    }

    #[tokio::test]
    async fn capture_stdout() {
        let executor = LocalExecutor::new();
        let action = test_action(vec!["echo", "-n", "test output"]);

        let result = executor.execute(&action).await.unwrap();

        assert_eq!(result.stdout_str().trim(), "test output");
    }

    #[tokio::test]
    async fn capture_stderr() {
        let executor = LocalExecutor::new();
        // Use sh to redirect to stderr
        let action = test_action(vec!["sh", "-c", "echo error >&2"]);

        let result = executor.execute(&action).await.unwrap();

        assert!(result.stderr_str().contains("error"));
    }

    #[tokio::test]
    async fn return_exit_code() {
        let executor = LocalExecutor::new();
        let action = test_action(vec!["sh", "-c", "exit 42"]);

        let result = executor.execute(&action).await.unwrap();

        assert_eq!(result.exit_code, 42);
        assert!(!result.success());
    }

    #[tokio::test]
    async fn handle_command_not_found() {
        let executor = LocalExecutor::new();
        let action = test_action(vec!["nonexistent_command_12345"]);

        let result = executor.execute(&action).await;

        assert!(result.is_err());
        if let Err(Error::ActionFailed { exit_code, .. }) = result {
            assert_eq!(exit_code, 127);
        }
    }

    #[tokio::test]
    async fn set_environment_variables() {
        let executor = LocalExecutor::new();
        let mut action = test_action(vec!["sh", "-c", "echo $MY_TEST_VAR"]);
        action.set_env("MY_TEST_VAR", "test_value_123");

        let result = executor.execute(&action).await.unwrap();

        assert!(result.stdout_str().contains("test_value_123"));
    }

    #[tokio::test]
    async fn executor_extra_env() {
        let executor = LocalExecutor::new().with_env("EXTRA_VAR", "extra_value");
        let action = test_action(vec!["sh", "-c", "echo $EXTRA_VAR"]);

        let result = executor.execute(&action).await.unwrap();

        assert!(result.stdout_str().contains("extra_value"));
    }

    #[tokio::test]
    async fn working_directory() {
        let executor = LocalExecutor::new();
        let mut action = test_action(vec!["pwd"]);
        action.working_dir = PathBuf::from("/tmp");

        let result = executor.execute(&action).await.unwrap();

        // On macOS, /tmp is a symlink to /private/tmp
        let stdout = result.stdout_str();
        assert!(stdout.contains("tmp"));
    }

    #[test]
    fn execute_sync_works() {
        let action = test_action(vec!["echo", "sync test"]);
        let result = execute_sync(&action).unwrap();

        assert!(result.success());
        assert!(result.stdout_str().contains("sync test"));
    }
}
