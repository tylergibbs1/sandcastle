use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use crate::error::CapabilityError;
use crate::limits::CapabilityLimits;
use crate::types::CapabilityCall;

// ---------------------------------------------------------------------------
// Capability trait
// ---------------------------------------------------------------------------

/// A host capability that can be invoked from sandboxed WASM guest code.
///
/// Implementations expose one or more methods that the guest can call through
/// the [`CapabilityBridge`]. Each capability has a unique name and a set of
/// typed methods described by [`MethodSchema`].
#[async_trait]
pub trait Capability: Send + Sync {
    /// The unique name of this capability (e.g. `"fs"`, `"http"`, `"kv"`).
    fn name(&self) -> &str;

    /// Describes the methods exposed by this capability for documentation and
    /// TypeScript declaration generation.
    fn methods(&self) -> Vec<MethodSchema>;

    /// Invoke a method on this capability.
    async fn call(
        &self,
        method: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CapabilityError>;
}

// ---------------------------------------------------------------------------
// MethodSchema
// ---------------------------------------------------------------------------

/// Describes a single method on a capability, used for documentation and
/// TypeScript declaration generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodSchema {
    /// Method name (e.g. `"read_file"`).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// JSON-Schema-style description of the input payload.
    pub input_schema: serde_json::Value,
    /// JSON-Schema-style description of the output payload.
    pub output_schema: serde_json::Value,
}

impl MethodSchema {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: serde_json::Value,
        output_schema: serde_json::Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            output_schema,
        }
    }
}

// ---------------------------------------------------------------------------
// CapabilityRegistry
// ---------------------------------------------------------------------------

/// Holds all registered capabilities along with their per-capability limits.
///
/// A single registry is typically shared across many sandbox executions. Each
/// execution gets its own [`CapabilityBridge`] which references the registry
/// and maintains per-execution quota state.
pub struct CapabilityRegistry {
    pub(crate) capabilities: HashMap<String, (Box<dyn Capability>, CapabilityLimits)>,
}

impl CapabilityRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            capabilities: HashMap::new(),
        }
    }

    /// Register a capability with default limits.
    pub fn register(&mut self, capability: Box<dyn Capability>) {
        let name = capability.name().to_owned();
        self.capabilities
            .insert(name, (capability, CapabilityLimits::default()));
    }

    /// Register a capability with explicit limits.
    pub fn register_with_limits(
        &mut self,
        capability: Box<dyn Capability>,
        limits: CapabilityLimits,
    ) {
        let name = capability.name().to_owned();
        self.capabilities.insert(name, (capability, limits));
    }

    /// Look up a capability by name.
    pub fn get(&self, name: &str) -> Option<&(Box<dyn Capability>, CapabilityLimits)> {
        self.capabilities.get(name)
    }

    /// Returns the names of all registered capabilities.
    pub fn capability_names(&self) -> Vec<&str> {
        self.capabilities.keys().map(|s| s.as_str()).collect()
    }

    /// Returns an iterator over all registered capabilities and their limits.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &dyn Capability, &CapabilityLimits)> {
        self.capabilities
            .iter()
            .map(|(name, (cap, limits))| (name.as_str(), cap.as_ref(), limits))
    }
}

impl Default for CapabilityRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// QuotaTracker
// ---------------------------------------------------------------------------

/// Tracks per-capability usage quotas for a single execution.
///
/// All counters use atomics so quota checks are lock-free on the fast path.
/// The [`Semaphore`] handles concurrency limiting.
pub struct QuotaTracker {
    limits: CapabilityLimits,
    call_count: AtomicU32,
    total_bytes: AtomicUsize,
    concurrency: Arc<Semaphore>,
}

impl QuotaTracker {
    /// Create a new tracker from the given limits.
    pub fn new(limits: CapabilityLimits) -> Self {
        let max_concurrent = limits.max_concurrent as usize;
        Self {
            limits,
            call_count: AtomicU32::new(0),
            total_bytes: AtomicUsize::new(0),
            concurrency: Arc::new(Semaphore::new(max_concurrent)),
        }
    }

