use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use tracing::debug;
use wasmtime::{Caller, Engine, Linker, Module, Store, StoreLimits, StoreLimitsBuilder, Trap};
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::preview1::WasiP1Ctx;

use crate::capability::{CapabilityBridge, CapabilityRegistry};
use crate::error::{ExecutionError, Result, SandcastleError};
use crate::limits::Limits;
use crate::transcript::{ExecutionTranscript, TranscriptRecorder};
use crate::types::{
    Artifact, CapabilityCall, ConsoleLevel, ConsoleMessage, ExecutionStatus, OutputArtifact,
    OutputValue,
};

/// Drop guard that aborts a spawned task when dropped.
/// Ensures the timeout task is cancelled even on early error returns.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Maximum allowed buffer size for a single guest-to-host parameter (16 MB).
/// Prevents a malicious guest from causing OOM via large allocation requests.
const MAX_GUEST_BUFFER_SIZE: usize = 16 * 1024 * 1024;

/// Validate an i32 length parameter from the guest.
/// Returns the length as usize, or 0 if negative, capped at MAX_GUEST_BUFFER_SIZE.
fn validated_len(len: i32) -> usize {
    if len <= 0 {
        return 0;
    }
    (len as usize).min(MAX_GUEST_BUFFER_SIZE)
}

/// Safely get the "memory" export from a Caller. Returns -1 sentinel if missing.
macro_rules! get_memory {
    ($caller:expr) => {
        match $caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
        {
            Some(mem) => mem,
            None => return -1,
        }
    };
    ($caller:expr, void) => {
        match $caller
            .get_export("memory")
            .and_then(|e| e.into_memory())
        {
            Some(mem) => mem,
            None => return,
        }
    };
}

/// Request to execute code in a sandbox.
pub struct ExecutionRequest {
    /// JavaScript code to execute.
    pub code: String,
    /// JSON input available to the guest code.
    pub input: serde_json::Value,
    /// Host capabilities available to this execution.
    pub capabilities: Arc<CapabilityRegistry>,
    /// Resource limits.
    pub limits: Limits,
    /// Input artifacts to mount.
    pub input_artifacts: Vec<Artifact>,
}

impl ExecutionRequest {
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            input: serde_json::Value::Null,
            capabilities: Arc::new(CapabilityRegistry::new()),
            limits: Limits::default(),
            input_artifacts: vec![],
        }
    }

    pub fn with_input(mut self, input: serde_json::Value) -> Self {
        self.input = input;
        self
    }

    pub fn with_capabilities(mut self, capabilities: Arc<CapabilityRegistry>) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_artifacts(mut self, artifacts: Vec<Artifact>) -> Self {
        self.input_artifacts = artifacts;
        self
    }
}

/// Result of a sandbox execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    pub status: ExecutionStatus,
    pub output: OutputValue,
    pub transcript: ExecutionTranscript,
    pub output_artifacts: Vec<OutputArtifact>,
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

/// State held inside the Wasmtime Store for host function access.
pub(crate) struct SandboxState {
    pub(crate) limits: StoreLimits,
    pub(crate) wasi: WasiP1Ctx,
    pub(crate) console_messages: Vec<ConsoleMessage>,
    pub(crate) capability_bridge: Option<Arc<CapabilityBridge>>,
    pub(crate) output: OutputValue,
    pub(crate) output_artifacts: Vec<OutputArtifact>,
    pub(crate) input_artifacts: Vec<Artifact>,
    pub(crate) input_json: serde_json::Value,
    pub(crate) start_time: Instant,
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) recorder: TranscriptRecorder,
}

/// A sandbox instance. Each sandbox gets its own Wasmtime Store.
pub struct Sandbox {
    engine: Engine,
    module: Module,
    linker: Arc<Linker<SandboxState>>,
}

impl Sandbox {
    pub(crate) fn new(
        engine: &Engine,
        module: &Module,
        linker: Arc<Linker<SandboxState>>,
    ) -> Result<Self> {
        Ok(Self {
            engine: engine.clone(),
            module: module.clone(),
            linker,
        })
    }

