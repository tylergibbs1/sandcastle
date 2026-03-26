use serde::{Deserialize, Serialize};

/// Security mode for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecurityMode {
    /// In-process Wasmtime execution. Sub-millisecond sandbox creation.
    Standard,
    /// Separate worker process with seccomp/landlock. 5-15ms creation overhead.
    Hardened,
}

impl Default for SecurityMode {
    fn default() -> Self {
        Self::Standard
    }
}

/// Status of a completed sandbox execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ExecutionStatus {
    Success,
    Timeout,
    FuelExhausted,
    MemoryExceeded,
    Cancelled,
    GuestError { message: String },
    CapabilityError { message: String },
}

impl ExecutionStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
}

/// Output value from a sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum OutputValue {
    Json(serde_json::Value),
    String(String),
    Bytes(Vec<u8>),
    Null,
}

impl Default for OutputValue {
    fn default() -> Self {
        Self::Null
    }
}

/// A console message captured from the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleMessage {
    pub level: ConsoleLevel,
    pub message: String,
    /// Timestamp in milliseconds relative to execution start.
    pub ts: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleLevel {
    Log,
    Warn,
    Error,
    Debug,
}

/// An input artifact to mount into the sandbox.
#[derive(Debug, Clone)]
pub struct Artifact {
    pub name: String,
    pub data: Vec<u8>,
}

impl Artifact {
    pub fn new(name: impl Into<String>, data: Vec<u8>) -> Self {
        Self {
            name: name.into(),
            data,
        }
    }

    pub fn from_text(name: impl Into<String>, content: &str) -> Self {
        Self {
            name: name.into(),
            data: content.as_bytes().to_vec(),
        }
    }
}

/// An output artifact produced by the sandbox.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputArtifact {
    pub name: String,
    pub data: Vec<u8>,
}

/// A recorded host capability call for the transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityCall {
    pub capability: String,
    pub method: String,
    pub input: serde_json::Value,
    pub output: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    /// Timestamp in milliseconds relative to execution start.
    pub ts: u64,
}
