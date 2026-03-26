use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Resource limits for a sandbox execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Limits {
    /// Maximum memory in megabytes.
    pub memory_mb: u32,
    /// Wall-clock timeout for the entire execution.
    pub timeout: Duration,
    /// Fuel units (instruction count cap). 0 = unlimited.
    pub fuel: u64,
    /// Maximum size of the output value in bytes.
    pub max_output_bytes: usize,
    /// Maximum number of input artifacts.
    pub max_input_artifacts: usize,
    /// Maximum total size of input artifacts in bytes.
    pub max_input_bytes: usize,
    /// Maximum number of output artifacts.
    pub max_output_artifacts: usize,
    /// Maximum total size of output artifacts in bytes.
    pub max_output_bytes_artifacts: usize,
    /// Maximum single artifact file size in bytes.
    pub max_artifact_file_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            memory_mb: 32,
            timeout: Duration::from_secs(10),
            fuel: 1_000_000_000,
            max_output_bytes: 1024 * 1024,        // 1 MB
            max_input_artifacts: 16,
            max_input_bytes: 32 * 1024 * 1024,    // 32 MB
            max_output_artifacts: 16,
            max_output_bytes_artifacts: 32 * 1024 * 1024, // 32 MB
            max_artifact_file_bytes: 16 * 1024 * 1024,   // 16 MB
        }
    }
}

/// Per-capability quota limits.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CapabilityLimits {
    /// Maximum number of calls to this capability per execution.
    pub max_calls: u32,
    /// Maximum size of a single request or response payload in bytes.
    pub max_payload_bytes: usize,
    /// Maximum total bytes transferred across all calls.
    pub max_total_bytes: usize,
    /// Maximum wall-clock time for a single capability call.
    pub call_timeout: Duration,
    /// Maximum concurrent in-flight calls.
    pub max_concurrent: u32,
}

impl Default for CapabilityLimits {
    fn default() -> Self {
        Self {
            max_calls: 100,
            max_payload_bytes: 1024 * 1024,      // 1 MB
            max_total_bytes: 10 * 1024 * 1024,   // 10 MB
            call_timeout: Duration::from_secs(5),
            max_concurrent: 4,
        }
    }
}