    /// Build a pre-configured Linker with WASI and sandcastle host functions.
    /// Called once at runtime init; the returned Linker is reused for every execution.
    pub(crate) fn build_linker(engine: &Engine) -> Result<Linker<SandboxState>> {
        let mut linker: Linker<SandboxState> = Linker::new(engine);

        wasmtime_wasi::preview1::add_to_linker_async(
            &mut linker,
            |state: &mut SandboxState| &mut state.wasi,
        )
        .map_err(|e| SandcastleError::RuntimeInit(format!("WASI linking failed: {e}")))?;

        Self::link_host_functions(&mut linker)?;

        Ok(linker)
    }

    /// Execute code in this sandbox.
    pub async fn execute(&self, request: ExecutionRequest) -> Result<ExecutionResult> {
        let fuel_limit = request.limits.fuel;
        let memory_limit = (request.limits.memory_mb as u64) * 1024 * 1024;
        let timeout = request.limits.timeout;

        let store_limits = StoreLimitsBuilder::new()
            .memory_size(memory_limit as usize)
            .instances(1)
            .tables(10)
            .memories(1)
            .trap_on_grow_failure(true)
            .build();

        let recorder = TranscriptRecorder::new(fuel_limit, memory_limit);

        let wasi = WasiCtxBuilder::new().build_p1();

        let state = SandboxState {
            limits: store_limits,
            wasi,
            console_messages: Vec::new(),
            capability_bridge: Some(Arc::new(CapabilityBridge::new(
                request.capabilities.clone(),
            ))),
            output: OutputValue::Null,
            output_artifacts: Vec::new(),
            input_artifacts: request.input_artifacts,
            input_json: request.input,
            start_time: Instant::now(),
            cancelled: Arc::new(AtomicBool::new(false)),
            recorder,
        };

        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);

        // Always set fuel. When fuel_limit is 0, use u64::MAX for effectively unlimited.
        let effective_fuel = if fuel_limit > 0 { fuel_limit } else { u64::MAX };
        store
            .set_fuel(effective_fuel)
            .map_err(SandcastleError::Wasm)?;
        // Set epoch deadline high enough to not immediately trip.
        // The timeout handler will increment the engine epoch past this.
        store.set_epoch_deadline(100);

        let instance = self
            .linker
            .instantiate_async(&mut store, &self.module)
            .await
            .map_err(|e| SandcastleError::SandboxCreation(e.to_string()))?;