    /// Atomically check and increment the call counter. Returns an error if
    /// the quota would be exceeded. Uses compare_exchange to avoid TOCTOU races.
    pub fn check_call_count(
        &self,
        capability: &str,
        method: &str,
    ) -> Result<(), CapabilityError> {
        loop {
            let current = self.call_count.load(Ordering::Relaxed);
            if current >= self.limits.max_calls {
                return Err(CapabilityError::CallQuotaExceeded {
                    capability: capability.to_owned(),
                    method: method.to_owned(),
                    count: current,
                    max: self.limits.max_calls,
                });
            }
            if self
                .call_count
                .compare_exchange(current, current + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    /// Check that a single payload does not exceed the per-call size limit.
    pub fn check_payload_size(
        &self,
        capability: &str,
        method: &str,
        size: usize,
    ) -> Result<(), CapabilityError> {
        if size > self.limits.max_payload_bytes {
            return Err(CapabilityError::PayloadTooLarge {
                capability: capability.to_owned(),
                method: method.to_owned(),
                size,
                limit: self.limits.max_payload_bytes,
            });
        }
        Ok(())
    }

    /// Atomically add `bytes` to the running total and check against the
    /// transfer limit. Uses compare_exchange to avoid TOCTOU races.
    pub fn track_bytes(
        &self,
        capability: &str,
        bytes: usize,
    ) -> Result<(), CapabilityError> {
        loop {
            let current = self.total_bytes.load(Ordering::Relaxed);
            let new_total = current + bytes;
            if new_total > self.limits.max_total_bytes {
                return Err(CapabilityError::TransferLimitExceeded {
                    capability: capability.to_owned(),
                    total: new_total,
                    limit: self.limits.max_total_bytes,
                });
            }
            if self
                .total_bytes
                .compare_exchange(current, new_total, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    /// Acquire a concurrency permit. Returns an error immediately if all
    /// permits are taken (try-acquire semantics).
    pub fn try_acquire_concurrency(
        &self,
        capability: &str,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, CapabilityError> {
        match Arc::clone(&self.concurrency).try_acquire_owned() {
            Ok(permit) => Ok(permit),
            Err(_) => Err(CapabilityError::ConcurrencyExceeded {
                capability: capability.to_owned(),
                active: self.limits.max_concurrent,
                max: self.limits.max_concurrent,
            }),
        }
    }

    /// Returns the call timeout configured for this capability.
    pub fn call_timeout(&self) -> std::time::Duration {
        self.limits.call_timeout
    }

    /// Current call count.
    pub fn current_call_count(&self) -> u32 {
        self.call_count.load(Ordering::Relaxed)
    }

    /// Current total bytes transferred.
    pub fn current_total_bytes(&self) -> usize {
        self.total_bytes.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------
// CapabilityBridge
// ---------------------------------------------------------------------------

/// Per-execution bridge that handles serialized calls from the WASM guest,
/// dispatches them to the correct [`Capability`], enforces quotas, and records
/// all calls for the execution transcript.
pub struct CapabilityBridge {
    registry: Arc<CapabilityRegistry>,
    quotas: HashMap<String, QuotaTracker>,
    /// Uses std::sync::Mutex (not tokio) so dispatch_sync can record without await.
    calls: std::sync::Mutex<Vec<CapabilityCall>>,
    /// Instant at which execution started, used to compute relative timestamps.
    execution_start: Instant,
}

impl CapabilityBridge {
    /// Create a new bridge for a single execution.
    ///
    /// A [`QuotaTracker`] is created for each capability in the registry, using
    /// the limits that were registered alongside it.
    pub fn new(registry: Arc<CapabilityRegistry>) -> Self {
        let quotas = registry
            .capabilities
            .iter()
            .map(|(name, (_, limits))| (name.clone(), QuotaTracker::new(limits.clone())))
            .collect();

        Self {
            registry,
            quotas,
            calls: std::sync::Mutex::new(Vec::new()),
            execution_start: Instant::now(),
        }
    }

    /// Dispatch a serialized call from the guest (async path, MessagePack).
    pub async fn dispatch(
        &self,
        capability: &str,
        method: &str,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        let call_start = Instant::now();
        let ts = self.execution_start.elapsed().as_millis() as u64;

        let input: serde_json::Value =
            rmp_serde::from_slice(input_bytes).map_err(|e| {
                CapabilityError::Serialization(format!(
                    "failed to deserialize MessagePack input: {e}"
                ))
            })?;

        let (cap, _) = self.registry.get(capability).ok_or_else(|| {
            CapabilityError::NotFound {
                capability: capability.to_owned(),
                method: method.to_owned(),
            }
        })?;

        let quota = self.quotas.get(capability).ok_or_else(|| {
            CapabilityError::NotFound {
                capability: capability.to_owned(),
                method: method.to_owned(),
            }
        })?;

        quota.check_call_count(capability, method)?;
        quota.check_payload_size(capability, method, input_bytes.len())?;

        let _permit = quota.try_acquire_concurrency(capability)?;

        let timeout_duration = quota.call_timeout();
        let result =
            tokio::time::timeout(timeout_duration, cap.call(method, input.clone())).await;

        let duration_ms = call_start.elapsed().as_millis() as u64;

        let was_timeout = result.is_err();
        let (output_value, error_msg) = match result {
            Ok(Ok(value)) => (Some(value), None),
            Ok(Err(e)) => (None, Some(e.to_string())),
            Err(_) => {
                let err = CapabilityError::Timeout {
                    capability: capability.to_owned(),
                    method: method.to_owned(),
                    elapsed_ms: duration_ms,
                };
                (None, Some(err.to_string()))
            }
        };

        let response_bytes = match &output_value {
            Some(val) => {
                let bytes = rmp_serde::to_vec(val).map_err(|e| {
                    CapabilityError::Serialization(format!(
                        "failed to serialize MessagePack output: {e}"
                    ))
                })?;

                quota.check_payload_size(capability, method, bytes.len())?;
                quota.track_bytes(capability, input_bytes.len() + bytes.len())?;

                bytes
            }
            None => {
                let _ = quota.track_bytes(capability, input_bytes.len());
                Vec::new()
            }
        };

        // Record in transcript
        let call_record = CapabilityCall {
            capability: capability.to_owned(),
            method: method.to_owned(),
            input,
            output: output_value.clone(),
            error: error_msg.clone(),
            duration_ms,
            ts,
        };
        self.record_call(call_record);

        // Return result or propagate the error
        match error_msg {
            Some(err_msg) => {
                // Reconstruct the appropriate error type
                if was_timeout {
                    Err(CapabilityError::Timeout {
                        capability: capability.to_owned(),
                        method: method.to_owned(),
                        elapsed_ms: duration_ms,
                    })
                } else {
                    Err(CapabilityError::InvocationFailed {
                        capability: capability.to_owned(),
                        method: method.to_owned(),
                        message: err_msg,
                    })
                }
            }
            None => Ok(response_bytes),
        }
    }

    /// Synchronous dispatch for use inside WASM host functions.
    ///
    /// Uses JSON (not MessagePack) for the sync path. Does NOT record calls —
    /// the caller (sandbox host function) is responsible for recording via
    /// the TranscriptRecorder to avoid double-recording.
    pub fn dispatch_sync(
        &self,
        capability: &str,
        method: &str,
        input_bytes: &[u8],
    ) -> Result<Vec<u8>, CapabilityError> {
        // Deserialize input as JSON
        let input: serde_json::Value = serde_json::from_slice(input_bytes).map_err(|e| {
            CapabilityError::Serialization(format!("failed to deserialize JSON input: {e}"))
        })?;

        // Look up capability
        let (cap, _) = self.registry.get(capability).ok_or_else(|| {
            CapabilityError::NotFound {
                capability: capability.to_owned(),
                method: method.to_owned(),
            }
        })?;

        // Quota checks (including concurrency)
        let _permit = if let Some(quota) = self.quotas.get(capability) {
            quota.check_call_count(capability, method)?;
            quota.check_payload_size(capability, method, input_bytes.len())?;
            Some(quota.try_acquire_concurrency(capability)?)
        } else {
            None
        };

        // Call capability synchronously
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(cap.call(method, input))
        });

        match result {
            Ok(output_value) => {
                let response_bytes = serde_json::to_vec(&output_value).map_err(|e| {
                    CapabilityError::Serialization(format!(
                        "failed to serialize JSON output: {e}"
                    ))
                })?;

                if let Some(quota) = self.quotas.get(capability) {
                    let _ =
                        quota.track_bytes(capability, input_bytes.len() + response_bytes.len());
                }

                Ok(response_bytes)
            }
            Err(e) => Err(e),
        }
    }

    /// Record a capability call in the bridge's call log.
    fn record_call(&self, call: CapabilityCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }

    /// Return all capability calls recorded during this execution.
    pub fn drain_calls(&self) -> Vec<CapabilityCall> {
        let mut calls = self.calls.lock().expect("calls mutex poisoned");
        std::mem::take(&mut *calls)
    }

    /// Return a snapshot of recorded calls without draining them.
    pub fn calls(&self) -> Vec<CapabilityCall> {
        self.calls.lock().expect("calls mutex poisoned").clone()
    }

    /// Get the quota tracker for a specific capability.
    pub fn quota(&self, capability: &str) -> Option<&QuotaTracker> {
        self.quotas.get(capability)
    }
}

// ---------------------------------------------------------------------------
// SimpleCapability — easy capability for testing and simple use cases
// ---------------------------------------------------------------------------

/// A simple capability backed by a closure, useful for testing and quick setups.
pub struct SimpleCapability {
    capability_name: String,
    handler: Box<
        dyn Fn(&str, serde_json::Value) -> Result<serde_json::Value, CapabilityError>
            + Send
            + Sync,
    >,
}

impl SimpleCapability {
    pub fn new<F>(name: impl Into<String>, handler: F) -> Self
    where
        F: Fn(&str, serde_json::Value) -> Result<serde_json::Value, CapabilityError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            capability_name: name.into(),
            handler: Box::new(handler),
        }
    }
}

#[async_trait]
impl Capability for SimpleCapability {
    fn name(&self) -> &str {
        &self.capability_name
    }

    fn methods(&self) -> Vec<MethodSchema> {
        vec![]
    }

    async fn call(
        &self,
        method: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CapabilityError> {
        (self.handler)(method, input)
    }
}

// ---------------------------------------------------------------------------
// TypeScriptGenerator
// ---------------------------------------------------------------------------

/// Generates TypeScript `.d.ts` declarations from registered capabilities.
pub struct TypeScriptGenerator;

impl TypeScriptGenerator {
    /// Generate a complete `.d.ts` file for all capabilities in the registry.
    pub fn generate(registry: &CapabilityRegistry) -> String {
        let mut output = String::new();
        output.push_str("// Auto-generated by SandCastle. Do not edit.\n\n");
        output.push_str("declare namespace SandCastle {\n");

        let mut names: Vec<&str> = registry.capability_names();
        names.sort();

        for name in &names {
            if let Some((cap, _)) = registry.get(name) {
                output.push_str(&Self::generate_capability(cap.as_ref()));
            }
        }

        output.push_str("}\n");
        output
    }

    fn generate_capability(cap: &dyn Capability) -> String {
        let mut out = String::new();
        let iface_name = to_pascal_case(cap.name());

        out.push_str(&format!("  interface {} {{\n", iface_name));

        for method in cap.methods() {
            if !method.description.is_empty() {
                out.push_str(&format!("    /** {} */\n", method.description));
            }

            let input_ts = json_schema_to_ts_type(&method.input_schema);
            let output_ts = json_schema_to_ts_type(&method.output_schema);

            out.push_str(&format!(
                "    {}(input: {}): Promise<{}>;\n",
                method.name, input_ts, output_ts
            ));
        }

        out.push_str("  }\n\n");

        out.push_str(&format!(
            "  const {}: {};\n\n",
            cap.name(),
            iface_name
        ));

        out
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn to_pascal_case(s: &str) -> String {
    s.split(|c: char| c == '_' || c == '-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    upper + chars.as_str()
                }
            }
        })
        .collect()
}

fn json_schema_to_ts_type(schema: &serde_json::Value) -> String {
    match schema {
        serde_json::Value::Object(obj) => match obj.get("type").and_then(|t| t.as_str()) {
            Some("string") => "string".to_owned(),
            Some("number") | Some("integer") => "number".to_owned(),
            Some("boolean") => "boolean".to_owned(),
            Some("null") => "null".to_owned(),
            Some("array") => {
                let items = obj
                    .get("items")
                    .map(json_schema_to_ts_type)
                    .unwrap_or_else(|| "unknown".to_owned());
                format!("{}[]", items)
            }
            Some("object") => {
                if let Some(serde_json::Value::Object(props)) = obj.get("properties") {
                    let required: Vec<&str> = obj
                        .get("required")
                        .and_then(|r| r.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    let fields: Vec<String> = props
                        .iter()
                        .map(|(key, val)| {
                            let ts_type = json_schema_to_ts_type(val);
                            let optional =
                                if required.contains(&key.as_str()) { "" } else { "?" };
                            format!("{}{}: {}", key, optional, ts_type)
                        })
                        .collect();

                    format!("{{ {} }}", fields.join("; "))
                } else {
                    "Record<string, unknown>".to_owned()
                }
            }
            _ => "unknown".to_owned(),
        },
        serde_json::Value::Null => "void".to_owned(),
        _ => "unknown".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pascal_case() {
        assert_eq!(to_pascal_case("hello_world"), "HelloWorld");
        assert_eq!(to_pascal_case("http-client"), "HttpClient");
        assert_eq!(to_pascal_case("fs"), "Fs");
        assert_eq!(to_pascal_case("kv_store"), "KvStore");
    }

    #[test]
    fn test_json_schema_to_ts_string() {
        let schema = serde_json::json!({"type": "string"});
        assert_eq!(json_schema_to_ts_type(&schema), "string");
    }

    #[test]
    fn test_json_schema_to_ts_array() {
        let schema = serde_json::json!({"type": "array", "items": {"type": "number"}});
        assert_eq!(json_schema_to_ts_type(&schema), "number[]");
    }

    #[test]
    fn test_json_schema_to_ts_void() {
        assert_eq!(json_schema_to_ts_type(&serde_json::Value::Null), "void");
    }

    #[test]
    fn test_quota_tracker_call_count() {
        let limits = CapabilityLimits {
            max_calls: 2,
            ..Default::default()
        };
        let tracker = QuotaTracker::new(limits);
        assert!(tracker.check_call_count("test", "m").is_ok());
        assert!(tracker.check_call_count("test", "m").is_ok());
        assert!(tracker.check_call_count("test", "m").is_err());
    }

    #[test]
    fn test_quota_tracker_payload_size() {
        let limits = CapabilityLimits {
            max_payload_bytes: 100,
            ..Default::default()
        };
        let tracker = QuotaTracker::new(limits);
        assert!(tracker.check_payload_size("test", "m", 50).is_ok());
        assert!(tracker.check_payload_size("test", "m", 101).is_err());
    }

    #[test]
    fn test_quota_tracker_total_bytes() {
        let limits = CapabilityLimits {
            max_total_bytes: 200,
            ..Default::default()
        };
        let tracker = QuotaTracker::new(limits);
        assert!(tracker.track_bytes("test", 100).is_ok());
        assert!(tracker.track_bytes("test", 100).is_ok());
        assert!(tracker.track_bytes("test", 1).is_err());
        // After rollback, counter should be at 200, not 201
        assert_eq!(tracker.current_total_bytes(), 200);
    }

    struct EchoCapability;

    #[async_trait]
    impl Capability for EchoCapability {
        fn name(&self) -> &str {
            "echo"
        }

        fn methods(&self) -> Vec<MethodSchema> {
            vec![MethodSchema::new(
                "echo",
                "Echoes the input back",
                serde_json::json!({"type": "object", "properties": {"msg": {"type": "string"}}, "required": ["msg"]}),
                serde_json::json!({"type": "object", "properties": {"msg": {"type": "string"}}, "required": ["msg"]}),
            )]
        }

        async fn call(
            &self,
            _method: &str,
            input: serde_json::Value,
        ) -> Result<serde_json::Value, CapabilityError> {
            Ok(input)
        }
    }

    #[test]
    fn test_registry_and_typescript_generation() {
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(EchoCapability));

        assert!(registry.get("echo").is_some());
        assert!(registry.get("missing").is_none());

        let ts = TypeScriptGenerator::generate(&registry);
        assert!(ts.contains("interface Echo"));
        assert!(ts.contains("echo(input:"));
        assert!(ts.contains("Promise<"));
    }

    #[tokio::test]
    async fn test_bridge_dispatch_roundtrip() {
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(EchoCapability));

        let bridge = CapabilityBridge::new(Arc::new(registry));

        let input = serde_json::json!({"msg": "hello"});
        let input_bytes = rmp_serde::to_vec(&input).unwrap();

        let result = bridge.dispatch("echo", "echo", &input_bytes).await;
        assert!(result.is_ok());

        let output_bytes = result.unwrap();
        let output: serde_json::Value = rmp_serde::from_slice(&output_bytes).unwrap();
        assert_eq!(output, input);

        let calls = bridge.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].capability, "echo");
        assert_eq!(calls[0].method, "echo");
        assert!(calls[0].error.is_none());
    }

    #[tokio::test]
    async fn test_bridge_dispatch_not_found() {
        let registry = CapabilityRegistry::new();
        let bridge = CapabilityBridge::new(Arc::new(registry));

        let input_bytes = rmp_serde::to_vec(&serde_json::json!({})).unwrap();
        let result = bridge.dispatch("missing", "method", &input_bytes).await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            CapabilityError::NotFound { .. }
        ));
    }
}
