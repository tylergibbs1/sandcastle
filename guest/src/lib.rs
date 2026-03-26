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
    // Two-pass protocol: first call with a small buffer to get the actual size,
    // then allocate exactly what's needed if the small buffer wasn't enough.
    let mut initial_buf = vec![0u8; 8 * 1024]; // 8KB initial buffer (covers most responses)
    let result_len = unsafe {
        host_call(
            capability.as_ptr(),
            capability.len() as i32,
            method.as_ptr(),
            method.len() as i32,
            payload.as_ptr(),
            payload.len() as i32,
            initial_buf.as_mut_ptr(),
            initial_buf.len() as i32,
        )
    };

    if result_len < 0 {
        let error_len = (-result_len) as usize;
        let error_msg =
            String::from_utf8_lossy(&initial_buf[..error_len.min(initial_buf.len())]).into_owned();
        Err(error_msg)
    } else {
        let needed = result_len as usize;
        if needed <= initial_buf.len() {
            initial_buf.truncate(needed);
            Ok(initial_buf)
        } else {
            // Response was larger than initial buffer — retry with exact size
            let mut large_buf = vec![0u8; needed];
            let retry_len = unsafe {
                host_call(
                    capability.as_ptr(),
                    capability.len() as i32,
                    method.as_ptr(),
                    method.len() as i32,
                    payload.as_ptr(),
                    payload.len() as i32,
                    large_buf.as_mut_ptr(),
                    large_buf.len() as i32,
                )
            };
            if retry_len < 0 {
                let error_len = (-retry_len) as usize;
                let error_msg = String::from_utf8_lossy(&large_buf[..error_len.min(large_buf.len())]).into_owned();
                Err(error_msg)
            } else {
                large_buf.truncate(retry_len as usize);
                Ok(large_buf)
            }
        }
    }
}

