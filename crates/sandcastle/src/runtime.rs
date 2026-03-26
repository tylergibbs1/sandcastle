use std::sync::Arc;

use tokio::sync::Semaphore;
use tracing::{debug, info};
use wasmtime::{Config as WasmConfig, Engine, Linker, Module};

use crate::error::{Result, SandcastleError};
use crate::sandbox::{ExecutionRequest, ExecutionResult, Sandbox, SandboxState};
use crate::types::SecurityMode;

/// Configuration for the SandCastle runtime.
#[derive(Debug, Clone)]
pub struct Config {
    /// Security mode for sandbox execution.
    pub security_mode: SecurityMode,
    /// Maximum number of concurrent sandboxes.
    pub max_concurrent_sandboxes: usize,
    /// Pre-compiled QuickJS WASM module bytes.
    pub guest_module: Vec<u8>,
}

impl Config {
    pub fn new(guest_module: Vec<u8>) -> Self {
        Self {
            security_mode: SecurityMode::Standard,
            max_concurrent_sandboxes: 1000,
            guest_module,
        }
    }
}

/// The SandCastle runtime. Create once at application startup.
pub struct SandCastle {
    engine: Engine,
    module: Module,
    linker: Arc<Linker<SandboxState>>,
    #[expect(dead_code, reason = "will be used when Hardened mode is implemented")]
    security_mode: SecurityMode,
    concurrency_semaphore: Arc<Semaphore>,
}

impl SandCastle {
    /// Create a new SandCastle runtime with the given configuration.
    pub fn new(config: Config) -> Result<Self> {
        if config.security_mode == SecurityMode::Hardened {
            return Err(SandcastleError::RuntimeInit(
                "SecurityMode::Hardened is not yet implemented".into(),
            ));
        }

        let mut wasm_config = WasmConfig::new();
        wasm_config.async_support(true);
        wasm_config.consume_fuel(true);
        wasm_config.epoch_interruption(true);
        wasm_config.wasm_bulk_memory(true);
        wasm_config.wasm_multi_value(true);

        let engine = Engine::new(&wasm_config)
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        info!("Compiling guest WASM module ({} bytes)", config.guest_module.len());
        let module = Module::new(&engine, &config.guest_module)
            .map_err(|e| SandcastleError::Compilation(e.to_string()))?;

        let linker = Arc::new(Sandbox::build_linker(&engine)?);

        let concurrency_semaphore = Arc::new(Semaphore::new(config.max_concurrent_sandboxes));
        let security_mode = config.security_mode;

        info!(
            mode = ?security_mode,
            max_concurrent = config.max_concurrent_sandboxes,
            "SandCastle runtime initialized"
        );

        // Drop config (and its guest_module bytes) by not storing it
        Ok(Self {
            engine,
            module,
            linker,
            security_mode,
            concurrency_semaphore,
        })
    }

    /// Execute code in a new sandbox. This is the primary API.
    ///
    /// Creates a sandbox, runs the code, and destroys the sandbox.
    /// Returns the execution result including output, transcript, and status.
    pub async fn execute(
        &self,
        request: ExecutionRequest,
    ) -> Result<ExecutionResult> {
        let _permit = self
            .concurrency_semaphore
            .acquire()
            .await
            .map_err(|_| SandcastleError::ResourceLimit("runtime is shutting down".into()))?;

        debug!(code_len = request.code.len(), "Creating sandbox for execution");

        let sandbox = Sandbox::new(&self.engine, &self.module, self.linker.clone())?;

        sandbox.execute(request).await
    }

    /// Create a retained sandbox for multi-turn execution.
    pub fn create_sandbox(&self) -> Result<Sandbox> {
        Sandbox::new(&self.engine, &self.module, self.linker.clone())
    }

    /// Dispatch to a pre-registered script by name.
    ///
    /// The script must have been registered via a `ScriptRegistry`.
    /// This is a convenience that looks up the script and calls `execute()`.
    pub async fn dispatch(
        &self,
        registry: &crate::registry::ScriptRegistry,
        script_name: &str,
        input: serde_json::Value,
    ) -> Result<ExecutionResult> {
        let script = registry
            .get(script_name)
            .ok_or_else(|| SandcastleError::ScriptNotFound(script_name.to_owned()))?;

        let request = ExecutionRequest::new(&script.code)
            .with_input(input)
            .with_capabilities(script.capabilities.clone())
            .with_limits(script.limits);

        self.execute(request).await
    }

    /// Get a reference to the WASM engine.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }
}
