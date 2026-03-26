//! End-to-end tests for the HTTP capability.
//!
//! Spins up a local Axum server and verifies that guest JS code can make
//! real HTTP requests through the sandboxed `__sandcastle_host_call("http", ...)`
//! bridge — including GET, POST, headers, domain allowlists, response size
//! caps, error handling, and JSON parsing.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::Query;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::Router;
use tokio::net::TcpListener;

use sandcastle::capabilities::HttpCapability;
use sandcastle::capability::CapabilityRegistry;
use sandcastle::limits::{CapabilityLimits, Limits};
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::types::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn try_load_guest_module() -> Option<Vec<u8>> {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?;
    let path = workspace_root.join("guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm");
    std::fs::read(path).ok()
}

fn create_runtime() -> Option<SandCastle> {
    let guest = try_load_guest_module()?;
    Some(SandCastle::new(Config::new(guest)).expect("Failed to create runtime"))
}

macro_rules! require_runtime {
    () => {
        match create_runtime() {
            Some(rt) => rt,
            None => { eprintln!("SKIP: guest WASM not found"); return; }
        }
    };
}

fn assert_json(result: &sandcastle::sandbox::ExecutionResult, check: impl FnOnce(&serde_json::Value)) {
    assert!(result.is_success(), "execution failed: {:?}", result.status);
    match &result.output {
        OutputValue::Json(v) => check(v),
        other => panic!("expected JSON, got {:?}", other),
    }
}

/// Start a local Axum server and return the base URL (e.g. "http://127.0.0.1:12345").
async fn start_server(app: Router) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://127.0.0.1:{}", addr.port())
}

fn http_caps(allowed_domains: Vec<String>) -> Arc<CapabilityRegistry> {
    let http = HttpCapability::new(allowed_domains, 10 * 1024 * 1024);
    let mut reg = CapabilityRegistry::new();
    reg.register(Box::new(http));
    Arc::new(reg)
}

fn http_caps_with_size_limit(allowed_domains: Vec<String>, max_bytes: usize) -> Arc<CapabilityRegistry> {
    let http = HttpCapability::new(allowed_domains, max_bytes);
    let mut reg = CapabilityRegistry::new();
    reg.register(Box::new(http));
    Arc::new(reg)
}

// ---------------------------------------------------------------------------
// Test server routes
// ---------------------------------------------------------------------------

