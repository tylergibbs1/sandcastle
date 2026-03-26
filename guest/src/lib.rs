//! SandCastle Guest JS Runtime
//!
//! This module is compiled to wasm32-wasip1 and loaded by the SandCastle host.
//! It embeds QuickJS (via rquickjs) and provides the evaluate/alloc exports
//! that the host calls into.
//!
//! # Exports
//! - `alloc(size: i32) -> i32` — allocate guest memory
//! - `dealloc(ptr: i32, size: i32)` — free guest memory
//! - `evaluate(code_ptr, code_len, input_ptr, input_len) -> i32` — run JS code
//!
//! # Imports (from "sandcastle" module)
//! - `__sandcastle_console(level, ptr, len)`
//! - `__sandcastle_set_output(ptr, len)`
//! - `__sandcastle_get_input(buf_ptr, buf_len) -> i32`
//! - `__sandcastle_host_call(cap_ptr, cap_len, method_ptr, method_len, payload_ptr, payload_len, result_ptr, result_buf_len) -> i32`
//! - `__sandcastle_read_artifact(name_ptr, name_len, buf_ptr, buf_len) -> i32`
//! - `__sandcastle_write_artifact(name_ptr, name_len, data_ptr, data_len) -> i32`

use std::alloc::{alloc as std_alloc, dealloc as std_dealloc, Layout};
use std::slice;

// Host imports from the "sandcastle" WASM module
#[link(wasm_import_module = "sandcastle")]
unsafe extern "C" {
    #[link_name = "__sandcastle_console"]
    fn host_console(level: i32, ptr: *const u8, len: i32);

    #[link_name = "__sandcastle_set_output"]
    fn host_set_output(ptr: *const u8, len: i32);

    #[link_name = "__sandcastle_get_input"]
    fn host_get_input(buf_ptr: *mut u8, buf_len: i32) -> i32;

    #[link_name = "__sandcastle_host_call"]
    fn host_call(
        cap_ptr: *const u8,
        cap_len: i32,
        method_ptr: *const u8,
        method_len: i32,
        payload_ptr: *const u8,
        payload_len: i32,
        result_ptr: *mut u8,
        result_buf_len: i32,
    ) -> i32;

    #[link_name = "__sandcastle_read_artifact"]
    fn host_read_artifact(
        name_ptr: *const u8,
        name_len: i32,
        buf_ptr: *mut u8,
        buf_len: i32,
    ) -> i32;

    #[link_name = "__sandcastle_write_artifact"]
    fn host_write_artifact(
        name_ptr: *const u8,
        name_len: i32,
        data_ptr: *const u8,
        data_len: i32,
    ) -> i32;
}

/// Allocate memory in the guest for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(size: i32) -> *mut u8 {
    if size <= 0 {
        return std::ptr::null_mut();
    }
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { std_alloc(layout) }
}

/// Free guest memory.
#[unsafe(no_mangle)]
pub extern "C" fn dealloc(ptr: *mut u8, size: i32) {
    if ptr.is_null() || size <= 0 {
        return;
    }
    let layout = Layout::from_size_align(size as usize, 1).unwrap();
    unsafe { std_dealloc(ptr, layout) }
}

fn console_log(level: i32, msg: &str) {
    unsafe {
        host_console(level, msg.as_ptr(), msg.len() as i32);
    }
}

fn set_output(data: &[u8]) {
    unsafe {
        host_set_output(data.as_ptr(), data.len() as i32);
    }
}

fn call_host_capability(
    capability: &str,
    method: &str,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    let mut result_buf = vec![0u8; 1024 * 1024]; // 1MB result buffer
    let result_len = unsafe {
        host_call(
            capability.as_ptr(),
            capability.len() as i32,
            method.as_ptr(),
            method.len() as i32,
            payload.as_ptr(),
            payload.len() as i32,
            result_buf.as_mut_ptr(),
            result_buf.len() as i32,
        )
    };

    if result_len < 0 {
        let error_len = (-result_len) as usize;
        let error_msg =
            String::from_utf8_lossy(&result_buf[..error_len.min(result_buf.len())]).to_string();
        Err(error_msg)
    } else {
        result_buf.truncate(result_len as usize);
        Ok(result_buf)
    }
}

fn read_artifact(name: &str) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 16 * 1024 * 1024]; // 16MB max artifact
    let len = unsafe {
        host_read_artifact(
            name.as_ptr(),
            name.len() as i32,
            buf.as_mut_ptr(),
            buf.len() as i32,
        )
    };

    if len < 0 {
        None
    } else {
        buf.truncate(len as usize);
        Some(buf)
    }
}