        // Set up epoch-based timeout with a drop guard to ensure cancellation
        // even on early error returns.
        let engine_clone = self.engine.clone();
        let cancelled = store.data().cancelled.clone();
        let _timeout_guard = AbortOnDrop(tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            cancelled.store(true, Ordering::SeqCst);
            // Increment the epoch past the store's deadline
            for _ in 0..200 {
                engine_clone.increment_epoch();
            }
        }));

        // Get the evaluate function from the guest
        let evaluate = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut store, "evaluate")
            .map_err(|e| {
                SandcastleError::SandboxCreation(format!(
                    "guest module missing 'evaluate' export: {e}"
                ))
            })?;

        // Get memory export
        let memory = instance.get_memory(&mut store, "memory").ok_or_else(|| {
            SandcastleError::SandboxCreation("guest module missing 'memory' export".into())
        })?;

        // Write code and input into guest memory via alloc
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut store, "alloc")
            .map_err(|e| {
                SandcastleError::SandboxCreation(format!(
                    "guest module missing 'alloc' export: {e}"
                ))
            })?;

        let code_bytes = request.code.as_bytes();
        let input_bytes = serde_json::to_vec(&store.data().input_json)
            .map_err(|e| SandcastleError::Serialization(e.to_string()))?;

        // Allocate and write code
        let code_ptr = alloc
            .call_async(&mut store, code_bytes.len() as i32)
            .await
            .map_err(|e| {
                SandcastleError::Execution(ExecutionError::GuestError {
                    message: format!("alloc failed: {e}"),
                })
            })?;

        memory
            .write(&mut store, code_ptr as usize, code_bytes)
            .map_err(|e| {
                SandcastleError::Execution(ExecutionError::GuestError {
                    message: format!("memory write failed: {e}"),
                })
            })?;

        // Allocate and write input
        let input_ptr = alloc
            .call_async(&mut store, input_bytes.len() as i32)
            .await
            .map_err(|e| {
                SandcastleError::Execution(ExecutionError::GuestError {
                    message: format!("alloc failed: {e}"),
                })
            })?;

        memory
            .write(&mut store, input_ptr as usize, &input_bytes)
            .map_err(|e| {
                SandcastleError::Execution(ExecutionError::GuestError {
                    message: format!("memory write failed: {e}"),
                })
            })?;

        // Call evaluate
        let result = evaluate
            .call_async(
                &mut store,
                (
                    code_ptr,
                    code_bytes.len() as i32,
                    input_ptr,
                    input_bytes.len() as i32,
                ),
            )
            .await;

        // _timeout_guard drop will abort the timeout task automatically.

        // Determine execution status
        let was_cancelled = store.data().cancelled.load(Ordering::SeqCst);

        let fuel_consumed = effective_fuel.saturating_sub(store.get_fuel().unwrap_or(0));

        let peak_memory = memory.data_size(&store) as u64;

        // C5: Use Wasmtime's Trap type for proper error classification
        let (status, output) = match result {
            Ok(0) => {
                let output = std::mem::replace(&mut store.data_mut().output, OutputValue::Null);
                (ExecutionStatus::Success, output)
            }
            Ok(_code) => {
                // Extract the JS error message from the structured error output
                // that the guest sets before returning a non-zero exit code.
                let error_message = match &store.data().output {
                    OutputValue::Json(v) => {
                        if v.get("__sandcastle_error")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                        {
                            v.get("message")
                                .and_then(|m| m.as_str())
                                .map(|s| s.to_owned())
                        } else {
                            None
                        }
                    }
                    _ => None,
                };

                // Fallback: scan console.error messages for the error
                let message = error_message.unwrap_or_else(|| {
                    store
                        .data()
                        .console_messages
                        .iter()
                        .rev()
                        .find(|m| m.level == ConsoleLevel::Error)
                        .map(|m| m.message.clone())
                        .unwrap_or_else(|| format!("guest returned error code: {_code}"))
                });

                (
                    ExecutionStatus::GuestError { message },
                    OutputValue::Null,
                )
            }
            Err(e) => {
                if was_cancelled {
                    (ExecutionStatus::Timeout, OutputValue::Null)
                } else {
                    // Walk the error chain to find a Trap
                    let trap = e.downcast_ref::<Trap>().copied().or_else(|| {
                        e.chain()
                            .find_map(|cause| cause.downcast_ref::<Trap>().copied())
                    });

                    // Check if any error in the chain mentions memory growth failure
                    let is_memory_error = e.chain().any(|cause| {
                        let msg = cause.to_string();
                        msg.contains("forcing trap when growing memory")
                            || msg.contains("memory minimum size")
                    });

                    if is_memory_error {
                        (ExecutionStatus::MemoryExceeded, OutputValue::Null)
                    } else if let Some(trap) = trap {
                        match trap {
                            Trap::OutOfFuel => {
                                (ExecutionStatus::FuelExhausted, OutputValue::Null)
                            }
                            Trap::Interrupt => {
                                (ExecutionStatus::Timeout, OutputValue::Null)
                            }
                            Trap::UnreachableCodeReached => {
                                // Often caused by memory allocation failure in the guest.
                                // Check if we're close to the memory limit.
                                let usage_ratio =
                                    peak_memory as f64 / memory_limit as f64;
                                if usage_ratio > 0.85 {
                                    (ExecutionStatus::MemoryExceeded, OutputValue::Null)
                                } else {
                                    (
                                        ExecutionStatus::GuestError {
                                            message: format!("WASM trap: {trap}"),
                                        },
                                        OutputValue::Null,
                                    )
                                }
                            }
                            _ => (
                                ExecutionStatus::GuestError {
                                    message: format!("WASM trap: {trap}"),
                                },
                                OutputValue::Null,
                            ),
                        }
                    } else {
                        (
                            ExecutionStatus::GuestError {
                                message: e.to_string(),
                            },
                            OutputValue::Null,
                        )
                    }
                }
            }
        };

        // Build transcript
        let state = store.into_data();
        let mut recorder = state.recorder;
        recorder.set_output(output.clone());
        recorder.set_fuel_consumed(fuel_consumed);
        recorder.set_peak_memory(peak_memory);
        let transcript = recorder.finalize(status.clone());

        Ok(ExecutionResult {
            status,
            output,
            transcript,
            output_artifacts: state.output_artifacts,
        })
    }

    fn link_host_functions(linker: &mut Linker<SandboxState>) -> Result<()> {
        // Console logging: __sandcastle_console(level: i32, ptr: i32, len: i32)
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_console",
                |mut caller: Caller<'_, SandboxState>, level: i32, ptr: i32, len: i32| {
                    let memory = get_memory!(caller, void);
                    let safe_len = validated_len(len);
                    if safe_len == 0 {
                        return;
                    }
                    let mut buf = vec![0u8; safe_len];
                    if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                        return;
                    }
                    let message = String::from_utf8_lossy(&buf).into_owned();
                    let elapsed_ms = caller.data().start_time.elapsed().as_millis() as u64;

                    let console_level = match level {
                        0 => ConsoleLevel::Log,
                        1 => ConsoleLevel::Warn,
                        2 => ConsoleLevel::Error,
                        3 => ConsoleLevel::Debug,
                        _ => ConsoleLevel::Log,
                    };

                    debug!(level = ?console_level, %message, "guest console");

                    let recorder_msg = message.clone();
                    let msg = ConsoleMessage {
                        level: console_level,
                        message,
                        ts: elapsed_ms,
                    };
                    caller.data_mut().console_messages.push(msg);
                    caller
                        .data_mut()
                        .recorder
                        .record_console(console_level, recorder_msg);
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        // Set output: __sandcastle_set_output(ptr: i32, len: i32)
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_set_output",
                |mut caller: Caller<'_, SandboxState>, ptr: i32, len: i32| {
                    let memory = get_memory!(caller, void);
                    let safe_len = validated_len(len);
                    if safe_len == 0 {
                        return;
                    }
                    let mut buf = vec![0u8; safe_len];
                    if memory.read(&caller, ptr as usize, &mut buf).is_err() {
                        return;
                    }

                    match serde_json::from_slice(&buf) {
                        Ok(value) => {
                            caller.data_mut().output = OutputValue::Json(value);
                        }
                        Err(_) => {
                            let s = String::from_utf8_lossy(&buf).into_owned();
                            caller.data_mut().output = OutputValue::String(s);
                        }
                    }
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        // Get input: __sandcastle_get_input(buf_ptr: i32, buf_len: i32) -> i32 (actual len)
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_get_input",
                |mut caller: Caller<'_, SandboxState>, buf_ptr: i32, buf_len: i32| -> i32 {
                    let input = serde_json::to_vec(&caller.data().input_json).unwrap_or_default();
                    let memory = get_memory!(caller);
                    let safe_len = validated_len(buf_len);
                    let copy_len = input.len().min(safe_len);
                    if copy_len > 0 {
                        if memory
                            .write(&mut caller, buf_ptr as usize, &input[..copy_len])
                            .is_err()
                        {
                            return -1;
                        }
                    }
                    input.len() as i32
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        // Host capability call
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_host_call",
                |mut caller: Caller<'_, SandboxState>,
                 cap_ptr: i32,
                 cap_len: i32,
                 method_ptr: i32,
                 method_len: i32,
                 payload_ptr: i32,
                 payload_len: i32,
                 result_ptr: i32,
                 result_buf_len: i32|
                 -> i32 {
                    let memory = get_memory!(caller);

                    let safe_cap_len = validated_len(cap_len);
                    let safe_method_len = validated_len(method_len);
                    let safe_payload_len = validated_len(payload_len);
                    let safe_result_len = validated_len(result_buf_len);

                    let mut cap_buf = vec![0u8; safe_cap_len];
                    let mut method_buf = vec![0u8; safe_method_len];
                    let mut payload_buf = vec![0u8; safe_payload_len];

                    if memory.read(&caller, cap_ptr as usize, &mut cap_buf).is_err()
                        || memory
                            .read(&caller, method_ptr as usize, &mut method_buf)
                            .is_err()
                        || memory
                            .read(&caller, payload_ptr as usize, &mut payload_buf)
                            .is_err()
                    {
                        return -1;
                    }

                    let capability = String::from_utf8_lossy(&cap_buf).into_owned();
                    let method = String::from_utf8_lossy(&method_buf).into_owned();

                    let bridge = caller.data().capability_bridge.clone();
                    let start = Instant::now();

                    if let Some(bridge) = bridge {
                        match bridge.dispatch_sync(&capability, &method, &payload_buf) {
                            Ok(result_bytes) => {
                                let duration_ms = start.elapsed().as_millis() as u64;
                                let ts =
                                    caller.data().start_time.elapsed().as_millis() as u64;

                                // Record the call in the transcript recorder (single source)
                                let input_val =
                                    serde_json::from_slice(&payload_buf).unwrap_or_default();
                                let output_val: Option<serde_json::Value> =
                                    serde_json::from_slice(&result_bytes).ok();
                                caller.data_mut().recorder.record_capability_call(
                                    CapabilityCall {
                                        capability,
                                        method,
                                        input: input_val,
                                        output: output_val,
                                        error: None,
                                        duration_ms,
                                        ts,
                                    },
                                );

                                let copy_len = result_bytes.len().min(safe_result_len);
                                if copy_len > 0 {
                                    if memory
                                        .write(
                                            &mut caller,
                                            result_ptr as usize,
                                            &result_bytes[..copy_len],
                                        )
                                        .is_err()
                                    {
                                        return -1;
                                    }
                                }
                                result_bytes.len() as i32
                            }
                            Err(e) => {
                                let duration_ms = start.elapsed().as_millis() as u64;
                                let ts =
                                    caller.data().start_time.elapsed().as_millis() as u64;
                                let input_val: serde_json::Value =
                                    serde_json::from_slice(&payload_buf).unwrap_or_default();

                                caller.data_mut().recorder.record_capability_call(
                                    CapabilityCall {
                                        capability,
                                        method,
                                        input: input_val,
                                        output: None,
                                        error: Some(e.to_string()),
                                        duration_ms,
                                        ts,
                                    },
                                );

                                let error_msg = e.to_string();
                                let error_bytes = error_msg.as_bytes();
                                let copy_len = error_bytes.len().min(safe_result_len);
                                if copy_len > 0 {
                                    let _ = memory.write(
                                        &mut caller,
                                        result_ptr as usize,
                                        &error_bytes[..copy_len],
                                    );
                                }
                                -(error_bytes.len() as i32)
                            }
                        }
                    } else {
                        -1
                    }
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        // Read input artifact
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_read_artifact",
                |mut caller: Caller<'_, SandboxState>,
                 name_ptr: i32,
                 name_len: i32,
                 buf_ptr: i32,
                 buf_len: i32|
                 -> i32 {
                    let memory = get_memory!(caller);
                    let safe_name_len = validated_len(name_len);
                    let safe_buf_len = validated_len(buf_len);

                    let mut name_buf = vec![0u8; safe_name_len];
                    if memory
                        .read(&caller, name_ptr as usize, &mut name_buf)
                        .is_err()
                    {
                        return -1;
                    }
                    let name = String::from_utf8_lossy(&name_buf).into_owned();

                    // Clone to avoid borrow conflict with caller
                    let artifact_data = caller
                        .data()
                        .input_artifacts
                        .iter()
                        .find(|a| a.name == name)
                        .map(|a| a.data.clone());

                    match artifact_data {
                        Some(data) => {
                            let copy_len = data.len().min(safe_buf_len);
                            if copy_len > 0 {
                                if memory
                                    .write(&mut caller, buf_ptr as usize, &data[..copy_len])
                                    .is_err()
                                {
                                    return -1;
                                }
                            }
                            data.len() as i32
                        }
                        None => -1,
                    }
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        // Write output artifact
        linker
            .func_wrap(
                "sandcastle",
                "__sandcastle_write_artifact",
                |mut caller: Caller<'_, SandboxState>,
                 name_ptr: i32,
                 name_len: i32,
                 data_ptr: i32,
                 data_len: i32|
                 -> i32 {
                    let memory = get_memory!(caller);

                    let safe_name_len = validated_len(name_len);
                    let safe_data_len = validated_len(data_len);

                    let mut name_buf = vec![0u8; safe_name_len];
                    let mut data_buf = vec![0u8; safe_data_len];
                    if memory
                        .read(&caller, name_ptr as usize, &mut name_buf)
                        .is_err()
                        || memory
                            .read(&caller, data_ptr as usize, &mut data_buf)
                            .is_err()
                    {
                        return -1;
                    }

                    let name = String::from_utf8_lossy(&name_buf).into_owned();
                    caller.data_mut().output_artifacts.push(OutputArtifact {
                        name,
                        data: data_buf,
                    });
                    0
                },
            )
            .map_err(|e| SandcastleError::RuntimeInit(e.to_string()))?;

        Ok(())
    }
}
