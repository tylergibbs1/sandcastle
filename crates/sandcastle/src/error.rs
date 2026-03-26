use thiserror::Error;

/// Top-level error type for SandCastle operations.
#[derive(Debug, Error)]
pub enum SandcastleError {
    #[error("runtime initialization failed: {0}")]
    RuntimeInit(String),

    #[error("module compilation failed: {0}")]
    Compilation(String),

    #[error("sandbox creation failed: {0}")]
    SandboxCreation(String),

    #[error("execution failed: {0}")]
    Execution(#[from] ExecutionError),

    #[error("capability error: {0}")]
    Capability(#[from] CapabilityError),

    #[error("wasm error: {0}")]
    Wasm(#[from] wasmtime::Error),

    #[error("serialization error: {0}")]
    Serialization(String),

    #[error("resource limit exceeded: {0}")]
    ResourceLimit(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("script not found: {0}")]
    ScriptNotFound(String),

    #[error("namespace not found: {0}")]
    NamespaceNotFound(String),

    #[error("namespace already exists: {0}")]
    NamespaceAlreadyExists(String),
}

/// Errors that occur during sandbox code execution.
#[derive(Debug, Error, Clone)]
pub enum ExecutionError {
    #[error("guest code error: {message}")]
    GuestError { message: String },

    #[error("execution timed out after {elapsed_ms}ms (limit: {limit_ms}ms)")]
    Timeout { elapsed_ms: u64, limit_ms: u64 },

    #[error("fuel exhausted: consumed {consumed} of {limit} fuel units")]
    FuelExhausted { consumed: u64, limit: u64 },

    #[error("memory limit exceeded: {used_bytes} bytes (limit: {limit_bytes} bytes)")]
    MemoryExceeded { used_bytes: u64, limit_bytes: u64 },

    #[error("output size limit exceeded: {size_bytes} bytes (limit: {limit_bytes} bytes)")]
    OutputSizeExceeded { size_bytes: u64, limit_bytes: u64 },

    #[error("sandbox was cancelled")]
    Cancelled,
}

/// Errors from host capability invocations.
#[derive(Debug, Error, Clone)]
pub enum CapabilityError {
    #[error("capability `{capability}::{method}` not found")]
    NotFound { capability: String, method: String },

    #[error("call quota exceeded for `{capability}::{method}`: {count}/{max} calls")]
    CallQuotaExceeded {
        capability: String,
        method: String,
        count: u32,
        max: u32,
    },

    #[error("payload too large for `{capability}::{method}`: {size} bytes (limit: {limit} bytes)")]
    PayloadTooLarge {
        capability: String,
        method: String,
        size: usize,
        limit: usize,
    },

    #[error("total transfer limit exceeded for `{capability}`: {total} bytes (limit: {limit} bytes)")]
    TransferLimitExceeded {
        capability: String,
        total: usize,
        limit: usize,
    },

    #[error("capability call timed out: `{capability}::{method}` after {elapsed_ms}ms")]
    Timeout {
        capability: String,
        method: String,
        elapsed_ms: u64,
    },

    #[error("concurrency limit exceeded for `{capability}`: {active}/{max} concurrent calls")]
    ConcurrencyExceeded {
        capability: String,
        active: u32,
        max: u32,
    },

    #[error("capability invocation failed: `{capability}::{method}`: {message}")]
    InvocationFailed {
        capability: String,
        method: String,
        message: String,
    },

    #[error("serialization error in capability call: {0}")]
    Serialization(String),
}

pub type Result<T> = std::result::Result<T, SandcastleError>;

// Compile-time assertion: SandcastleError must be Send + Sync for async boundaries.
const _: () = {
    fn _assert_send_sync<T: Send + Sync>() {}
    fn _check() {
        _assert_send_sync::<SandcastleError>();
    }
};
