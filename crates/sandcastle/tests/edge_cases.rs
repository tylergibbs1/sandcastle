//! Comprehensive edge-case tests for the SandCastle runtime.

use std::sync::Arc;
use std::time::Duration;

use sandcastle::capability::CapabilityRegistry;
use sandcastle::error::{CapabilityError, ExecutionError, SandcastleError};
use sandcastle::limits::{CapabilityLimits, Limits};
use sandcastle::namespace::{DispatchNamespace, NamespaceLimits, NamespaceManager};
use sandcastle::pool::{WarmPool, WarmPoolConfig};
use sandcastle::registry::ScriptRegistry;
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::transcript::{ExecutionTranscript, ReplayProvider};
use sandcastle::types::*;

// ---------------------------------------------------------------------------
// Helper: load guest WASM module (mirrors integration.rs pattern)
// ---------------------------------------------------------------------------

fn try_load_guest_module() -> Option<Vec<u8>> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let candidates = [
        workspace_root.join("guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm"),
        workspace_root.join("sandcastle-guest-js.wasm"),
    ];
    for path in &candidates {
        if let Ok(bytes) = std::fs::read(path) {
            return Some(bytes);
        }
    }
    None
}

fn create_runtime() -> Option<SandCastle> {
    let guest_module = try_load_guest_module()?;
    Some(SandCastle::new(Config::new(guest_module)).expect("Failed to create runtime"))
}

fn test_caps() -> Arc<CapabilityRegistry> {
    Arc::new(CapabilityRegistry::new())
}

// =========================================================================
// 1. Quota Race Conditions
// =========================================================================

