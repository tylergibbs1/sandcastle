use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{CapabilityCall, ConsoleLevel, ConsoleMessage, ExecutionStatus, OutputValue};

/// Fast monotonic counter for execution IDs (avoids UUID v4 RNG overhead).
static EXECUTION_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The full structured transcript of a sandbox execution.
///
/// Contains resource usage, console output, capability calls, and the final
/// output value. Serializable for storage and later deterministic replay.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTranscript {
    pub execution_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub status: ExecutionStatus,
    pub fuel_consumed: u64,
    pub fuel_limit: u64,
    pub peak_memory_bytes: u64,
    pub memory_limit_bytes: u64,
    pub output: OutputValue,
    pub console: Vec<ConsoleMessage>,
    pub capability_calls: Vec<CapabilityCall>,
    pub input_artifacts: Vec<String>,
    pub output_artifacts: Vec<String>,
}

/// Mutable recorder used during execution to collect console messages,
/// capability calls, and resource usage. Call [`finalize`](Self::finalize) at
/// the end of execution to produce the immutable [`ExecutionTranscript`].
pub struct TranscriptRecorder {
    execution_id: String,
    started_at: DateTime<Utc>,
    start_instant: Instant,
    fuel_limit: u64,
    memory_limit_bytes: u64,
    fuel_consumed: u64,
    peak_memory_bytes: u64,
    output: OutputValue,
    console: Vec<ConsoleMessage>,
    capability_calls: Vec<CapabilityCall>,
    input_artifacts: Vec<String>,
    output_artifacts: Vec<String>,
}

impl TranscriptRecorder {
    /// Create a new recorder. Captures `started_at` and the monotonic
    /// [`Instant`] used to derive relative timestamps.
    pub fn new(fuel_limit: u64, memory_limit_bytes: u64) -> Self {
        Self {
            execution_id: EXECUTION_COUNTER.fetch_add(1, Ordering::Relaxed).to_string(),
            started_at: Utc::now(),
            start_instant: Instant::now(),
            fuel_limit,
            memory_limit_bytes,
            fuel_consumed: 0,
            peak_memory_bytes: 0,
            output: OutputValue::default(),
            console: Vec::new(),
            capability_calls: Vec::new(),
            input_artifacts: Vec::new(),
            output_artifacts: Vec::new(),
        }
    }

    /// Record a console message at the given level. The timestamp is
    /// automatically computed as elapsed milliseconds since execution start.
    pub fn record_console(&mut self, level: ConsoleLevel, message: String) {
        let ts = self.start_instant.elapsed().as_millis() as u64;
        self.console.push(ConsoleMessage {
            level,
            message,
            ts,
        });
    }

    /// Record a host capability call (already fully populated with timing and
    /// response data).
    pub fn record_capability_call(&mut self, call: CapabilityCall) {
        self.capability_calls.push(call);
    }

    /// Set the final output value produced by the guest.
    pub fn set_output(&mut self, output: OutputValue) {
        self.output = output;
    }

    /// Set the total fuel consumed during execution.
    pub fn set_fuel_consumed(&mut self, fuel: u64) {
        self.fuel_consumed = fuel;
    }

    /// Set the peak memory usage observed during execution.
    pub fn set_peak_memory(&mut self, bytes: u64) {
        self.peak_memory_bytes = bytes;
    }

    /// Register input artifact names (call before execution starts).
    pub fn set_input_artifacts(&mut self, names: Vec<String>) {
        self.input_artifacts = names;
    }

    /// Register output artifact names (call after execution completes).
    pub fn set_output_artifacts(&mut self, names: Vec<String>) {
        self.output_artifacts = names;
    }

    /// Consume the recorder, set `finished_at`, and build the final
    /// [`ExecutionTranscript`].
    pub fn finalize(self, status: ExecutionStatus) -> ExecutionTranscript {
        ExecutionTranscript {
            execution_id: self.execution_id,
            started_at: self.started_at,
            finished_at: Some(Utc::now()),
            status,
            fuel_consumed: self.fuel_consumed,
            fuel_limit: self.fuel_limit,
            peak_memory_bytes: self.peak_memory_bytes,
            memory_limit_bytes: self.memory_limit_bytes,
            output: self.output,
            console: self.console,
            capability_calls: self.capability_calls,
            input_artifacts: self.input_artifacts,
            output_artifacts: self.output_artifacts,
        }
    }
}