fn write_artifact(name: &str, data: &[u8]) -> bool {
    let result = unsafe {
        host_write_artifact(
            name.as_ptr(),
            name.len() as i32,
            data.as_ptr(),
            data.len() as i32,
        )
    };
    result == 0
}

/// Main evaluate entry point called by the host.
///
/// Returns 0 on success, non-zero on error.
#[unsafe(no_mangle)]
pub extern "C" fn evaluate(
    code_ptr: *const u8,
    code_len: i32,
    input_ptr: *const u8,
    input_len: i32,
) -> i32 {
    let code = unsafe {
        let slice = slice::from_raw_parts(code_ptr, code_len as usize);
        match std::str::from_utf8(slice) {
            Ok(s) => s,
            Err(_) => {
                console_log(2, "Invalid UTF-8 in code");
                return 1;
            }
        }
    };

    let input = unsafe {
        let slice = slice::from_raw_parts(input_ptr, input_len as usize);
        match serde_json::from_slice::<serde_json::Value>(slice) {
            Ok(v) => v,
            Err(e) => {
                console_log(2, &format!("Invalid JSON input: {e}"));
                return 1;
            }
        }
    };

    match run_js(code, &input) {
        Ok(output) => {
            match serde_json::to_vec(&output) {
                Ok(bytes) => set_output(&bytes),
                Err(e) => {
                    console_log(2, &format!("Failed to serialize output: {e}"));
                    return 1;
                }
            }
            0
        }
        Err(e) => {
            console_log(2, &format!("Execution error: {e}"));
            // Send structured error through set_output so the host can
            // include the JS error message in GuestError.message
            let error_json = serde_json::json!({
                "__sandcastle_error": true,
                "message": e.to_string()
            });
            if let Ok(bytes) = serde_json::to_vec(&error_json) {
                set_output(&bytes);
            }
            1
        }
    }
}

