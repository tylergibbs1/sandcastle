use std::sync::Arc;

use sandcastle::capability::{CapabilityRegistry, SimpleCapability};
use sandcastle::limits::Limits;
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::types::*;

fn load_guest_module() -> Vec<u8> {
    // CARGO_MANIFEST_DIR points to crates/sandcastle, go up two levels to workspace root
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
            return bytes;
        }
    }
    panic!(
        "Guest WASM module not found. Build with: cd guest && ./build.sh\nTried: {:?}",
        candidates
    );
}

fn create_runtime() -> SandCastle {
    let guest_module = load_guest_module();
    SandCastle::new(Config::new(guest_module)).expect("Failed to create runtime")
}

#[tokio::test(flavor = "multi_thread")]
async fn test_simple_expression() {
    let runtime = create_runtime();
    let result = runtime
        .execute(ExecutionRequest::new("return 1 + 1;"))
        .await
        .unwrap();

    assert!(result.is_success());
    match &result.output {
        OutputValue::Json(v) => assert_eq!(v, &serde_json::json!(2)),
        other => panic!("Expected JSON output, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_json_input() {
    let runtime = create_runtime();
    let input = serde_json::json!({"name": "Alice", "age": 30});
    let result = runtime
        .execute(
            ExecutionRequest::new(
                "const input = globalThis.__sandcastle_input; return { greeting: 'Hello ' + input.name };"
            )
            .with_input(input),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    match &result.output {
        OutputValue::Json(v) => {
            assert_eq!(v["greeting"], "Hello Alice");
        }
        other => panic!("Expected JSON output, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_console_output() {
    let runtime = create_runtime();
    let result = runtime
        .execute(ExecutionRequest::new(
            r#"
            console.log("hello");
            console.warn("warning!");
            console.error("error!");
            return null;
            "#,
        ))
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(result.transcript.console.len() >= 3);
    assert_eq!(result.transcript.console[0].message, "hello");
    assert_eq!(result.transcript.console[0].level, ConsoleLevel::Log);
    assert_eq!(result.transcript.console[1].level, ConsoleLevel::Warn);
    assert_eq!(result.transcript.console[2].level, ConsoleLevel::Error);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_fuel_exhaustion() {
    let runtime = create_runtime();
    // QuickJS init takes ~50-100M fuel. Use enough to init but not enough for an infinite loop.
    let limits = Limits {
        fuel: 200_000_000,
        ..Limits::default()
    };

    let result = runtime
        .execute(
            ExecutionRequest::new("let x = 0; while(true) { x++; } return x;")
                .with_limits(limits),
        )
        .await
        .unwrap();

    // Should hit either fuel exhaustion or timeout (depending on how fast fuel is consumed)
    assert!(
        matches!(result.status, ExecutionStatus::FuelExhausted)
            || matches!(result.status, ExecutionStatus::Timeout)
            || matches!(result.status, ExecutionStatus::GuestError { .. }),
        "Expected fuel exhaustion or timeout, got {:?}",
        result.status
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn test_timeout() {
    let runtime = create_runtime();
    let limits = Limits {
        timeout: std::time::Duration::from_secs(2),
        fuel: 0, // Unlimited fuel to ensure timeout triggers
        ..Limits::default()
    };

    let start = std::time::Instant::now();
    let result = runtime
        .execute(
            ExecutionRequest::new("while(true) {} return null;").with_limits(limits),
        )
        .await
        .unwrap();

    let elapsed = start.elapsed();
    assert!(
        matches!(result.status, ExecutionStatus::Timeout),
        "Expected Timeout, got {:?}",
        result.status
    );
    // Should complete within a reasonable time after the timeout
    assert!(elapsed < std::time::Duration::from_secs(10));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_guest_error() {
    let runtime = create_runtime();
    let result = runtime
        .execute(ExecutionRequest::new("throw new Error('test error');"))
        .await
        .unwrap();

    assert!(matches!(result.status, ExecutionStatus::GuestError { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn test_transcript_generation() {
    let runtime = create_runtime();
    let result = runtime
        .execute(ExecutionRequest::new(
            r#"
            console.log("step 1");
            console.log("step 2");
            return { result: 42 };
            "#,
        ))
        .await
        .unwrap();

    assert!(result.is_success());
    let transcript = &result.transcript;
    assert!(!transcript.execution_id.is_empty());
    assert!(transcript.finished_at.is_some());
    assert!(transcript.console.len() >= 2);
    assert!(transcript.fuel_consumed > 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn test_input_artifacts() {
    let runtime = create_runtime();
    let artifact = Artifact::from_text("data.txt", "Hello, World!");

    let result = runtime
        .execute(
            ExecutionRequest::new(
                r#"
                const data = globalThis.__sandcastle_read_artifact("data.txt");
                return { content: data };
                "#,
            )
            .with_artifacts(vec![artifact]),
        )
        .await
        .unwrap();

    assert!(result.is_success());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_output_artifacts() {
    let runtime = create_runtime();

    let result = runtime
        .execute(ExecutionRequest::new(
            r#"
            globalThis.__sandcastle_write_artifact("report.json", JSON.stringify({ ok: true }));
            return null;
            "#,
        ))
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(!result.output_artifacts.is_empty());
    assert_eq!(result.output_artifacts[0].name, "report.json");
}

#[tokio::test(flavor = "multi_thread")]
async fn test_capability_call() {
    let runtime = create_runtime();

    let mut registry = CapabilityRegistry::new();
    registry.register(Box::new(SimpleCapability::new(
        "test_service",
        |method, input| {
            if method == "echo" {
                Ok(input)
            } else {
                Err(sandcastle::error::CapabilityError::NotFound {
                    capability: "test_service".into(),
                    method: method.to_string(),
                })
            }
        },
    )));

    let result = runtime
        .execute(
            ExecutionRequest::new(
                r#"
                const result = __sandcastle_host_call("test_service", "echo", '{"hello":"world"}');
                return JSON.parse(result);
                "#,
            )
            .with_capabilities(Arc::new(registry)),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    assert!(!result.transcript.capability_calls.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn test_data_transformation() {
    let runtime = create_runtime();
    let input = serde_json::json!({
        "numbers": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10]
    });

    let result = runtime
        .execute(
            ExecutionRequest::new(
                r#"
                const input = globalThis.__sandcastle_input;
                const evens = input.numbers.filter(n => n % 2 === 0);
                const sum = evens.reduce((a, b) => a + b, 0);
                return { evens: evens, sum: sum };
                "#,
            )
            .with_input(input),
        )
        .await
        .unwrap();

    assert!(result.is_success());
    match &result.output {
        OutputValue::Json(v) => {
            assert_eq!(v["sum"], 30);
            assert_eq!(v["evens"], serde_json::json!([2, 4, 6, 8, 10]));
        }
        other => panic!("Expected JSON output, got {:?}", other),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn test_concurrent_executions() {
    let runtime = Arc::new(create_runtime());
    let mut handles = Vec::new();

    for i in 0..10 {
        let rt = runtime.clone();
        handles.push(tokio::spawn(async move {
            let code = format!("return {{ id: {i} }};");
            rt.execute(ExecutionRequest::new(code)).await
        }));
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let result = handle.await.unwrap().unwrap();
        assert!(result.is_success());
        match &result.output {
            OutputValue::Json(v) => assert_eq!(v["id"], i as i64),
            other => panic!("Expected JSON output, got {:?}", other),
        }
    }
}