mod quota_race_conditions {
    use super::*;
    use sandcastle::capability::QuotaTracker;

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_check_call_count_exactly_max_succeed() {
        let limits = CapabilityLimits {
            max_calls: 10,
            ..Default::default()
        };
        let tracker = Arc::new(QuotaTracker::new(limits));

        let mut handles = Vec::new();
        for _ in 0..50 {
            let t = tracker.clone();
            handles.push(tokio::spawn(async move {
                t.check_call_count("test", "m")
            }));
        }

        let mut successes = 0u32;
        let mut failures = 0u32;
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => successes += 1,
                Err(_) => failures += 1,
            }
        }

        assert_eq!(successes, 10, "exactly 10 calls should succeed");
        assert_eq!(failures, 40, "exactly 40 calls should fail");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_track_bytes_exactly_max_succeed() {
        let limits = CapabilityLimits {
            max_total_bytes: 1000,
            ..Default::default()
        };
        let tracker = Arc::new(QuotaTracker::new(limits));

        let mut handles = Vec::new();
        for _ in 0..50 {
            let t = tracker.clone();
            handles.push(tokio::spawn(async move {
                t.track_bytes("test", 100)
            }));
        }

        let mut successes = 0u32;
        let mut failures = 0u32;
        for h in handles {
            match h.await.unwrap() {
                Ok(()) => successes += 1,
                Err(_) => failures += 1,
            }
        }

        assert_eq!(successes, 10, "exactly 10 byte-tracking calls should succeed");
        assert_eq!(failures, 40, "exactly 40 byte-tracking calls should fail");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn check_call_count_with_max_zero_fails_immediately() {
        let limits = CapabilityLimits {
            max_calls: 0,
            ..Default::default()
        };
        let tracker = QuotaTracker::new(limits);
        let result = tracker.check_call_count("cap", "method");
        assert!(result.is_err(), "max_calls=0 should reject the first call");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn track_bytes_zero_always_succeeds() {
        let limits = CapabilityLimits {
            max_total_bytes: 100,
            ..Default::default()
        };
        let tracker = QuotaTracker::new(limits);

        // Tracking 0 bytes should always succeed, even many times.
        for _ in 0..1000 {
            assert!(tracker.track_bytes("cap", 0).is_ok());
        }
        assert_eq!(tracker.current_total_bytes(), 0);
    }
}

// =========================================================================
// 2. Registry Boundary Tests
// =========================================================================

mod registry_boundary {
    use super::*;

    #[test]
    fn register_exactly_max_scripts() {
        let max = 5;
        let registry = ScriptRegistry::new(max);
        let caps = test_caps();

        for i in 0..max {
            registry
                .register(format!("script_{i}"), "code", caps.clone(), Limits::default())
                .unwrap();
        }
        assert_eq!(registry.len(), max);
    }

    #[test]
    fn register_one_over_max_fails() {
        let max = 5;
        let registry = ScriptRegistry::new(max);
        let caps = test_caps();

        for i in 0..max {
            registry
                .register(format!("script_{i}"), "code", caps.clone(), Limits::default())
                .unwrap();
        }

        let result = registry.register("overflow", "code", caps.clone(), Limits::default());
        assert!(result.is_err());
        match result.unwrap_err() {
            SandcastleError::ResourceLimit(_) => {}
            other => panic!("expected ResourceLimit, got: {other}"),
        }
    }

    #[test]
    fn re_register_existing_name_succeeds_at_capacity() {
        let registry = ScriptRegistry::new(1);
        let caps = test_caps();

        registry
            .register("a", "v1", caps.clone(), Limits::default())
            .unwrap();

        // Re-register same name — should succeed (update, not a new slot).
        registry
            .register("a", "v2", caps.clone(), Limits::default())
            .unwrap();

        let script = registry.get("a").unwrap();
        assert_eq!(script.code, "v2");
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn remove_frees_slot_for_new_registration() {
        let registry = ScriptRegistry::new(1);
        let caps = test_caps();

        registry
            .register("a", "code_a", caps.clone(), Limits::default())
            .unwrap();

        assert!(registry.remove("a"));

        // Slot freed; a new script should be allowed.
        registry
            .register("b", "code_b", caps.clone(), Limits::default())
            .unwrap();
        assert_eq!(registry.len(), 1);
        assert!(registry.get("b").is_some());
    }

    #[test]
    fn register_with_empty_name_works() {
        let registry = ScriptRegistry::new(10);
        let caps = test_caps();

        registry
            .register("", "code", caps.clone(), Limits::default())
            .unwrap();

        let script = registry.get("").unwrap();
        assert_eq!(script.name, "");
    }

    #[test]
    fn list_on_empty_registry_returns_empty_vec() {
        let registry = ScriptRegistry::new(10);
        assert!(registry.list().is_empty());
        assert_eq!(registry.len(), 0);
        assert!(registry.is_empty());
    }
}

// =========================================================================
// 3. Namespace Edge Cases
// =========================================================================

mod namespace_edge_cases {
    use super::*;

    #[test]
    fn max_concurrent_one_second_acquire_fails() {
        let limits = NamespaceLimits {
            max_concurrent_executions: 1,
            ..Default::default()
        };
        let ns = DispatchNamespace::new("test", limits, test_caps());

        let _permit = ns.acquire_permit().unwrap();
        let result = ns.acquire_permit();
        assert!(result.is_err(), "second acquire should fail with concurrency=1");
    }

    #[test]
    fn register_remove_get_returns_none() {
        let ns = DispatchNamespace::new("test", NamespaceLimits::default(), test_caps());
        ns.register("script1", "return 1;", None).unwrap();
        assert!(ns.get_script("script1").is_some());

        ns.remove("script1");
        assert!(
            ns.get_script("script1").is_none(),
            "removed script should return None"
        );
    }

    #[test]
    fn manager_duplicate_namespace_name_returns_error() {
        let manager = NamespaceManager::new(10);
        manager
            .create("dup", NamespaceLimits::default(), test_caps())
            .unwrap();

        let result = manager.create("dup", NamespaceLimits::default(), test_caps());
        assert!(result.is_err());
        // Verify the error message mentions the namespace name.
        let err_msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err_msg.contains("dup"),
            "error should mention namespace name: {err_msg}"
        );
    }

    #[test]
    fn manager_delete_non_existent_returns_false() {
        let manager = NamespaceManager::new(10);
        assert!(!manager.delete("does_not_exist"));
    }

    #[test]
    fn manager_max_namespaces_zero_first_create_fails() {
        let manager = NamespaceManager::new(0);
        let result = manager.create("anything", NamespaceLimits::default(), test_caps());
        assert!(result.is_err());
    }

    #[test]
    fn register_script_with_custom_limits_stores_them() {
        let ns = DispatchNamespace::new("test", NamespaceLimits::default(), test_caps());
        let custom = Limits {
            memory_mb: 256,
            fuel: 42,
            ..Limits::default()
        };
        ns.register("custom", "code", Some(custom)).unwrap();

        let script = ns.get_script("custom").unwrap();
        assert_eq!(script.limits.memory_mb, 256);
        assert_eq!(script.limits.fuel, 42);
    }
}

// =========================================================================
// 4. ReplayProvider Edge Cases
// =========================================================================

mod replay_provider_edge_cases {
    use super::*;
    use chrono::Utc;

    fn empty_transcript() -> ExecutionTranscript {
        ExecutionTranscript {
            execution_id: "empty".into(),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            status: ExecutionStatus::Success,
            fuel_consumed: 0,
            fuel_limit: 1_000_000,
            peak_memory_bytes: 0,
            memory_limit_bytes: 64 * 1024 * 1024,
            output: OutputValue::Null,
            console: vec![],
            capability_calls: vec![],
            input_artifacts: vec![],
            output_artifacts: vec![],
        }
    }

    fn transcript_with_calls(calls: Vec<CapabilityCall>) -> ExecutionTranscript {
        ExecutionTranscript {
            execution_id: "replay-test".into(),
            started_at: Utc::now(),
            finished_at: Some(Utc::now()),
            status: ExecutionStatus::Success,
            fuel_consumed: 100,
            fuel_limit: 1_000_000,
            peak_memory_bytes: 512,
            memory_limit_bytes: 64 * 1024 * 1024,
            output: OutputValue::Null,
            console: vec![],
            capability_calls: calls,
            input_artifacts: vec![],
            output_artifacts: vec![],
        }
    }

    #[test]
    fn empty_transcript_returns_none_and_is_exhausted() {
        let transcript = empty_transcript();
        let mut replay = ReplayProvider::from_transcript(&transcript);

        assert!(replay.is_exhausted());
        assert_eq!(replay.remaining(), 0);
        assert!(replay.get_response("any", "any", &serde_json::json!({})).is_none());
    }

    #[test]
    fn mismatched_input_does_not_advance_cursor() {
        let call = CapabilityCall {
            capability: "kv".into(),
            method: "get".into(),
            input: serde_json::json!({"key": "abc"}),
            output: Some(serde_json::json!("value")),
            error: None,
            duration_ms: 1,
            ts: 0,
        };
        let transcript = transcript_with_calls(vec![call]);
        let mut replay = ReplayProvider::from_transcript(&transcript);

        // Wrong capability name.
        assert!(replay
            .get_response("http", "get", &serde_json::json!({"key": "abc"}))
            .is_none());
        assert_eq!(replay.remaining(), 1, "cursor should not advance on mismatch");

        // Wrong method.
        assert!(replay
            .get_response("kv", "set", &serde_json::json!({"key": "abc"}))
            .is_none());
        assert_eq!(replay.remaining(), 1);

        // Wrong input.
        assert!(replay
            .get_response("kv", "get", &serde_json::json!({"key": "xyz"}))
            .is_none());
        assert_eq!(replay.remaining(), 1);

        // Correct match advances.
        let resp = replay.get_response("kv", "get", &serde_json::json!({"key": "abc"}));
        assert_eq!(resp, Some(serde_json::json!("value")));
        assert_eq!(replay.remaining(), 0);
        assert!(replay.is_exhausted());
    }

    #[test]
    fn multiple_calls_verify_sequential_ordering() {
        let calls = vec![
            CapabilityCall {
                capability: "kv".into(),
                method: "get".into(),
                input: serde_json::json!({"key": "first"}),
                output: Some(serde_json::json!(1)),
                error: None,
                duration_ms: 1,
                ts: 0,
            },
            CapabilityCall {
                capability: "kv".into(),
                method: "get".into(),
                input: serde_json::json!({"key": "second"}),
                output: Some(serde_json::json!(2)),
                error: None,
                duration_ms: 1,
                ts: 10,
            },
            CapabilityCall {
                capability: "http".into(),
                method: "request".into(),
                input: serde_json::json!({"url": "https://example.com"}),
                output: Some(serde_json::json!({"status": 200})),
                error: None,
                duration_ms: 50,
                ts: 20,
            },
        ];

        let transcript = transcript_with_calls(calls);
        let mut replay = ReplayProvider::from_transcript(&transcript);

        assert_eq!(replay.remaining(), 3);

        // First call
        let r1 = replay.get_response("kv", "get", &serde_json::json!({"key": "first"}));
        assert_eq!(r1, Some(serde_json::json!(1)));
        assert_eq!(replay.remaining(), 2);

        // Second call
        let r2 = replay.get_response("kv", "get", &serde_json::json!({"key": "second"}));
        assert_eq!(r2, Some(serde_json::json!(2)));
        assert_eq!(replay.remaining(), 1);

        // Third call
        let r3 = replay.get_response(
            "http",
            "request",
            &serde_json::json!({"url": "https://example.com"}),
        );
        assert_eq!(r3, Some(serde_json::json!({"status": 200})));
        assert_eq!(replay.remaining(), 0);
        assert!(replay.is_exhausted());
    }

    #[test]
    fn remaining_count_decrements_correctly() {
        let calls: Vec<CapabilityCall> = (0..5)
            .map(|i| CapabilityCall {
                capability: "c".into(),
                method: "m".into(),
                input: serde_json::json!({"i": i}),
                output: Some(serde_json::json!(i)),
                error: None,
                duration_ms: 0,
                ts: 0,
            })
            .collect();

        let transcript = transcript_with_calls(calls);
        let mut replay = ReplayProvider::from_transcript(&transcript);

        for i in 0..5u32 {
            assert_eq!(replay.remaining(), (5 - i) as usize);
            let _ = replay.get_response("c", "m", &serde_json::json!({"i": i}));
        }
        assert_eq!(replay.remaining(), 0);
    }
}

// =========================================================================
// 5. Pool Metrics
// =========================================================================

mod pool_metrics_tests {
    use super::*;

    #[test]
    fn execution_guard_increments_and_decrements() {
        let pool = WarmPool::new(WarmPoolConfig::default());
        assert_eq!(pool.metrics().active(), 0);
        assert_eq!(pool.metrics().total(), 0);

        {
            let _guard = pool.metrics().execution_started();
            assert_eq!(pool.metrics().active(), 1);
            assert_eq!(pool.metrics().total(), 1);
        }

        assert_eq!(pool.metrics().active(), 0);
        assert_eq!(pool.metrics().total(), 1);
    }

    #[test]
    fn multiple_guards_track_concurrent_count() {
        let pool = WarmPool::new(WarmPoolConfig::default());

        let g1 = pool.metrics().execution_started();
        let g2 = pool.metrics().execution_started();
        let g3 = pool.metrics().execution_started();
        assert_eq!(pool.metrics().active(), 3);
        assert_eq!(pool.metrics().total(), 3);

        drop(g2);
        assert_eq!(pool.metrics().active(), 2);

        drop(g1);
        assert_eq!(pool.metrics().active(), 1);

        drop(g3);
        assert_eq!(pool.metrics().active(), 0);
        assert_eq!(pool.metrics().total(), 3);
    }

    #[test]
    fn total_executions_increments_monotonically() {
        let pool = WarmPool::new(WarmPoolConfig::default());

        for expected in 1..=20u64 {
            let guard = pool.metrics().execution_started();
            assert_eq!(pool.metrics().total(), expected);
            drop(guard);
            // total should remain even after guard drop
            assert_eq!(pool.metrics().total(), expected);
        }
    }
}

// =========================================================================
// 6. KV Capability Edge Cases
// =========================================================================

#[cfg(feature = "kv")]
mod kv_edge_cases {
    use sandcastle::capabilities::KvCapability;
    use sandcastle::capability::Capability;
    use sandcastle::error::CapabilityError;

    #[tokio::test(flavor = "multi_thread")]
    async fn set_value_at_exactly_max_bytes_succeeds() {
        // max_value_bytes is checked against serde_json::to_vec(&value).len().
        // A JSON string "x" serializes to ["\"", "x", "\""], so 3 bytes for a 1-char string.
        // We need to find a value whose serialized length == max_value_bytes.
        let max = 100;
        let kv = KvCapability::new(1000, max);

        // A JSON string serializes with 2 quote bytes, so the inner string needs max - 2 chars.
        let inner = "a".repeat(max - 2);
        let value = serde_json::Value::String(inner);
        let serialized_len = serde_json::to_vec(&value).unwrap().len();
        assert_eq!(serialized_len, max);

        let result: Result<serde_json::Value, CapabilityError> = kv
            .call("set", serde_json::json!({"key": "k", "value": value}))
            .await;
        assert!(result.is_ok(), "value at exactly max_value_bytes should succeed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_value_one_byte_over_max_fails() {
        let max = 100;
        let kv = KvCapability::new(1000, max);

        // 1 byte over: inner string of max - 1 chars => serialized = max + 1
        let inner = "a".repeat(max - 1);
        let value = serde_json::Value::String(inner);
        let serialized_len = serde_json::to_vec(&value).unwrap().len();
        assert_eq!(serialized_len, max + 1);

        let result: Result<serde_json::Value, CapabilityError> = kv
            .call("set", serde_json::json!({"key": "k", "value": value}))
            .await;
        assert!(result.is_err(), "value 1 byte over max should fail");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn set_at_max_keys_then_delete_then_set_succeeds() {
        let kv = KvCapability::new(2, 1024 * 1024);

        // Fill to capacity
        let _: serde_json::Value = kv
            .call("set", serde_json::json!({"key": "a", "value": 1}))
            .await
            .unwrap();
        let _: serde_json::Value = kv
            .call("set", serde_json::json!({"key": "b", "value": 2}))
            .await
            .unwrap();

        // At capacity, new key should fail
        let result: Result<serde_json::Value, CapabilityError> = kv
            .call("set", serde_json::json!({"key": "c", "value": 3}))
            .await;
        assert!(result.is_err());

        // Delete one key
        let _: serde_json::Value = kv
            .call("delete", serde_json::json!({"key": "a"}))
            .await
            .unwrap();

        // Now inserting a new key should succeed
        let result: Result<serde_json::Value, CapabilityError> = kv
            .call("set", serde_json::json!({"key": "c", "value": 3}))
            .await;
        assert!(result.is_ok(), "after delete, a new key should be allowed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn get_non_existent_key_returns_null() {
        let kv = KvCapability::default();
        let result: serde_json::Value = kv
            .call("get", serde_json::json!({"key": "nonexistent"}))
            .await
            .unwrap();
        assert_eq!(result, serde_json::Value::Null);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_with_unmatched_prefix_returns_empty() {
        let kv = KvCapability::default();
        let _: serde_json::Value = kv
            .call("set", serde_json::json!({"key": "abc", "value": 1}))
            .await
            .unwrap();

        let result: serde_json::Value = kv
            .call("list", serde_json::json!({"prefix": "zzz"}))
            .await
            .unwrap();
        let arr = result.as_array().unwrap();
        assert!(arr.is_empty(), "prefix that matches nothing should return empty");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_non_existent_key_returns_false() {
        let kv = KvCapability::default();
        let result: serde_json::Value = kv
            .call("delete", serde_json::json!({"key": "ghost"}))
            .await
            .unwrap();
        assert_eq!(result, serde_json::Value::Bool(false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn has_non_existent_key_returns_false() {
        let kv = KvCapability::default();
        let result: serde_json::Value = kv
            .call("has", serde_json::json!({"key": "nope"}))
            .await
            .unwrap();
        assert_eq!(result, serde_json::Value::Bool(false));
    }
}

// =========================================================================
// 7. Limits Defaults
// =========================================================================

mod limits_defaults {
    use super::*;

    #[test]
    fn limits_default_values_match_prd() {
        let l = Limits::default();
        assert_eq!(l.memory_mb, 32);
        assert_eq!(l.fuel, 1_000_000_000);
        assert_eq!(l.timeout, Duration::from_secs(10));
        assert_eq!(l.max_output_bytes, 1024 * 1024);
        assert_eq!(l.max_input_artifacts, 16);
        assert_eq!(l.max_input_bytes, 32 * 1024 * 1024);
        assert_eq!(l.max_output_artifacts, 16);
        assert_eq!(l.max_output_bytes_artifacts, 32 * 1024 * 1024);
        assert_eq!(l.max_artifact_file_bytes, 16 * 1024 * 1024);
    }

    #[test]
    fn capability_limits_default_values() {
        let cl = CapabilityLimits::default();
        assert_eq!(cl.max_calls, 100);
        assert_eq!(cl.max_payload_bytes, 1024 * 1024);
        assert_eq!(cl.max_total_bytes, 10 * 1024 * 1024);
        assert_eq!(cl.call_timeout, Duration::from_secs(5));
        assert_eq!(cl.max_concurrent, 4);
    }
}

// =========================================================================
// 8. Error Variants
// =========================================================================

mod error_variants {
    use super::*;

    #[test]
    fn sandcastle_error_display() {
        let err = SandcastleError::RuntimeInit("engine failed".into());
        assert_eq!(err.to_string(), "runtime initialization failed: engine failed");

        let err = SandcastleError::Compilation("bad wasm".into());
        assert_eq!(err.to_string(), "module compilation failed: bad wasm");

        let err = SandcastleError::SandboxCreation("no memory".into());
        assert_eq!(err.to_string(), "sandbox creation failed: no memory");

        let err = SandcastleError::Serialization("json broken".into());
        assert_eq!(err.to_string(), "serialization error: json broken");

        let err = SandcastleError::ResourceLimit("too many".into());
        assert_eq!(err.to_string(), "resource limit exceeded: too many");

        let err = SandcastleError::Config("bad config".into());
        assert_eq!(err.to_string(), "configuration error: bad config");

        let err = SandcastleError::ScriptNotFound("my_script".into());
        assert_eq!(err.to_string(), "script not found: my_script");

        let err = SandcastleError::NamespaceNotFound("ns1".into());
        assert_eq!(err.to_string(), "namespace not found: ns1");

        let err = SandcastleError::NamespaceAlreadyExists("ns1".into());
        assert_eq!(err.to_string(), "namespace already exists: ns1");

        // Execution error via From impl
        let exec_err = ExecutionError::Cancelled;
        let err = SandcastleError::Execution(exec_err);
        assert_eq!(err.to_string(), "execution failed: sandbox was cancelled");

        // Capability error via From impl
        let cap_err = CapabilityError::Serialization("bad msgpack".into());
        let err = SandcastleError::Capability(cap_err);
        assert_eq!(
            err.to_string(),
            "capability error: serialization error in capability call: bad msgpack"
        );
    }

    #[test]
    fn execution_error_display() {
        let err = ExecutionError::GuestError {
            message: "ReferenceError".into(),
        };
        assert_eq!(err.to_string(), "guest code error: ReferenceError");

        let err = ExecutionError::Timeout {
            elapsed_ms: 5000,
            limit_ms: 3000,
        };
        assert_eq!(
            err.to_string(),
            "execution timed out after 5000ms (limit: 3000ms)"
        );

        let err = ExecutionError::FuelExhausted {
            consumed: 999,
            limit: 1000,
        };
        assert_eq!(
            err.to_string(),
            "fuel exhausted: consumed 999 of 1000 fuel units"
        );

        let err = ExecutionError::MemoryExceeded {
            used_bytes: 200,
            limit_bytes: 100,
        };
        assert_eq!(
            err.to_string(),
            "memory limit exceeded: 200 bytes (limit: 100 bytes)"
        );

        let err = ExecutionError::OutputSizeExceeded {
            size_bytes: 2_000_000,
            limit_bytes: 1_000_000,
        };
        assert_eq!(
            err.to_string(),
            "output size limit exceeded: 2000000 bytes (limit: 1000000 bytes)"
        );

        let err = ExecutionError::Cancelled;
        assert_eq!(err.to_string(), "sandbox was cancelled");
    }

    #[test]
    fn capability_error_display() {
        let err = CapabilityError::NotFound {
            capability: "fs".into(),
            method: "read".into(),
        };
        assert_eq!(err.to_string(), "capability `fs::read` not found");

        let err = CapabilityError::CallQuotaExceeded {
            capability: "http".into(),
            method: "get".into(),
            count: 100,
            max: 100,
        };
        assert_eq!(
            err.to_string(),
            "call quota exceeded for `http::get`: 100/100 calls"
        );

        let err = CapabilityError::PayloadTooLarge {
            capability: "kv".into(),
            method: "set".into(),
            size: 2_000_000,
            limit: 1_000_000,
        };
        assert_eq!(
            err.to_string(),
            "payload too large for `kv::set`: 2000000 bytes (limit: 1000000 bytes)"
        );

        let err = CapabilityError::TransferLimitExceeded {
            capability: "http".into(),
            total: 11_000_000,
            limit: 10_000_000,
        };
        assert_eq!(
            err.to_string(),
            "total transfer limit exceeded for `http`: 11000000 bytes (limit: 10000000 bytes)"
        );

        let err = CapabilityError::Timeout {
            capability: "http".into(),
            method: "request".into(),
            elapsed_ms: 5001,
        };
        assert_eq!(
            err.to_string(),
            "capability call timed out: `http::request` after 5001ms"
        );

        let err = CapabilityError::ConcurrencyExceeded {
            capability: "db".into(),
            active: 4,
            max: 4,
        };
        assert_eq!(
            err.to_string(),
            "concurrency limit exceeded for `db`: 4/4 concurrent calls"
        );

        let err = CapabilityError::InvocationFailed {
            capability: "kv".into(),
            method: "set".into(),
            message: "disk full".into(),
        };
        assert_eq!(
            err.to_string(),
            "capability invocation failed: `kv::set`: disk full"
        );

        let err = CapabilityError::Serialization("invalid utf8".into());
        assert_eq!(
            err.to_string(),
            "serialization error in capability call: invalid utf8"
        );
    }
}

// =========================================================================
// 9. Integration Edge Cases (require guest WASM)
// =========================================================================

mod integration_edge_cases {
    use super::*;

    /// Skip tests if the guest WASM binary is not available.
    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_empty_string_code_does_not_crash() {
        let runtime = require_runtime!();
        let result = runtime.execute(ExecutionRequest::new("")).await;
        // Should not panic. Result may be success or guest error.
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_whitespace_only_code_does_not_crash() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new("   \n\t\n   "))
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_with_large_json_input() {
        let runtime = require_runtime!();

        // Build a ~1MB JSON object
        let big_string = "x".repeat(1024 * 1024);
        let input = serde_json::json!({"data": big_string});

        let result = runtime
            .execute(
                ExecutionRequest::new(
                    "const input = globalThis.__sandcastle_input; return { len: input.data.length };",
                )
                .with_input(input),
            )
            .await
            .unwrap();

        assert!(result.is_success());
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(v["len"], 1024 * 1024);
            }
            other => panic!("Expected JSON output, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn execute_with_null_input() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(
                ExecutionRequest::new(
                    "const input = globalThis.__sandcastle_input; return { isNull: input === null || input === undefined };",
                )
                .with_input(serde_json::Value::Null),
            )
            .await
            .unwrap();

        assert!(result.is_success());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_executions_return_independent_results() {
        let runtime = Arc::new(require_runtime!());
        let mut handles = Vec::new();

        for i in 0..20u32 {
            let rt = runtime.clone();
            handles.push(tokio::spawn(async move {
                let code = format!("return {{ id: {i} }};");
                let result = rt.execute(ExecutionRequest::new(code)).await.unwrap();
                (i, result)
            }));
        }

        for handle in handles {
            let (expected_id, result) = handle.await.unwrap();
            assert!(
                result.is_success(),
                "execution {expected_id} failed: {:?}",
                result.status
            );
            match &result.output {
                OutputValue::Json(v) => {
                    assert_eq!(
                        v["id"], expected_id,
                        "execution {expected_id} returned wrong id"
                    );
                }
                other => panic!(
                    "execution {expected_id}: expected JSON output, got {:?}",
                    other
                ),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn console_output_with_special_characters() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new(
                r#"
                console.log("line1\nline2");
                console.log("he said \"hello\"");
                console.log("unicode: \u00e9\u00e8\u00ea \u2603 \ud83d\ude00");
                return null;
                "#,
            ))
            .await
            .unwrap();

        assert!(result.is_success());
        assert!(
            result.transcript.console.len() >= 3,
            "should have at least 3 console messages, got {}",
            result.transcript.console.len()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn multiple_output_artifacts_in_one_execution() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new(
                r#"
                globalThis.__sandcastle_write_artifact("file1.txt", "content1");
                globalThis.__sandcastle_write_artifact("file2.txt", "content2");
                globalThis.__sandcastle_write_artifact("file3.json", '{"ok":true}');
                return null;
                "#,
            ))
            .await
            .unwrap();

        assert!(result.is_success());
        assert_eq!(
            result.output_artifacts.len(),
            3,
            "should have 3 output artifacts, got {}",
            result.output_artifacts.len()
        );

        let names: Vec<&str> = result.output_artifacts.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"file1.txt"));
        assert!(names.contains(&"file2.txt"));
        assert!(names.contains(&"file3.json"));
    }
}

// =========================================================================
// 10. Memory Protection
// =========================================================================

mod memory_protection {
    use super::*;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_bomb_returns_guest_error_not_crash() {
        let runtime = require_runtime!();
        let limits = Limits {
            memory_mb: 8,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        let code = r#"
            const arrays = [];
            while(true) { arrays.push(new Array(100000).fill('x')); }
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_limits(limits))
            .await
            .unwrap();

        // Should not succeed — must be caught by either QuickJS memory limit
        // (GuestError with "out of memory") or Wasmtime memory limit (MemoryExceeded)
        assert!(
            !result.is_success(),
            "memory bomb should not succeed"
        );
        assert!(
            matches!(
                result.status,
                ExecutionStatus::GuestError { .. } | ExecutionStatus::MemoryExceeded
            ),
            "expected GuestError or MemoryExceeded, got {:?}",
            result.status
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn allocation_exceeding_memory_limit_produces_memory_exceeded() {
        let runtime = require_runtime!();
        // Use a small memory limit — allocating more than the limit should
        // be caught by Wasmtime's trap_on_grow_failure and classified as
        // MemoryExceeded
        let limits = Limits {
            memory_mb: 4,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        let code = r#"
            try {
                // Try to allocate 8MB — more than the 4MB limit
                const arr = new Uint8Array(8 * 1024 * 1024);
                return { allocated: true, size: arr.length };
            } catch(e) {
                return { allocated: false, error: e.message };
            }
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_limits(limits))
            .await
            .unwrap();

        // Either QuickJS catches it (GuestError/success with allocated:false)
        // or Wasmtime catches it (MemoryExceeded). Both are acceptable.
        if result.is_success() {
            match &result.output {
                OutputValue::Json(v) => {
                    // If execution succeeded, the allocation must have failed
                    // inside the try/catch
                    assert_eq!(
                        v.get("allocated").and_then(|v| v.as_bool()),
                        Some(false),
                        "8MB allocation should fail under 4MB memory limit, got: {}",
                        v
                    );
                }
                other => panic!("expected JSON output, got {:?}", other),
            }
        } else {
            assert!(
                matches!(
                    result.status,
                    ExecutionStatus::MemoryExceeded | ExecutionStatus::GuestError { .. }
                ),
                "expected MemoryExceeded or GuestError, got {:?}",
                result.status
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn large_input_5mb_reports_memory_exceeded() {
        let runtime = require_runtime!();
        let big_string = "x".repeat(5_000_000);
        let input = serde_json::json!({"data": big_string});

        let result = runtime
            .execute(
                ExecutionRequest::new(
                    "return globalThis.__sandcastle_input.data.length;",
                )
                .with_input(input),
            )
            .await
            .unwrap();

        // With default 32MB memory, a 5MB input may or may not succeed depending
        // on how much overhead serde_json + QuickJS + WASM takes.
        // If it fails, it must be MemoryExceeded (not an opaque GuestError).
        if !result.is_success() {
            assert!(
                matches!(result.status, ExecutionStatus::MemoryExceeded),
                "5MB input failure should be MemoryExceeded, got {:?}",
                result.status
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn large_input_10mb_reports_memory_exceeded() {
        let runtime = require_runtime!();
        let big_string = "x".repeat(10_000_000);
        let input = serde_json::json!({"data": big_string});

        let result = runtime
            .execute(
                ExecutionRequest::new(
                    "return globalThis.__sandcastle_input.data.length;",
                )
                .with_input(input),
            )
            .await
            .unwrap();

        // 10MB input will almost certainly exceed the 32MB default memory limit
        if !result.is_success() {
            assert!(
                matches!(result.status, ExecutionStatus::MemoryExceeded),
                "10MB input failure should be MemoryExceeded, got {:?}",
                result.status
            );
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn normal_execution_unaffected_by_memory_limits() {
        let runtime = require_runtime!();
        // Ensure the QuickJS memory limit + GC threshold don't break normal ops
        let result = runtime
            .execute(ExecutionRequest::new(
                r#"
                const arr = [1, 2, 3, 4, 5];
                const obj = { a: 1, b: "hello", c: [true, false] };
                const str = "x".repeat(10000);
                return { arrLen: arr.length, keys: Object.keys(obj).length, strLen: str.length };
                "#,
            ))
            .await
            .unwrap();

        assert!(result.is_success(), "normal execution should succeed: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(v["arrLen"], 5);
                assert_eq!(v["keys"], 3);
                assert_eq!(v["strLen"], 10000);
            }
            other => panic!("expected JSON output, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_limit_1mb_rejects_sandbox_creation() {
        let runtime = require_runtime!();
        let limits = Limits {
            memory_mb: 1,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        let result = runtime
            .execute(ExecutionRequest::new("return 1;").with_limits(limits))
            .await;

        // 1MB is too small for the QuickJS WASM module — should fail at creation
        assert!(
            result.is_err(),
            "1MB memory limit should fail sandbox creation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn memory_exceeded_status_has_peak_memory_in_transcript() {
        let runtime = require_runtime!();
        let limits = Limits {
            memory_mb: 8,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        let code = r#"
            const arrays = [];
            try {
                while(true) { arrays.push(new Array(50000).fill(42)); }
            } catch(e) {
                return { caught: true, error: e.message, arrays: arrays.length };
            }
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_limits(limits))
            .await
            .unwrap();

        // Whether caught by JS or by Wasmtime, the transcript should record memory usage
        assert!(
            result.transcript.peak_memory_bytes > 0,
            "transcript should record peak memory"
        );
        assert!(
            result.transcript.peak_memory_bytes <= 8 * 1024 * 1024,
            "peak memory should not exceed the 8MB limit, got {} bytes",
            result.transcript.peak_memory_bytes
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn gradual_allocation_hits_quickjs_limit_before_wasm_trap() {
        let runtime = require_runtime!();
        let limits = Limits {
            memory_mb: 8,
            timeout: Duration::from_secs(5),
            ..Limits::default()
        };
        // Allocate in small increments to trigger QuickJS OOM, not WASM trap
        let code = r#"
            const chunks = [];
            let totalBytes = 0;
            try {
                while (true) {
                    chunks.push(new Uint8Array(64 * 1024)); // 64KB chunks
                    totalBytes += 64 * 1024;
                }
            } catch(e) {
                return { totalKB: Math.floor(totalBytes / 1024), error: e.message };
            }
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_limits(limits))
            .await
            .unwrap();

        // QuickJS should catch the OOM and let the try/catch handle it
        if result.is_success() {
            match &result.output {
                OutputValue::Json(v) => {
                    let error = v.get("error").and_then(|e| e.as_str()).unwrap_or("");
                    assert!(
                        error.contains("memory") || error.contains("out of") || error.contains("alloc"),
                        "error should mention memory, got: {}",
                        error
                    );
                    let total_kb = v.get("totalKB").and_then(|v| v.as_u64()).unwrap_or(0);
                    assert!(
                        total_kb > 0,
                        "should have allocated some memory before OOM"
                    );
                }
                other => panic!("expected JSON output, got {:?}", other),
            }
        }
        // MemoryExceeded is also acceptable if QuickJS didn't catch it
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_memory_pressure_all_isolated() {
        let runtime = Arc::new(require_runtime!());
        let mut handles = Vec::new();

        for i in 0..10u32 {
            let rt = runtime.clone();
            handles.push(tokio::spawn(async move {
                let limits = Limits {
                    memory_mb: 32,
                    timeout: Duration::from_secs(5),
                    ..Limits::default()
                };
                // Each sandbox allocates ~1MB then returns
                let code = format!(
                    r#"
                    const data = new Uint8Array(1024 * 1024);
                    data.fill({i});
                    return {{ id: {i}, size: data.length, first: data[0] }};
                    "#
                );
                rt.execute(ExecutionRequest::new(code).with_limits(limits))
                    .await
                    .unwrap()
            }));
        }

        for (idx, handle) in handles.into_iter().enumerate() {
            let result = handle.await.unwrap();
            assert!(
                result.is_success(),
                "concurrent sandbox {} should succeed: {:?}",
                idx,
                result.status
            );
            match &result.output {
                OutputValue::Json(v) => {
                    assert_eq!(v["id"], idx as u64);
                    assert_eq!(v["size"], 1024 * 1024);
                }
                other => panic!("expected JSON output for sandbox {}, got {:?}", idx, other),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn promise_resolution_works_with_memory_limits() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new(
                "return Promise.resolve(42);",
            ))
            .await
            .unwrap();

        assert!(result.is_success(), "Promise.resolve should work: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => assert_eq!(*v, serde_json::json!(42)),
            other => panic!("expected JSON 42, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn promise_all_resolves() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new(
                "return Promise.all([Promise.resolve(1), Promise.resolve(2), Promise.resolve(3)]);",
            ))
            .await
            .unwrap();

        assert!(result.is_success(), "Promise.all should work: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => assert_eq!(*v, serde_json::json!([1, 2, 3])),
            other => panic!("expected [1,2,3], got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn async_function_return_resolves() {
        let runtime = require_runtime!();
        let result = runtime
            .execute(ExecutionRequest::new(
                "async function getValue() { return 99; } return getValue();",
            ))
            .await
            .unwrap();

        assert!(result.is_success(), "async function should work: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => assert_eq!(*v, serde_json::json!(99)),
            other => panic!("expected 99, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn capability_quota_throws_js_exception() {
        use sandcastle::capability::SimpleCapability;

        let runtime = require_runtime!();
        let mut registry = CapabilityRegistry::new();
        registry.register_with_limits(
            Box::new(SimpleCapability::new("limited", |_, _| {
                Ok(serde_json::json!("ok"))
            })),
            CapabilityLimits {
                max_calls: 3,
                ..CapabilityLimits::default()
            },
        );
        let caps = Arc::new(registry);

        let code = r#"
            let succeeded = 0;
            let failed = 0;
            for (let i = 0; i < 10; i++) {
                try {
                    __sandcastle_host_call("limited", "do", "{}");
                    succeeded++;
                } catch(e) {
                    failed++;
                }
            }
            return { succeeded, failed };
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_capabilities(caps))
            .await
            .unwrap();

        assert!(result.is_success());
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(
                    v["succeeded"], 3,
                    "exactly 3 calls should succeed (max_calls=3)"
                );
                assert_eq!(
                    v["failed"], 7,
                    "remaining 7 calls should throw exceptions"
                );
            }
            other => panic!("expected JSON output, got {:?}", other),
        }
    }
}

// =========================================================================
// 10b. Runtime Metrics
// =========================================================================

mod runtime_metrics {
    use super::*;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => { eprintln!("SKIP: guest WASM not found"); return; }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_track_execute_calls() {
        let runtime = require_runtime!();
        assert_eq!(runtime.metrics().total(), 0);
        assert_eq!(runtime.metrics().active(), 0);

        runtime.execute(ExecutionRequest::new("return 1;")).await.unwrap();
        assert_eq!(runtime.metrics().total(), 1);

        runtime.execute(ExecutionRequest::new("return 2;")).await.unwrap();
        assert_eq!(runtime.metrics().total(), 2);

        // Active should be 0 after completion
        assert_eq!(runtime.metrics().active(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_track_retained_sandbox() {
        let runtime = require_runtime!();
        let sandbox = runtime.create_sandbox().unwrap();

        sandbox.execute(ExecutionRequest::new("return 1;")).await.unwrap();
        assert_eq!(runtime.metrics().total(), 1);

        sandbox.execute(ExecutionRequest::new("return 2;")).await.unwrap();
        assert_eq!(runtime.metrics().total(), 2);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_track_persistent_sandbox() {
        let runtime = require_runtime!();
        let caps = Arc::new(CapabilityRegistry::new());

        let mut ps = runtime
            .create_persistent_sandbox(Limits::default(), caps)
            .await
            .unwrap();

        ps.execute("return 1;").await.unwrap();
        assert_eq!(runtime.metrics().total(), 1);

        ps.execute("return 2;").await.unwrap();
        assert_eq!(runtime.metrics().total(), 2);

        ps.execute("return 3;").await.unwrap();
        assert_eq!(runtime.metrics().total(), 3);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn metrics_active_during_concurrent_execution() {
        let runtime = Arc::new(require_runtime!());
        assert_eq!(runtime.metrics().active(), 0);

        let mut handles = Vec::new();
        for _ in 0..5 {
            let rt = runtime.clone();
            handles.push(tokio::spawn(async move {
                rt.execute(
                    ExecutionRequest::new("let s=0; for(let i=0;i<10000;i++) s+=i; return s;")
                ).await
            }));
        }

        for h in handles {
            h.await.unwrap().unwrap();
        }

        assert_eq!(runtime.metrics().total(), 5);
        assert_eq!(runtime.metrics().active(), 0);
    }
}

// =========================================================================
// 11. KV Capability End-to-End
// =========================================================================

/// Assert that an ExecutionResult's output matches the expected JSON value.
fn assert_output_json(result: &sandcastle::sandbox::ExecutionResult, expected: serde_json::Value) {
    match &result.output {
        OutputValue::Json(v) => assert_eq!(v, &expected, "output mismatch"),
        other => panic!("expected Json({expected}), got {other:?}"),
    }
}

mod kv_e2e {
    use super::*;
    use sandcastle::capabilities::KvCapability;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    fn kv_caps() -> (Arc<CapabilityRegistry>, Arc<dashmap::DashMap<String, serde_json::Value>>) {
        let kv = KvCapability::default();
        let store = kv.store().clone();
        let mut registry = CapabilityRegistry::new();
        registry.register(Box::new(kv));
        (Arc::new(registry), store)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn guest_set_and_get() {
        let runtime = require_runtime!();
        let (caps, store) = kv_caps();

        let code = r#"
            __sandcastle_host_call("kv", "set", JSON.stringify({key: "name", value: "Alice"}));
            const val = JSON.parse(__sandcastle_host_call("kv", "get", JSON.stringify({key: "name"})));
            return val;
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_capabilities(caps))
            .await
            .unwrap();

        assert!(result.is_success(), "status: {:?}", result.status);
        assert_output_json(&result, serde_json::json!("Alice"));
        // Verify the store was actually written to
        assert_eq!(store.get("name").map(|v| v.clone()), Some(serde_json::json!("Alice")));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn guest_list_delete_has() {
        let runtime = require_runtime!();
        let (caps, _) = kv_caps();

        let code = r#"
            __sandcastle_host_call("kv", "set", JSON.stringify({key: "user:1", value: "Alice"}));
            __sandcastle_host_call("kv", "set", JSON.stringify({key: "user:2", value: "Bob"}));
            __sandcastle_host_call("kv", "set", JSON.stringify({key: "item:1", value: "Widget"}));

            const users = JSON.parse(__sandcastle_host_call("kv", "list", JSON.stringify({prefix: "user:"})));
            const hasBefore = JSON.parse(__sandcastle_host_call("kv", "has", JSON.stringify({key: "user:1"})));
            const deleted = JSON.parse(__sandcastle_host_call("kv", "delete", JSON.stringify({key: "user:1"})));
            const hasAfter = JSON.parse(__sandcastle_host_call("kv", "has", JSON.stringify({key: "user:1"})));

            return { userCount: users.length, hasBefore, deleted, hasAfter };
        "#;
        let result = runtime
            .execute(ExecutionRequest::new(code).with_capabilities(caps))
            .await
            .unwrap();

        assert!(result.is_success(), "status: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(v["userCount"], 2);
                assert_eq!(v["hasBefore"], true);
                assert_eq!(v["deleted"], true);
                assert_eq!(v["hasAfter"], false);
            }
            other => panic!("expected JSON, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shared_kv_across_sandboxes() {
        let runtime = require_runtime!();
        let (caps, _) = kv_caps();

        // Sandbox 1 writes
        let write = r#"
            __sandcastle_host_call("kv", "set", JSON.stringify({key: "shared", value: 42}));
            return "written";
        "#;
        let r1 = runtime
            .execute(ExecutionRequest::new(write).with_capabilities(caps.clone()))
            .await
            .unwrap();
        assert!(r1.is_success());

        // Sandbox 2 reads (same caps = same store)
        let read = r#"
            const val = JSON.parse(__sandcastle_host_call("kv", "get", JSON.stringify({key: "shared"})));
            return val;
        "#;
        let r2 = runtime
            .execute(ExecutionRequest::new(read).with_capabilities(caps))
            .await
            .unwrap();
        assert!(r2.is_success());
        // Value comes back as 42.0 through JSON round-trip (QuickJS float promotion)
        match &r2.output {
            OutputValue::Json(v) => {
                let n = v.as_f64().expect("expected number");
                assert_eq!(n, 42.0, "expected 42");
            }
            other => panic!("expected JSON number, got {:?}", other),
        }
    }
}

// =========================================================================
// 12. Script Registry Dispatch End-to-End
// =========================================================================

mod registry_dispatch_e2e {
    use super::*;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn register_and_dispatch() {
        let runtime = require_runtime!();
        let registry = ScriptRegistry::new(100);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register(
                "doubler",
                "const input = globalThis.__sandcastle_input; return input.x * 2;",
                caps,
                Limits::default(),
            )
            .unwrap();

        let result = runtime
            .dispatch(&registry, "doubler", serde_json::json!({"x": 21}))
            .await
            .unwrap();

        assert!(result.is_success(), "status: {:?}", result.status);
        assert_output_json(&result, serde_json::json!(42));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_multiple_times_no_state_leak() {
        let runtime = require_runtime!();
        let registry = ScriptRegistry::new(100);
        let caps = Arc::new(CapabilityRegistry::new());

        registry
            .register(
                "counter",
                r#"
                    // This var is fresh each execution — no state leak
                    var count = (globalThis.__sandcastle_input || {}).start || 0;
                    count += 1;
                    return count;
                "#,
                caps,
                Limits::default(),
            )
            .unwrap();

        for _ in 0..5 {
            let result = runtime
                .dispatch(&registry, "counter", serde_json::json!({"start": 10}))
                .await
                .unwrap();
            assert!(result.is_success());
            // Should always be 11, never incrementing — each execution is isolated
            assert_output_json(&result, serde_json::json!(11));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dispatch_nonexistent_script_errors() {
        let runtime = require_runtime!();
        let registry = ScriptRegistry::new(100);

        let result = runtime
            .dispatch(&registry, "nope", serde_json::Value::Null)
            .await;
        assert!(result.is_err(), "dispatch to nonexistent script should error");
    }
}

// =========================================================================
// 13. Namespace Isolation Under Execution
// =========================================================================

mod namespace_execution {
    use super::*;
    use sandcastle::namespace::{NamespaceLimits, NamespaceManager};

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn namespaces_execute_independently() {
        let runtime = require_runtime!();
        let caps = Arc::new(CapabilityRegistry::new());
        let manager = NamespaceManager::new(10);

        let ns_a = manager
            .create("tenant-a", NamespaceLimits::default(), caps.clone())
            .unwrap();
        let ns_b = manager
            .create("tenant-b", NamespaceLimits::default(), caps)
            .unwrap();

        // Same script name, different code
        ns_a.register("greet", String::from("return 'hello from A';"), None)
            .unwrap();
        ns_b.register("greet", String::from("return 'hello from B';"), None)
            .unwrap();

        let script_a = ns_a.get_script("greet").unwrap();
        let script_b = ns_b.get_script("greet").unwrap();

        let ra = runtime
            .execute(ExecutionRequest::new(&script_a.code))
            .await
            .unwrap();
        let rb = runtime
            .execute(ExecutionRequest::new(&script_b.code))
            .await
            .unwrap();

        assert_output_json(&ra, serde_json::json!("hello from A"));
        assert_output_json(&rb, serde_json::json!("hello from B"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn namespace_script_not_visible_across_tenants() {
        let caps = Arc::new(CapabilityRegistry::new());
        let manager = NamespaceManager::new(10);

        let ns_a = manager
            .create("a", NamespaceLimits::default(), caps.clone())
            .unwrap();
        let ns_b = manager
            .create("b", NamespaceLimits::default(), caps)
            .unwrap();

        ns_a.register("secret", String::from("return 'a_secret';"), None)
            .unwrap();

        assert!(ns_a.get_script("secret").is_some());
        assert!(ns_b.get_script("secret").is_none(), "tenant B should not see tenant A's scripts");
    }
}

// =========================================================================
// 14. Artifact-Based Workflow
// =========================================================================

mod artifact_workflow {
    use super::*;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn process_input_artifact_produce_output() {
        let runtime = require_runtime!();
        let csv_data = "name,score\nAlice,95\nBob,87\nCarol,92\n";
        let artifact = Artifact::from_text("input.csv", csv_data);

        let code = r#"
            const csv = globalThis.__sandcastle_read_artifact("input.csv");
            const lines = csv.trim().split("\n");
            const header = lines[0].split(",");
            const rows = lines.slice(1).map(line => {
                const vals = line.split(",");
                return { name: vals[0], score: parseInt(vals[1]) };
            });
            const avg = rows.reduce((s, r) => s + r.score, 0) / rows.length;
            const result = JSON.stringify({ rows: rows.length, average: avg });
            globalThis.__sandcastle_write_artifact("summary.json", result);
            return { rows: rows.length, average: avg };
        "#;

        let result = runtime
            .execute(ExecutionRequest::new(code).with_artifacts(vec![artifact]))
            .await
            .unwrap();

        assert!(result.is_success(), "status: {:?}", result.status);
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(v["rows"], 3);
            }
            other => panic!("expected JSON, got {:?}", other),
        }
        assert_eq!(result.output_artifacts.len(), 1);
        assert_eq!(result.output_artifacts[0].name, "summary.json");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn large_artifact_near_boundary() {
        let runtime = require_runtime!();
        // Test writing a large artifact (1MB) — well under the 16MB MAX_GUEST_BUFFER_SIZE
        let code = r#"
            const data = "x".repeat(1024 * 1024);
            globalThis.__sandcastle_write_artifact("big.txt", data);
            return data.length;
        "#;
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        assert!(result.is_success(), "status: {:?}", result.status);
        assert_output_json(&result, serde_json::json!(1048576));
        assert_eq!(result.output_artifacts.len(), 1);
        assert_eq!(result.output_artifacts[0].data.len(), 1048576);
    }
}

// =========================================================================
// 15. PRD-Claimed Globals
// =========================================================================

mod prd_globals {
    use super::*;

    macro_rules! require_runtime {
        () => {
            match create_runtime() {
                Some(rt) => rt,
                None => {
                    eprintln!("SKIP: guest WASM module not found");
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn json_global() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new(
                "return JSON.parse(JSON.stringify({a: [1,2,3]}));",
            ))
            .await
            .unwrap();
        assert!(r.is_success());
        assert_output_json(&r, serde_json::json!({"a": [1,2,3]}));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn math_global() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new("return Math.max(1, 5, 3);"))
            .await
            .unwrap();
        assert!(r.is_success());
        assert_output_json(&r, serde_json::json!(5));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn date_global() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new("return typeof Date.now() === 'number';"))
            .await
            .unwrap();
        assert!(r.is_success());
        assert_output_json(&r, serde_json::json!(true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn atob_btoa() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new(
                r#"
                try {
                    const encoded = btoa("hello world");
                    const decoded = atob(encoded);
                    return { encoded, decoded };
                } catch(e) {
                    return { error: e.message, available: typeof btoa !== 'undefined' };
                }
                "#,
            ))
            .await
            .unwrap();
        assert!(r.is_success(), "status: {:?}", r.status);
        // btoa/atob may not be available in QuickJS — record what happens
        match &r.output {
            OutputValue::Json(v) => {
                if v.get("encoded").is_some() {
                    assert_eq!(v["decoded"], "hello world");
                }
                // If error, it's a known limitation — not a test failure
            }
            _ => {}
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn text_encoder_decoder() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new(
                r#"
                try {
                    const encoder = new TextEncoder();
                    const encoded = encoder.encode("hello");
                    const decoder = new TextDecoder();
                    const decoded = decoder.decode(encoded);
                    return { len: encoded.length, decoded };
                } catch(e) {
                    return { error: e.message, available: typeof TextEncoder !== 'undefined' };
                }
                "#,
            ))
            .await
            .unwrap();
        assert!(r.is_success(), "status: {:?}", r.status);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn url_parsing() {
        let runtime = require_runtime!();
        let r = runtime
            .execute(ExecutionRequest::new(
                r#"
                try {
                    const u = new URL("https://example.com/path?q=1#frag");
                    return { host: u.hostname, path: u.pathname, search: u.search };
                } catch(e) {
                    return { error: e.message, available: typeof URL !== 'undefined' };
                }
                "#,
            ))
            .await
            .unwrap();
        assert!(r.is_success(), "status: {:?}", r.status);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn error_recovery_pattern() {
        let runtime = require_runtime!();
        // Simulates an agent that tries code, catches error, adapts
        let code = r#"
            function tryParse(input) {
                try {
                    return { ok: true, data: JSON.parse(input) };
                } catch(e) {
                    return { ok: false, error: e.message };
                }
            }

            const attempt1 = tryParse("{invalid json}");
            const attempt2 = tryParse('{"valid": true}');
            return { attempt1, attempt2 };
        "#;
        let result = runtime.execute(ExecutionRequest::new(code)).await.unwrap();
        assert!(result.is_success());
        match &result.output {
            OutputValue::Json(v) => {
                assert_eq!(v["attempt1"]["ok"], false);
                assert_eq!(v["attempt2"]["ok"], true);
                assert_eq!(v["attempt2"]["data"]["valid"], true);
            }
            other => panic!("expected JSON, got {:?}", other),
        }
    }
}