/// Execute JavaScript code using QuickJS via rquickjs.
fn run_js(code: &str, input: &serde_json::Value) -> Result<serde_json::Value, String> {
    let rt = rquickjs::Runtime::new().map_err(|e| format!("Failed to create runtime: {e}"))?;
    let ctx = rquickjs::Context::full(&rt).map_err(|e| format!("Failed to create context: {e}"))?;

    ctx.with(|ctx| {
        // Inject `__sandcastle_input` global
        let input_json = serde_json::to_string(input).unwrap_or_else(|_| "null".to_string());

        // Set up console object
        let setup = r#"
            globalThis.console = {
                log: function(...args) {
                    globalThis.__sandcastle_console_log(0, args.map(a => typeof a === 'string' ? a : JSON.stringify(a)).join(' '));
                },
                warn: function(...args) {
                    globalThis.__sandcastle_console_log(1, args.map(a => typeof a === 'string' ? a : JSON.stringify(a)).join(' '));
                },
                error: function(...args) {
                    globalThis.__sandcastle_console_log(2, args.map(a => typeof a === 'string' ? a : JSON.stringify(a)).join(' '));
                },
                debug: function(...args) {
                    globalThis.__sandcastle_console_log(3, args.map(a => typeof a === 'string' ? a : JSON.stringify(a)).join(' '));
                }
            };
        "#;

        ctx.eval::<(), _>(setup)
            .map_err(|e| format!("Failed to set up console: {e}"))?;

        // Inject the console_log callback
        let console_fn = rquickjs::Function::new(ctx.clone(), |level: i32, msg: String| {
            console_log(level, &msg);
        })
        .map_err(|e| format!("Failed to create console function: {e}"))?;

        let globals = ctx.globals();
        globals
            .set("__sandcastle_console_log", console_fn)
            .map_err(|e| format!("Failed to set console function: {e}"))?;

        // Inject host_call function for capabilities
        let host_call_fn =
            rquickjs::Function::new(ctx.clone(), |cap: String, method: String, payload: String| -> String {
                let payload_bytes = payload.as_bytes();
                match call_host_capability(&cap, &method, payload_bytes) {
                    Ok(result) => String::from_utf8_lossy(&result).to_string(),
                    Err(e) => format!("{{\"error\": \"{}\"}}", e.replace('"', "\\\"")),
                }
            })
            .map_err(|e| format!("Failed to create host_call function: {e}"))?;

        globals
            .set("__sandcastle_host_call", host_call_fn)
            .map_err(|e| format!("Failed to set host_call function: {e}"))?;

        // Inject fs functions for artifacts
        let read_artifact_fn =
            rquickjs::Function::new(ctx.clone(), |name: String| -> rquickjs::Result<Option<String>> {
                Ok(read_artifact(&name).map(|data| String::from_utf8_lossy(&data).to_string()))
            })
            .map_err(|e| format!("Failed to create read_artifact function: {e}"))?;

        let write_artifact_fn =
            rquickjs::Function::new(ctx.clone(), |name: String, data: String| -> bool {
                write_artifact(&name, data.as_bytes())
            })
            .map_err(|e| format!("Failed to create write_artifact function: {e}"))?;

        globals
            .set("__sandcastle_read_artifact", read_artifact_fn)
            .map_err(|e| format!("Failed to set read_artifact function: {e}"))?;
        globals
            .set("__sandcastle_write_artifact", write_artifact_fn)
            .map_err(|e| format!("Failed to set write_artifact function: {e}"))?;

        // Inject input and host API bridge
        let bridge_code = format!(
            r#"
            globalThis.__sandcastle_input = {input_json};

            // Create the host:api import bridge
            globalThis.__sandcastle_modules = {{}};

            globalThis.__sandcastle_register_module = function(name, methods) {{
                globalThis.__sandcastle_modules[name] = {{}};
                for (const method of methods) {{
                    globalThis.__sandcastle_modules[name][method] = function(...args) {{
                        const payload = JSON.stringify(args);
                        const result = __sandcastle_host_call(name, method, payload);
                        try {{
                            return JSON.parse(result);
                        }} catch(e) {{
                            return result;
                        }}
                    }};
                }}
            }};

            // Virtual fs module
            globalThis.__sandcastle_fs = {{
                readFile: function(path, encoding) {{
                    const name = path.startsWith('/') ? path.slice(1) : path;
                    const data = __sandcastle_read_artifact(name);
                    if (data === null || data === undefined) {{
                        throw new Error("File not found: " + path);
                    }}
                    return data;
                }},
                writeFile: function(path, data) {{
                    const name = path.startsWith('/output/') ? path.slice(8) : path.startsWith('/') ? path.slice(1) : path;
                    return __sandcastle_write_artifact(name, typeof data === 'string' ? data : JSON.stringify(data));
                }}
            }};
            "#
        );

        ctx.eval::<(), _>(bridge_code.as_str())
            .map_err(|e| format!("Failed to set up bridge: {e}"))?;

        // Wrap user code to capture the result
        // Transform import statements to use our bridge
        let wrapped_code = format!(
            r#"
            (function() {{
                // Simple import resolution for host:api and host:fs
                const host_api = globalThis.__sandcastle_modules["host:api"] || {{}};
                const host_fs = globalThis.__sandcastle_fs;

                // Make input available
                const input = globalThis.__sandcastle_input;

                // User code result
                let __result;
                try {{
                    __result = (function() {{
                        {code}
                    }})();
                }} catch(e) {{
                    throw e;
                }}
                return __result;
            }})()
            "#
        );

        let result: rquickjs::Value = ctx
            .eval(wrapped_code.as_str())
            .map_err(|e| format!("JavaScript error: {e}"))?;

        // Convert rquickjs Value to serde_json Value
        value_to_json(&ctx, &result)
    })
}

/// Convert a rquickjs Value to a serde_json Value.
fn value_to_json<'js>(
    ctx: &rquickjs::Ctx<'js>,
    val: &rquickjs::Value<'js>,
) -> Result<serde_json::Value, String> {
    if val.is_undefined() || val.is_null() {
        Ok(serde_json::Value::Null)
    } else if val.is_bool() {
        Ok(serde_json::Value::Bool(
            val.as_bool().unwrap_or(false),
        ))
    } else if let Some(n) = val.as_int() {
        Ok(serde_json::Value::Number(n.into()))
    } else if let Some(n) = val.as_float() {
        match serde_json::Number::from_f64(n) {
            Some(num) => Ok(serde_json::Value::Number(num)),
            None => Ok(serde_json::Value::Null),
        }
    } else if let Some(s) = val.as_string() {
        let s = s.to_string().map_err(|e| format!("String conversion error: {e}"))?;
        Ok(serde_json::Value::String(s))
    } else {
        // For objects and arrays, serialize via JSON.stringify in QuickJS
        let json_stringify: rquickjs::Function = ctx
            .globals()
            .get::<_, rquickjs::Object>("JSON")
            .map_err(|e| format!("JSON global not found: {e}"))?
            .get("stringify")
            .map_err(|e| format!("JSON.stringify not found: {e}"))?;

        let json_str: String = json_stringify
            .call((val.clone(),))
            .map_err(|e| format!("JSON.stringify failed: {e}"))?;

        serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {e}"))
    }
}