fn test_app() -> Router {
    Router::new()
        .route("/hello", get(|| async { "Hello, SandCastle!" }))
        .route("/json", get(|| async {
            axum::Json(serde_json::json!({
                "name": "test",
                "version": 1,
                "items": [1, 2, 3]
            }))
        }))
        .route("/echo", post(|body: String| async move { body }))
        .route("/echo-headers", get(|headers: HeaderMap| async move {
            let mut map = serde_json::Map::new();
            for (k, v) in headers.iter() {
                map.insert(
                    k.as_str().to_string(),
                    serde_json::Value::String(v.to_str().unwrap_or("").to_string()),
                );
            }
            axum::Json(serde_json::Value::Object(map))
        }))
        .route("/status/{code}", get(|axum::extract::Path(code): axum::extract::Path<u16>| async move {
            (StatusCode::from_u16(code).unwrap_or(StatusCode::OK), format!("status {}", code))
        }))
        .route("/large", get(|| async {
            // Return 100KB of data
            "x".repeat(100 * 1024)
        }))
        .route("/delay", get(|Query(params): Query<std::collections::HashMap<String, String>>| async move {
            let ms: u64 = params.get("ms").and_then(|s| s.parse().ok()).unwrap_or(100);
            tokio::time::sleep(Duration::from_millis(ms)).await;
            format!("waited {}ms", ms)
        }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_get_text() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/hello" }})));
        return {{ status: resp.status, body: resp.body }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 200);
        assert_eq!(v["body"], "Hello, SandCastle!");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_get_json_and_parse() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/json" }})));
        const data = JSON.parse(resp.body);
        return {{ status: resp.status, name: data.name, items: data.items }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 200);
        assert_eq!(v["name"], "test");
        assert_eq!(v["items"], serde_json::json!([1, 2, 3]));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_post_with_body() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{
                method: "POST",
                url: "{base}/echo",
                body: "hello from sandbox"
            }})));
        return {{ status: resp.status, body: resp.body }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 200);
        assert_eq!(v["body"], "hello from sandbox");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_custom_headers() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{
                method: "GET",
                url: "{base}/echo-headers",
                headers: {{ "x-custom": "sandcastle-test", "x-request-id": "abc123" }}
            }})));
        const headers = JSON.parse(resp.body);
        return {{ custom: headers["x-custom"], requestId: headers["x-request-id"] }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["custom"], "sandcastle-test");
        assert_eq!(v["requestId"], "abc123");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_handles_404() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/nonexistent" }})));
        return {{ status: resp.status, is404: resp.status === 404 }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 404);
        assert_eq!(v["is404"], true);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_handles_500() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/status/500" }})));
        return {{ status: resp.status, isError: resp.status >= 500 }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 500);
        assert_eq!(v["isError"], true);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn domain_allowlist_blocks_disallowed_host() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    // Only allow "example.com" — 127.0.0.1 should be blocked
    let caps = http_caps(vec!["example.com".to_string()]);

    let code = format!(r#"
        try {{
            const resp = __sandcastle_host_call("http", "request",
                JSON.stringify({{ method: "GET", url: "{base}/hello" }}));
            return {{ allowed: true }};
        }} catch(e) {{
            return {{ allowed: false, error: String(e) }};
        }}
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["allowed"], false);
        let error = v["error"].as_str().unwrap_or("");
        assert!(error.contains("domain") || error.contains("allow"), "error should mention domain: {}", error);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn domain_allowlist_permits_allowed_host() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/hello" }})));
        return resp.status;
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v.as_f64().unwrap(), 200.0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_allowlist_permits_all() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec![]); // empty = allow all

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/hello" }})));
        return resp.status;
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v.as_f64().unwrap(), 200.0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn response_body_capped_at_max_bytes() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    // Cap at 1KB — the /large endpoint returns 100KB
    let caps = http_caps_with_size_limit(vec!["127.0.0.1".to_string()], 1024);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/large" }})));
        return {{ status: resp.status, bodyLen: resp.body.length }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["status"], 200);
        let len = v["bodyLen"].as_u64().unwrap();
        assert!(len <= 1024, "body should be capped at 1024 bytes, got {}", len);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn guest_http_post_json_and_parse_response() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const payload = JSON.stringify({{ name: "test", value: 42 }});
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{
                method: "POST",
                url: "{base}/echo",
                headers: {{ "content-type": "application/json" }},
                body: payload
            }})));
        const echoed = JSON.parse(resp.body);
        return {{ name: echoed.name, value: echoed.value }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["name"], "test");
        assert_eq!(v["value"], 42);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn multiple_http_calls_in_one_sandbox() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        function httpGet(path) {{
            return JSON.parse(__sandcastle_host_call("http", "request",
                JSON.stringify({{ method: "GET", url: "{base}" + path }})));
        }}
        const r1 = httpGet("/hello");
        const r2 = httpGet("/json");
        const r3 = httpGet("/status/201");
        return {{
            call1: r1.body,
            call2: JSON.parse(r2.body).name,
            call3: r3.status
        }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["call1"], "Hello, SandCastle!");
        assert_eq!(v["call2"], "test");
        assert_eq!(v["call3"], 201);
    });
    assert_eq!(result.transcript.capability_calls.len(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn http_invalid_url_throws() {
    let runtime = require_runtime!();
    let caps = http_caps(vec![]);

    let code = r#"
        try {
            __sandcastle_host_call("http", "request",
                JSON.stringify({ method: "GET", url: "not-a-url" }));
            return { threw: false };
        } catch(e) {
            return { threw: true, error: String(e) };
        }
    "#;

    let result = runtime.execute(ExecutionRequest::new(code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["threw"], true);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn http_connection_refused_throws() {
    let runtime = require_runtime!();
    let caps = http_caps(vec![]);

    // Port 1 is almost certainly not listening
    let code = r#"
        try {
            __sandcastle_host_call("http", "request",
                JSON.stringify({ method: "GET", url: "http://127.0.0.1:1/nope" }));
            return { threw: false };
        } catch(e) {
            return { threw: true, error: String(e) };
        }
    "#;

    let result = runtime.execute(ExecutionRequest::new(code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["threw"], true);
        let err = v["error"].as_str().unwrap_or("");
        assert!(err.contains("request failed") || err.contains("error") || err.contains("connect"),
            "error should mention connection failure: {}", err);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn http_response_headers_accessible() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/json" }})));
        return {{
            hasContentType: "content-type" in resp.headers,
            contentType: resp.headers["content-type"] || "missing"
        }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["hasContentType"], true);
        let ct = v["contentType"].as_str().unwrap_or("");
        assert!(ct.contains("json"), "content-type should contain json: {}", ct);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_then_process_pattern() {
    let runtime = require_runtime!();
    let base = start_server(test_app()).await;
    let caps = http_caps(vec!["127.0.0.1".to_string()]);

    // Real agent pattern: fetch API → parse → transform → return
    let code = format!(r#"
        const resp = JSON.parse(__sandcastle_host_call("http", "request",
            JSON.stringify({{ method: "GET", url: "{base}/json" }})));
        if (resp.status !== 200) throw new Error("Bad status: " + resp.status);

        const data = JSON.parse(resp.body);
        const doubled = data.items.map(x => x * 2);
        const sum = doubled.reduce((a, b) => a + b, 0);

        return {{
            source: data.name,
            original: data.items,
            doubled: doubled,
            sum: sum
        }};
    "#);

    let result = runtime.execute(ExecutionRequest::new(&code).with_capabilities(caps)).await.unwrap();
    assert_json(&result, |v| {
        assert_eq!(v["source"], "test");
        assert_eq!(v["original"], serde_json::json!([1, 2, 3]));
        assert_eq!(v["doubled"], serde_json::json!([2, 4, 6]));
        assert_eq!(v["sum"], 12);
    });
}
