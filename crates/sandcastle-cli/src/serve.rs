use std::sync::Arc;

use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post};
use axum::Router;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tracing::info;

use sandcastle::capability::CapabilityRegistry;
use sandcastle::limits::Limits;
use sandcastle::namespace::{DispatchNamespace, NamespaceLimits, NamespaceManager};
use sandcastle::registry::ScriptRegistry;
use sandcastle::sandbox::ExecutionRequest;
use sandcastle::SandCastle;

/// Shared state for the HTTP server.
pub struct AppState {
    pub runtime: SandCastle,
    pub registry: ScriptRegistry,
    pub namespaces: NamespaceManager,
    pub default_capabilities: Arc<CapabilityRegistry>,
}

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ExecuteBody {
    pub code: String,
    pub input: Option<serde_json::Value>,
    pub limits: Option<LimitsBody>,
}

#[derive(Deserialize)]
pub struct RegisterBody {
    pub name: String,
    pub code: String,
    pub limits: Option<LimitsBody>,
}

#[derive(Deserialize)]
pub struct DispatchBody {
    pub input: Option<serde_json::Value>,
    pub limits: Option<LimitsBody>,
}

#[derive(Deserialize)]
pub struct CreateNamespaceBody {
    pub name: String,
    pub max_scripts: Option<usize>,
    pub max_concurrent: Option<usize>,
}

#[derive(Deserialize)]
pub struct LimitsBody {
    pub memory_mb: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub fuel: Option<u64>,
}

#[derive(Serialize)]
struct OkResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    names: Option<Vec<String>>,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    error: String,
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        // Ad-hoc execution
        .route("/execute", post(execute_handler))
        // Script registry
        .route("/scripts", post(register_handler))
        .route("/scripts", get(list_scripts_handler))
        .route("/scripts/{name}", delete(remove_script_handler))
        // Dispatch
        .route("/dispatch/{name}", post(dispatch_handler))
        // Namespaces
        .route("/namespaces", post(create_namespace_handler))
        .route("/namespaces", get(list_namespaces_handler))
        .route("/namespaces/{ns}", delete(delete_namespace_handler))
        .route(
            "/namespaces/{ns}/scripts",
            post(ns_register_handler),
        )
        .route(
            "/namespaces/{ns}/scripts",
            get(ns_list_scripts_handler),
        )
        .route(
            "/namespaces/{ns}/scripts/{name}",
            delete(ns_remove_script_handler),
        )
        .route(
            "/namespaces/{ns}/dispatch/{name}",
            post(ns_dispatch_handler),
        )
        // Health
        .route("/health", get(health_handler))
        .with_state(state)
}

fn limits_from_body(body: &Option<LimitsBody>) -> Limits {
    let mut limits = Limits::default();
    if let Some(b) = body {
        if let Some(m) = b.memory_mb {
            limits.memory_mb = m;
        }
        if let Some(t) = b.timeout_ms {
            limits.timeout = std::time::Duration::from_millis(t);
        }
        if let Some(f) = b.fuel {
            limits.fuel = f;
        }
    }
    limits
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn execute_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<ExecuteBody>,
) -> impl IntoResponse {
    let limits = limits_from_body(&body.limits);
    let request = ExecutionRequest::new(body.code)
        .with_input(body.input.unwrap_or(serde_json::Value::Null))
        .with_capabilities(state.default_capabilities.clone())
        .with_limits(limits);

    match state.runtime.execute(request).await {
        Ok(result) => match serde_json::to_value(&result) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { ok: false, error: format!("serialization error: {e}") }),
            ).into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                ok: false,
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn register_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    let limits = limits_from_body(&body.limits);
    match state.registry.register(
        &body.name,
        body.code,
        state.default_capabilities.clone(),
        limits,
    ) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(OkResponse {
                ok: true,
                name: Some(body.name),
                names: None,
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                ok: false,
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn list_scripts_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(OkResponse {
        ok: true,
        name: None,
        names: Some(state.registry.list()),
    })
}

async fn remove_script_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    if state.registry.remove(&name) {
        (StatusCode::OK, Json(OkResponse { ok: true, name: Some(name), names: None })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                ok: false,
                error: format!("script not found: {name}"),
            }),
        )
            .into_response()
    }
}

async fn dispatch_handler(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    Json(body): Json<DispatchBody>,
) -> impl IntoResponse {
    let script = match state.registry.get(&name) {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    ok: false,
                    error: format!("script not found: {name}"),
                }),
            )
                .into_response()
        }
    };

    let mut limits = script.limits;
    if let Some(b) = &body.limits {
        if let Some(m) = b.memory_mb { limits.memory_mb = m; }
        if let Some(t) = b.timeout_ms { limits.timeout = std::time::Duration::from_millis(t); }
        if let Some(f) = b.fuel { limits.fuel = f; }
    }

    let request = ExecutionRequest::new(&script.code)
        .with_input(body.input.unwrap_or(serde_json::Value::Null))
        .with_capabilities(script.capabilities.clone())
        .with_limits(limits);

    match state.runtime.execute(request).await {
        Ok(result) => match serde_json::to_value(&result) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { ok: false, error: format!("serialization error: {e}") }),
            ).into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { ok: false, error: e.to_string() }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Namespace handlers