fn read_artifact(name: &str) -> Option<Vec<u8>> {
    // Two-pass protocol: first call with a small buffer to get the actual size,
    // then allocate exactly what's needed.
    let mut initial_buf = vec![0u8; 8 * 1024]; // 8KB initial buffer
    let len = unsafe {
        host_read_artifact(
            name.as_ptr(),
            name.len() as i32,
            initial_buf.as_mut_ptr(),
            initial_buf.len() as i32,
        )
    };

    if len < 0 {
        None
    } else {
        let needed = len as usize;
        if needed <= initial_buf.len() {
            initial_buf.truncate(needed);
            Some(initial_buf)
        } else {
            // Artifact is larger than initial buffer — retry with exact size
            let mut large_buf = vec![0u8; needed];
            let retry_len = unsafe {
                host_read_artifact(
                    name.as_ptr(),
                    name.len() as i32,
                    large_buf.as_mut_ptr(),
                    large_buf.len() as i32,
                )
            };
            if retry_len < 0 {
                None
            } else {
                large_buf.truncate(retry_len as usize);
                Some(large_buf)
            }
        }
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

    // Lower the GC threshold to reduce fragmentation under memory pressure.
    // This triggers more frequent garbage collection, helping prevent
    // memory fragmentation that could lead to premature OOM.
    rt.set_gc_threshold(256 * 1024);

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

        // Polyfill Web APIs that QuickJS doesn't provide natively.
        // These are declared in sdk/typescript/src/guest/index.d.ts and
        // expected by user code.
        let polyfills = r#"
            // --- atob / btoa (Base64) ---
            if (typeof globalThis.btoa === 'undefined') {
                const _b64 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/=';
                globalThis.btoa = function(str) {
                    let out = '';
                    for (let i = 0; i < str.length; i += 3) {
                        const a = str.charCodeAt(i);
                        const b = i + 1 < str.length ? str.charCodeAt(i + 1) : 0;
                        const c = i + 2 < str.length ? str.charCodeAt(i + 2) : 0;
                        out += _b64[a >> 2] + _b64[((a & 3) << 4) | (b >> 4)];
                        out += i + 1 < str.length ? _b64[((b & 15) << 2) | (c >> 6)] : '=';
                        out += i + 2 < str.length ? _b64[c & 63] : '=';
                    }
                    return out;
                };
                globalThis.atob = function(str) {
                    let out = '';
                    str = str.replace(/=+$/, '');
                    for (let i = 0; i < str.length; i += 4) {
                        const a = _b64.indexOf(str[i]);
                        const b = _b64.indexOf(str[i + 1]);
                        const c = _b64.indexOf(str[i + 2]);
                        const d = _b64.indexOf(str[i + 3]);
                        out += String.fromCharCode((a << 2) | (b >> 4));
                        if (c !== -1 && c !== 64) out += String.fromCharCode(((b & 15) << 4) | (c >> 2));
                        if (d !== -1 && d !== 64) out += String.fromCharCode(((c & 3) << 6) | d);
                    }
                    return out;
                };
            }

            // --- TextEncoder / TextDecoder (UTF-8) ---
            if (typeof globalThis.TextEncoder === 'undefined') {
                globalThis.TextEncoder = class TextEncoder {
                    get encoding() { return 'utf-8'; }
                    encode(str) {
                        const buf = [];
                        for (let i = 0; i < str.length; i++) {
                            let c = str.charCodeAt(i);
                            if (c < 0x80) { buf.push(c); }
                            else if (c < 0x800) { buf.push(0xC0 | (c >> 6), 0x80 | (c & 0x3F)); }
                            else if (c >= 0xD800 && c <= 0xDBFF && i + 1 < str.length) {
                                const next = str.charCodeAt(++i);
                                c = ((c - 0xD800) << 10) + (next - 0xDC00) + 0x10000;
                                buf.push(0xF0 | (c >> 18), 0x80 | ((c >> 12) & 0x3F), 0x80 | ((c >> 6) & 0x3F), 0x80 | (c & 0x3F));
                            }
                            else { buf.push(0xE0 | (c >> 12), 0x80 | ((c >> 6) & 0x3F), 0x80 | (c & 0x3F)); }
                        }
                        return new Uint8Array(buf);
                    }
                };
                globalThis.TextDecoder = class TextDecoder {
                    constructor(label) { this._label = label || 'utf-8'; }
                    get encoding() { return this._label; }
                    decode(buf) {
                        const bytes = new Uint8Array(buf.buffer || buf);
                        let str = '', i = 0;
                        while (i < bytes.length) {
                            let c = bytes[i++];
                            if (c < 0x80) { str += String.fromCharCode(c); }
                            else if (c < 0xE0) { str += String.fromCharCode(((c & 0x1F) << 6) | (bytes[i++] & 0x3F)); }
                            else if (c < 0xF0) { str += String.fromCharCode(((c & 0x0F) << 12) | ((bytes[i++] & 0x3F) << 6) | (bytes[i++] & 0x3F)); }
                            else {
                                const cp = ((c & 0x07) << 18) | ((bytes[i++] & 0x3F) << 12) | ((bytes[i++] & 0x3F) << 6) | (bytes[i++] & 0x3F);
                                str += String.fromCodePoint(cp);
                            }
                        }
                        return str;
                    }
                };
            }

            // --- URL / URLSearchParams ---
            if (typeof globalThis.URL === 'undefined') {
                globalThis.URLSearchParams = class URLSearchParams {
                    constructor(init) {
                        this._params = [];
                        if (typeof init === 'string') {
                            const s = init.startsWith('?') ? init.slice(1) : init;
                            if (s) for (const pair of s.split('&')) {
                                const [k, ...v] = pair.split('=');
                                this._params.push([decodeURIComponent(k), decodeURIComponent(v.join('='))]);
                            }
                        }
                    }
                    get(key) { const p = this._params.find(([k]) => k === key); return p ? p[1] : null; }
                    set(key, value) { const i = this._params.findIndex(([k]) => k === key); if (i >= 0) this._params[i][1] = String(value); else this._params.push([key, String(value)]); }
                    has(key) { return this._params.some(([k]) => k === key); }
                    delete(key) { this._params = this._params.filter(([k]) => k !== key); }
                    toString() { return this._params.map(([k, v]) => encodeURIComponent(k) + '=' + encodeURIComponent(v)).join('&'); }
                    entries() { return this._params[Symbol.iterator](); }
                    [Symbol.iterator]() { return this._params[Symbol.iterator](); }
                };
                globalThis.URL = class URL {
                    constructor(url, base) {
                        if (base) {
                            const b = new URL(base);
                            if (url.startsWith('/')) url = b.origin + url;
                            else if (!url.match(/^[a-z]+:\/\//i)) url = b.href.replace(/\/[^\/]*$/, '/') + url;
                        }
                        const m = url.match(/^([a-z][a-z0-9+\-.]*):\/\/([^/?#]*)([^?#]*)(\?[^#]*)?(#.*)?$/i);
                        if (!m) throw new TypeError("Invalid URL: " + url);
                        this.protocol = m[1] + ':';
                        const hostPort = m[2];
                        const portMatch = hostPort.match(/:(\d+)$/);
                        this.hostname = portMatch ? hostPort.slice(0, -portMatch[0].length) : hostPort;
                        this.port = portMatch ? portMatch[1] : '';
                        this.host = this.port ? this.hostname + ':' + this.port : this.hostname;
                        this.pathname = m[3] || '/';
                        this.search = m[4] || '';
                        this.hash = m[5] || '';
                        this.searchParams = new URLSearchParams(this.search);
                        this.origin = this.protocol + '//' + this.host;
                        this.href = this.origin + this.pathname + this.search + this.hash;
                    }
                    toString() { return this.href; }
                };
            }

            // --- crypto ---
            if (typeof globalThis.crypto === 'undefined') {
                globalThis.crypto = {
                    getRandomValues(array) {
                        for (let i = 0; i < array.length; i++) {
                            array[i] = Math.floor(Math.random() * 256);
                        }
                        return array;
                    },
                    randomUUID() {
                        const h = '0123456789abcdef';
                        const s = (n) => { let r = ''; for (let i = 0; i < n; i++) r += h[Math.floor(Math.random() * 16)]; return r; };
                        return s(8) + '-' + s(4) + '-4' + s(3) + '-' + h[8 + Math.floor(Math.random() * 4)] + s(3) + '-' + s(12);
                    }
                };
            }
        "#;

        ctx.eval::<(), _>(polyfills)
            .map_err(|e| format!("Failed to set up polyfills: {e}"))?;

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
        // On error, this throws a JS exception so the guest can't silently ignore quota errors
        let host_call_fn =
            rquickjs::Function::new(ctx.clone(), |ctx: rquickjs::Ctx<'_>, cap: String, method: String, payload: String| -> rquickjs::Result<String> {
                let payload_bytes = payload.as_bytes();
                match call_host_capability(&cap, &method, payload_bytes) {
                    Ok(result) => Ok(String::from_utf8_lossy(&result).into_owned()),
                    Err(e) => Err(ctx.throw(rquickjs::Value::from_string(
                        rquickjs::String::from_str(ctx.clone(), &e)
                            .map_err(|_| rquickjs::Error::Unknown)?
                    ).into())),
                }
            })
            .map_err(|e| format!("Failed to create host_call function: {e}"))?;

        globals
            .set("__sandcastle_host_call", host_call_fn)
            .map_err(|e| format!("Failed to set host_call function: {e}"))?;

        // Inject fs functions for artifacts
        let read_artifact_fn =
            rquickjs::Function::new(ctx.clone(), |name: String| -> rquickjs::Result<Option<String>> {
                Ok(read_artifact(&name).map(|data| String::from_utf8_lossy(&data).into_owned()))
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

        // Wrap user code in an async IIFE so that:
        // 1. Top-level `return` works
        // 2. The microtask queue is flushed before the result is captured
        // 3. If the user returns a Promise, we await it
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

        // Flush the microtask/Promise job queue so that Promise.resolve().then(...)
        // and async/await patterns complete before we capture the return value.
        while ctx.execute_pending_job() {}

        // If the result is a Promise, try to extract its resolved value
        let result = resolve_promise_if_needed(&ctx, &result)?;

        // Convert rquickjs Value to serde_json Value
        value_to_json(&ctx, &result)
    })
}

/// If the value is a Promise, drain the job queue and extract the resolved value.
/// Returns the original value unchanged if it's not a Promise.
fn resolve_promise_if_needed<'js>(
    ctx: &rquickjs::Ctx<'js>,
    val: &rquickjs::Value<'js>,
) -> Result<rquickjs::Value<'js>, String> {
    // Check if the value is a Promise by looking for a .then method
    if let Some(obj) = val.as_object() {
        if let Ok(then_fn) = obj.get::<_, rquickjs::Value>("then") {
            if then_fn.is_function() {
                // It's a thenable/Promise — set up resolution capture
                let globals = ctx.globals();
                globals.set("__sandcastle_promise_result", rquickjs::Value::new_undefined(ctx.clone()))
                    .map_err(|e| format!("Failed to set promise result global: {e}"))?;
                globals.set("__sandcastle_promise_error", rquickjs::Value::new_undefined(ctx.clone()))
                    .map_err(|e| format!("Failed to set promise error global: {e}"))?;

                let capture_code = r#"
                    (function(p) {
                        p.then(
                            function(v) { globalThis.__sandcastle_promise_result = v; },
                            function(e) { globalThis.__sandcastle_promise_error = e; }
                        );
                    })
                "#;
                let capture_fn: rquickjs::Function = ctx.eval(capture_code)
                    .map_err(|e| format!("Failed to create promise capture: {e}"))?;
                capture_fn.call::<_, ()>((val.clone(),))
                    .map_err(|e| format!("Failed to attach promise handlers: {e}"))?;

                // Drain the job queue to let the promise resolve
                while ctx.execute_pending_job() {}

                // Check if the promise rejected
                let error_val: rquickjs::Value = globals.get("__sandcastle_promise_error")
                    .map_err(|e| format!("Failed to get promise error: {e}"))?;
                if !error_val.is_undefined() && !error_val.is_null() {
                    let msg = if let Some(s) = error_val.as_string() {
                        s.to_string().unwrap_or_else(|_| "Promise rejected".to_string())
                    } else {
                        "Promise rejected".to_string()
                    };
                    return Err(format!("JavaScript error: {msg}"));
                }

                // Get the resolved value
                let resolved_val: rquickjs::Value = globals.get("__sandcastle_promise_result")
                    .map_err(|e| format!("Failed to get promise result: {e}"))?;
                if !resolved_val.is_undefined() {
                    return Ok(resolved_val);
                }
            }
        }
    }
    Ok(val.clone())
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
