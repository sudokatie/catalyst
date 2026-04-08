//! Remote execution protocol messages

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Request to execute an action on a remote worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteRequest {
    /// Unique request ID for tracking
    pub request_id: u64,
    /// Command to execute
    pub command: Vec<String>,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Working directory (relative to worker workspace)
    pub working_dir: PathBuf,
    /// Input file hashes for cache validation
    pub input_hashes: HashMap<PathBuf, String>,
    /// Expected output paths
    pub outputs: Vec<PathBuf>,
}

/// Response from a remote worker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteResponse {
    /// Request ID this responds to
    pub request_id: u64,
    /// Execution result
    pub result: ExecuteResult,
}

/// Result of remote execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecuteResult {
    /// Execution succeeded
    Success {
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        /// Output file hashes
        output_hashes: HashMap<PathBuf, String>,
    },
    /// Execution failed
    Failed {
        exit_code: i32,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        error: String,
    },
    /// Worker error (not execution failure)
    Error { message: String },
}

/// Worker status for health checks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    /// Worker identifier
    pub worker_id: String,
    /// Number of active jobs
    pub active_jobs: usize,
    /// Maximum concurrent jobs
    pub max_jobs: usize,
    /// Worker is healthy and accepting jobs
    pub healthy: bool,
}

/// Message types for the protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Execute an action
    Execute(ExecuteRequest),
    /// Execution response
    Response(ExecuteResponse),
    /// Request worker status
    StatusRequest,
    /// Worker status response
    Status(WorkerStatus),
    /// Heartbeat ping
    Ping,
    /// Heartbeat pong
    Pong,
}

impl Message {
    /// Serialize message to bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, bincode::Error> {
        bincode::serialize(self)
    }

    /// Deserialize message from bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, bincode::Error> {
        bincode::deserialize(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_execute_request() {
        let req = ExecuteRequest {
            request_id: 1,
            command: vec!["echo".to_string(), "hello".to_string()],
            env: HashMap::new(),
            working_dir: PathBuf::from("."),
            input_hashes: HashMap::new(),
            outputs: vec![],
        };
        let msg = Message::Execute(req);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Execute(req) = decoded {
            assert_eq!(req.request_id, 1);
            assert_eq!(req.command, vec!["echo", "hello"]);
        } else {
            panic!("Expected Execute message");
        }
    }

    #[test]
    fn serialize_response_success() {
        let resp = ExecuteResponse {
            request_id: 42,
            result: ExecuteResult::Success {
                exit_code: 0,
                stdout: b"output".to_vec(),
                stderr: vec![],
                output_hashes: HashMap::new(),
            },
        };
        let msg = Message::Response(resp);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Response(resp) = decoded {
            assert_eq!(resp.request_id, 42);
            if let ExecuteResult::Success { exit_code, .. } = resp.result {
                assert_eq!(exit_code, 0);
            } else {
                panic!("Expected Success result");
            }
        } else {
            panic!("Expected Response message");
        }
    }

    #[test]
    fn serialize_worker_status() {
        let status = WorkerStatus {
            worker_id: "worker-1".to_string(),
            active_jobs: 2,
            max_jobs: 4,
            healthy: true,
        };
        let msg = Message::Status(status);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Status(s) = decoded {
            assert_eq!(s.worker_id, "worker-1");
            assert_eq!(s.active_jobs, 2);
        } else {
            panic!("Expected Status message");
        }
    }

    #[test]
    fn serialize_ping_pong() {
        let ping = Message::Ping;
        let pong = Message::Pong;
        
        assert!(ping.to_bytes().is_ok());
        assert!(pong.to_bytes().is_ok());
    }

    #[test]
    fn serialize_response_failed() {
        let resp = ExecuteResponse {
            request_id: 100,
            result: ExecuteResult::Failed {
                exit_code: 1,
                stdout: vec![],
                stderr: b"error message".to_vec(),
                error: "command failed".to_string(),
            },
        };
        let msg = Message::Response(resp);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Response(resp) = decoded {
            assert_eq!(resp.request_id, 100);
            if let ExecuteResult::Failed { exit_code, error, .. } = resp.result {
                assert_eq!(exit_code, 1);
                assert_eq!(error, "command failed");
            } else {
                panic!("Expected Failed result");
            }
        } else {
            panic!("Expected Response message");
        }
    }

    #[test]
    fn serialize_response_error() {
        let resp = ExecuteResponse {
            request_id: 200,
            result: ExecuteResult::Error {
                message: "worker crashed".to_string(),
            },
        };
        let msg = Message::Response(resp);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Response(resp) = decoded {
            if let ExecuteResult::Error { message } = resp.result {
                assert_eq!(message, "worker crashed");
            } else {
                panic!("Expected Error result");
            }
        } else {
            panic!("Expected Response message");
        }
    }

    #[test]
    fn serialize_request_with_env() {
        let mut env = HashMap::new();
        env.insert("PATH".to_string(), "/usr/bin".to_string());
        env.insert("HOME".to_string(), "/home/user".to_string());
        
        let req = ExecuteRequest {
            request_id: 50,
            command: vec!["make".to_string()],
            env,
            working_dir: PathBuf::from("/project"),
            input_hashes: HashMap::new(),
            outputs: vec![PathBuf::from("output.o")],
        };
        let msg = Message::Execute(req);
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        if let Message::Execute(req) = decoded {
            assert_eq!(req.env.get("PATH"), Some(&"/usr/bin".to_string()));
            assert_eq!(req.outputs.len(), 1);
        } else {
            panic!("Expected Execute message");
        }
    }

    #[test]
    fn status_request_roundtrip() {
        let msg = Message::StatusRequest;
        let bytes = msg.to_bytes().unwrap();
        let decoded = Message::from_bytes(&bytes).unwrap();
        
        assert!(matches!(decoded, Message::StatusRequest));
    }
}