/// Provides mock capability responses sourced from a previously recorded
/// [`ExecutionTranscript`], enabling deterministic replay of sandbox
/// executions.
///
/// Calls are matched in the order they were originally recorded. The caller
/// should invoke [`get_response`](Self::get_response) with the same sequence
/// of capability/method/input values that the original execution produced;
/// mismatches return `None`.
pub struct ReplayProvider {
    calls: Vec<CapabilityCall>,
    cursor: usize,
}

impl ReplayProvider {
    /// Create a replay provider from a recorded transcript.
    pub fn from_transcript(transcript: &ExecutionTranscript) -> Self {
        Self {
            calls: transcript.capability_calls.clone(),
            cursor: 0,
        }
    }

    /// Return the next recorded response if the capability, method, and input
    /// match the call at the current cursor position.
    ///
    /// Returns `None` when the cursor is past the end of recorded calls or
    /// when the provided arguments do not match the next expected call.
    pub fn get_response(
        &mut self,
        capability: &str,
        method: &str,
        input: &serde_json::Value,
    ) -> Option<serde_json::Value> {
        let call = self.calls.get(self.cursor)?;

        if call.capability != capability || call.method != method || call.input != *input {
            return None;
        }

        self.cursor += 1;
        call.output.clone()
    }

    /// Returns `true` when all recorded calls have been consumed.
    pub fn is_exhausted(&self) -> bool {
        self.cursor >= self.calls.len()
    }

    /// Returns the number of recorded calls remaining.
    pub fn remaining(&self) -> usize {
        self.calls.len().saturating_sub(self.cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ConsoleLevel;

    #[test]
    fn recorder_produces_transcript() {
        let mut recorder = TranscriptRecorder::new(1_000_000, 64 * 1024 * 1024);
        recorder.record_console(ConsoleLevel::Log, "hello".into());
        recorder.set_fuel_consumed(42);
        recorder.set_peak_memory(1024);
        recorder.set_output(OutputValue::String("done".into()));

        let transcript = recorder.finalize(ExecutionStatus::Success);

        assert!(transcript.finished_at.is_some());
        assert_eq!(transcript.fuel_consumed, 42);
        assert_eq!(transcript.peak_memory_bytes, 1024);
        assert_eq!(transcript.console.len(), 1);
        assert_eq!(transcript.console[0].message, "hello");
        assert!(transcript.status.is_success());
    }

    #[test]
    fn replay_matches_in_order() {
        let call = CapabilityCall {
            capability: "http".into(),
            method: "get".into(),
            input: serde_json::json!({"url": "https://example.com"}),
            output: Some(serde_json::json!({"status": 200})),
            error: None,
            duration_ms: 50,
            ts: 10,
        };

        let transcript = ExecutionTranscript {
            execution_id: "test-id".into(),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            status: ExecutionStatus::Success,
            fuel_consumed: 100,
            fuel_limit: 1_000_000,
            peak_memory_bytes: 512,
            memory_limit_bytes: 64 * 1024 * 1024,
            output: OutputValue::Null,
            console: vec![],
            capability_calls: vec![call],
            input_artifacts: vec![],
            output_artifacts: vec![],
        };

        let mut replay = ReplayProvider::from_transcript(&transcript);
        assert_eq!(replay.remaining(), 1);

        // Matching call returns the recorded output.
        let resp = replay.get_response(
            "http",
            "get",
            &serde_json::json!({"url": "https://example.com"}),
        );
        assert_eq!(resp, Some(serde_json::json!({"status": 200})));
        assert!(replay.is_exhausted());

        // No more calls available.
        let resp = replay.get_response("http", "get", &serde_json::json!({}));
        assert!(resp.is_none());
    }

    #[test]
    fn replay_rejects_mismatch() {
        let call = CapabilityCall {
            capability: "http".into(),
            method: "get".into(),
            input: serde_json::json!({"url": "https://example.com"}),
            output: Some(serde_json::json!({"status": 200})),
            error: None,
            duration_ms: 50,
            ts: 10,
        };

        let transcript = ExecutionTranscript {
            execution_id: "test-id".into(),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            status: ExecutionStatus::Success,
            fuel_consumed: 0,
            fuel_limit: 1_000_000,
            peak_memory_bytes: 0,
            memory_limit_bytes: 64 * 1024 * 1024,
            output: OutputValue::Null,
            console: vec![],
            capability_calls: vec![call],
            input_artifacts: vec![],
            output_artifacts: vec![],
        };

        let mut replay = ReplayProvider::from_transcript(&transcript);

        // Wrong method should return None without advancing the cursor.
        let resp = replay.get_response("http", "post", &serde_json::json!({}));
        assert!(resp.is_none());
        assert_eq!(replay.remaining(), 1);
    }
}