// ---------------------------------------------------------------------------

async fn create_namespace_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateNamespaceBody>,
) -> impl IntoResponse {
    let limits = NamespaceLimits {
        max_scripts: body.max_scripts.unwrap_or(1000),
        max_concurrent_executions: body.max_concurrent.unwrap_or(100),
        default_limits: Limits::default(),
    };

    match state.namespaces.create(&body.name, limits, state.default_capabilities.clone()) {
        Ok(_) => (
            StatusCode::CREATED,
            Json(OkResponse { ok: true, name: Some(body.name), names: None }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { ok: false, error: e.to_string() }),
        )
            .into_response(),
    }
}

async fn list_namespaces_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(OkResponse {
        ok: true,
        name: None,
        names: Some(state.namespaces.list()),
    })
}

async fn delete_namespace_handler(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
) -> impl IntoResponse {
    if state.namespaces.delete(&ns) {
        (StatusCode::OK, Json(OkResponse { ok: true, name: Some(ns), names: None })).into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { ok: false, error: format!("namespace not found: {ns}") }),
        )
            .into_response()
    }
}

async fn ns_register_handler(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
    Json(body): Json<RegisterBody>,
) -> impl IntoResponse {
    let namespace = match state.namespaces.get(&ns) {
        Some(n) => n,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse { ok: false, error: format!("namespace not found: {ns}") }),
            )
                .into_response()
        }
    };

    let limits = limits_from_body(&body.limits);
    match namespace.register(&body.name, body.code, Some(limits)) {
        Ok(()) => (
            StatusCode::CREATED,
            Json(OkResponse { ok: true, name: Some(body.name), names: None }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse { ok: false, error: e.to_string() }),
        )
            .into_response(),
    }
}

async fn ns_list_scripts_handler(
    State(state): State<Arc<AppState>>,
    Path(ns): Path<String>,
) -> impl IntoResponse {
    match state.namespaces.get(&ns) {
        Some(namespace) => Json(OkResponse {
            ok: true,
            name: None,
            names: Some(namespace.list_scripts()),
        })
        .into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { ok: false, error: format!("namespace not found: {ns}") }),
        )
            .into_response(),
    }
}

async fn ns_remove_script_handler(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
) -> impl IntoResponse {
    match state.namespaces.get(&ns) {
        Some(namespace) => {
            if namespace.remove(&name) {
                (StatusCode::OK, Json(OkResponse { ok: true, name: Some(name), names: None })).into_response()
            } else {
                (
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse { ok: false, error: format!("script not found: {name}") }),
                )
                    .into_response()
            }
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse { ok: false, error: format!("namespace not found: {ns}") }),
        )
            .into_response(),
    }
}

async fn ns_dispatch_handler(
    State(state): State<Arc<AppState>>,
    Path((ns, name)): Path<(String, String)>,
    Json(body): Json<DispatchBody>,
) -> impl IntoResponse {
    let namespace = match state.namespaces.get(&ns) {
        Some(n) => n,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse { ok: false, error: format!("namespace not found: {ns}") }),
            )
                .into_response()
        }
    };

    // Acquire namespace concurrency permit
    let _permit = match namespace.acquire_permit() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(ErrorResponse { ok: false, error: e.to_string() }),
            )
                .into_response()
        }
    };

    let script = match namespace.get_script(&name) {
        Some(s) => s,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse { ok: false, error: format!("script not found: {name}") }),
            )
                .into_response()
        }
    };

    let mut limits = script.limits;
    if let Some(b) = &body.limits {
        if let Some(m) = b.memory_mb { limits.memory_mb = m; }
        if let Some(t) = b.timeout_ms { limits.timeout = std::time::Duration::from_millis(t); }
        if let Some(f) = b.fuel { limits.fuel = f; }
    }

    let request = ExecutionRequest::new(&script.code)
        .with_input(body.input.unwrap_or(serde_json::Value::Null))
        .with_capabilities(script.capabilities.clone())
        .with_limits(limits);

    match state.runtime.execute(request).await {
        Ok(result) => match serde_json::to_value(&result) {
            Ok(val) => (StatusCode::OK, Json(val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse { ok: false, error: format!("serialization error: {e}") }),
            ).into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse { ok: false, error: e.to_string() }),
        )
            .into_response(),
    }
}

/// Start the HTTP server.
pub async fn start(state: Arc<AppState>, addr: &str) -> anyhow::Result<()> {
    let app = router(state);
    let listener = TcpListener::bind(addr).await?;
    info!("SandCastle HTTP server listening on {addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C signal handler");
    info!("Shutting down HTTP server...");
}
