//! Build actions - atomic units of work

use std::collections::HashMap;
use std::path::PathBuf;

/// Unique identifier for an action
pub type ActionId = u64;

/// SHA-256 hash used as cache key
pub type ActionKey = [u8; 32];

/// Atomic build action.
///
/// An action represents a single command to execute with declared inputs and outputs.
/// Actions are hermetic: same inputs should always produce same outputs.
#[derive(Debug, Clone)]
pub struct Action {
    /// Unique identifier for this action
    pub id: ActionId,
    /// Command to execute (e.g., ["rustc", "-o", "out", "in.rs"])
    pub command: Vec<String>,
    /// Declared input files
    pub inputs: Vec<PathBuf>,
    /// Declared output files
    pub outputs: Vec<PathBuf>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Working directory for execution
    pub working_dir: PathBuf,
}

impl Action {
    /// Create a new action with the given command
    pub fn new(command: Vec<String>) -> Self {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        
        Self {
            id: COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            command,
            inputs: Vec::new(),
            outputs: Vec::new(),
            env: HashMap::new(),
            working_dir: PathBuf::from("."),
        }
    }

    /// Create an action with a specific ID (for testing/deserialization)
    pub fn with_id(id: ActionId, command: Vec<String>) -> Self {
        Self {
            id,
            command,
            inputs: Vec::new(),
            outputs: Vec::new(),
            env: HashMap::new(),
            working_dir: PathBuf::from("."),
        }
    }

    /// Add an input file
    pub fn add_input(&mut self, path: PathBuf) {
        self.inputs.push(path);
    }

    /// Add an output file
    pub fn add_output(&mut self, path: PathBuf) {
        self.outputs.push(path);
    }

    /// Set an environment variable
    pub fn set_env(&mut self, key: &str, value: &str) {
        self.env.insert(key.to_string(), value.to_string());
    }

    /// Set the working directory
    pub fn set_working_dir(&mut self, path: PathBuf) {
        self.working_dir = path;
    }

    /// Get the command as a string for display
    pub fn command_string(&self) -> String {
        self.command.join(" ")
    }
}

/// Result of executing an action
#[derive(Debug, Clone)]
pub struct ActionResult {
    /// Exit code from the command
    pub exit_code: i32,
    /// Hashes of output files (path -> hash)
    pub output_hashes: HashMap<PathBuf, ActionKey>,
    /// Captured stdout
    pub stdout: String,
    /// Captured stderr
    pub stderr: String,
    /// Execution duration
    pub duration: std::time::Duration,
}

impl ActionResult {
    /// Create a successful result
    pub fn success(duration: std::time::Duration) -> Self {
        Self {
            exit_code: 0,
            output_hashes: HashMap::new(),
            stdout: String::new(),
            stderr: String::new(),
            duration,
        }
    }

    /// Create a failed result
    pub fn failure(exit_code: i32, stderr: String, duration: std::time::Duration) -> Self {
        Self {
            exit_code,
            output_hashes: HashMap::new(),
            stdout: String::new(),
            stderr,
            duration,
        }
    }

    /// Check if the action succeeded
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_action_with_command() {
        let action = Action::new(vec!["rustc".to_string(), "-o".to_string(), "out".to_string()]);
        assert_eq!(action.command, vec!["rustc", "-o", "out"]);
        assert!(action.id > 0);
    }

    #[test]
    fn action_has_unique_ids() {
        let a1 = Action::new(vec!["cmd1".to_string()]);
        let a2 = Action::new(vec!["cmd2".to_string()]);
        assert_ne!(a1.id, a2.id);
    }

    #[test]
    fn add_inputs() {
        let mut action = Action::new(vec!["rustc".to_string()]);
        action.add_input(PathBuf::from("src/main.rs"));
        action.add_input(PathBuf::from("src/lib.rs"));
        
        assert_eq!(action.inputs.len(), 2);
        assert_eq!(action.inputs[0], PathBuf::from("src/main.rs"));
    }

    #[test]
    fn add_outputs() {
        let mut action = Action::new(vec!["rustc".to_string()]);
        action.add_output(PathBuf::from("target/debug/myapp"));
        
        assert_eq!(action.outputs.len(), 1);
        assert_eq!(action.outputs[0], PathBuf::from("target/debug/myapp"));
    }

    #[test]
    fn set_environment_variables() {
        let mut action = Action::new(vec!["cargo".to_string(), "build".to_string()]);
        action.set_env("RUSTFLAGS", "-C opt-level=3");
        action.set_env("CARGO_TARGET_DIR", "/tmp/build");
        
        assert_eq!(action.env.get("RUSTFLAGS"), Some(&"-C opt-level=3".to_string()));
        assert_eq!(action.env.get("CARGO_TARGET_DIR"), Some(&"/tmp/build".to_string()));
    }

    #[test]
    fn action_is_clonable() {
        let mut action = Action::new(vec!["echo".to_string(), "hello".to_string()]);
        action.add_input(PathBuf::from("input.txt"));
        action.set_env("FOO", "bar");
        
        let cloned = action.clone();
        assert_eq!(cloned.id, action.id);
        assert_eq!(cloned.command, action.command);
        assert_eq!(cloned.inputs, action.inputs);
        assert_eq!(cloned.env, action.env);
    }

    #[test]
    fn command_string() {
        let action = Action::new(vec!["rustc".to_string(), "-o".to_string(), "out".to_string(), "main.rs".to_string()]);
        assert_eq!(action.command_string(), "rustc -o out main.rs");
    }

    #[test]
    fn action_result_success() {
        let result = ActionResult::success(std::time::Duration::from_secs(1));
        assert!(result.is_success());
        assert_eq!(result.exit_code, 0);
    }

    #[test]
    fn action_result_failure() {
        let result = ActionResult::failure(1, "error: not found".to_string(), std::time::Duration::from_millis(100));
        assert!(!result.is_success());
        assert_eq!(result.exit_code, 1);
        assert_eq!(result.stderr, "error: not found");
    }
}
