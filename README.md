<p align="center">
  <img src="logo.svg" alt="SandCastle" width="200" />
</p>

<h1 align="center">SandCastle</h1>

<p align="center">Lightweight WASM-based sandbox runtime for AI agent code execution.</p>

SandCastle lets AI agents execute JavaScript in secure, isolated sandboxes with sub-millisecond cold starts and <2MB memory per sandbox. It uses WebAssembly (Wasmtime) as the isolation layer and QuickJS as the JavaScript engine.

```
Benchmarked (Apple Silicon, release mode):
  Sandbox creation:     600µs per op (1,670 ops/sec)
  Simple expression:    600µs per op (1,670 ops/sec)
  JSON processing:      1.5ms per op (100 items filter+map)
  1000 concurrent:      105ms total (105µs per sandbox)
  Sustained throughput: 1,700 ops/sec (stable over 5s)
  Peak memory:          ~1.3MB per sandbox
  Guest WASM module:    ~852KB
  Tail latency (p99):   671µs (p99/p50 = 1.08x)
```

## Why

AI agents need to run code. The options are containers (slow), V8 isolates (heavy), or `eval` (insecure). SandCastle is a different tradeoff — WASM-based sandboxes that are faster and lighter than both.

| Solution | Startup | Memory | Binary/Runtime | JS Compat | Security |
|----------|---------|--------|----------------|-----------|----------|
| Docker | ~500ms | ~100MB+ | Docker daemon | Full Node.js | Namespace isolation |
| E2B | ~100-200ms | ~512MB min | Hosted / KVM | Full Node.js | Firecracker |
| V8 isolate (self-hosted) | ~3-5ms | ~5MB | ~50MB (V8 lib) | Full ES2024+ | V8 isolate boundary |
| Cloudflare Workers | ~3ms | ~5MB | Cloudflare only | Full ES2024+ | V8 isolates |
| **SandCastle** | **<1ms** | **~1.3MB** | **~852KB WASM** | **ES2024+ (QuickJS-NG)** | **WASM spec boundary** |

### Containers vs SandCastle

Containers boot an entire OS kernel to run `return 1 + 1`. That's 100-500ms startup and 100MB+ memory — fine for long-running services, but AI agents make 10-50 tool calls per conversation. At 500ms per sandbox, that's 5-25 seconds of waiting. At 600µs, it's 6-30ms.

### V8 isolates vs SandCastle

V8 isolates are the closest alternative — they're in-process and don't need containers. The tradeoffs:

- **V8 wins on JIT performance** — faster for compute-heavy loops due to JIT compilation
- **SandCastle wins on startup** (0.6ms vs 3-5ms), **memory** (1.3MB vs 5MB), **binary size** (852KB vs ~50MB for the V8 library), and **embedding simplicity** (Wasmtime's API is small and clean; V8's is notoriously complex)
- **JS compatibility is now comparable** — SandCastle uses QuickJS-NG which supports ES2024+ including `Object.groupBy`, `Promise.withResolvers`, `Array.fromAsync`, `Set` methods, iterator helpers, and more

If you need Node.js APIs or heavy compute (JIT matters), use V8. If you need fast, lightweight sandboxes for AI agent code — data transforms, API orchestration, JSON processing — SandCastle is purpose-built for that.

### How the sandbox works

QuickJS (a C JavaScript engine) compiles to WASM and runs *inside* the sandbox. Guest code has zero access to the host — no filesystem, no network, no syscalls. Everything goes through explicit host function imports that you control and meter. The sandbox boundary is the WASM spec itself.

## Quick Start

### Scaffold a new project

```bash
sandcastle init my-project
cd my-project
sandcastle run scripts/hello.js --input '{"name": "Alice"}'
```

### Rust (library mode)

```rust
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;

let guest_module = std::fs::read("guest-js.wasm")?;
let runtime = SandCastle::new(Config::new(guest_module))?;

let result = runtime.execute(
    ExecutionRequest::new("return 1 + 1;")
).await?;

println!("{:?}", result.output); // Json(2)
```

### With secrets and streaming

```rust
use sandcastle::sandbox::ExecutionRequest;

let result = runtime.execute(
    ExecutionRequest::new(r#"
        const key = process.env.API_KEY;
        console.log("Using key:", key.slice(0, 8) + "...");
        return { authenticated: key.length > 0 };
    "#)
    .with_env("API_KEY", "sk-live-abc123")
    .with_console_callback(|level, msg| {
        println!("[{:?}] {}", level, msg);  // Real-time streaming
    })
).await?;
```

### Persistent KV (data survives restarts)

```rust
use sandcastle::capabilities::PersistentKvCapability;
use sandcastle::capability::CapabilityRegistry;

let kv = PersistentKvCapability::open("agent_memory.db").unwrap();
let mut caps = CapabilityRegistry::new();
caps.register(Box::new(kv));

// Guest code can now use: __sandcastle_host_call("kv", "set", '{"key":"memory","value":"..."}')
// Data persists across process restarts
```

### TypeScript SDK

```bash
bun add @grayhaven/sandcastle  # or npm install @grayhaven/sandcastle
```

```typescript
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");
// result === 2
```

### CLI

```bash
# Run a script
sandcastle run script.js --input '{"name": "Alice"}'

# Interactive REPL
sandcastle repl

# Generate TypeScript declarations from capability definitions
sandcastle codegen capabilities.json -o types.d.ts
```

### HTTP Server

```bash
# Start server with hot-reload from a scripts directory
sandcastle serve --http 0.0.0.0:8080 --watch ./scripts

# Execute code
curl -X POST http://localhost:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{"code": "return input.x * 2", "input": {"x": 21}}'

# Register a named script
curl -X POST http://localhost:8080/scripts \
  -H 'Content-Type: application/json' \
  -d '{"name": "doubler", "code": "return globalThis.__sandcastle_input.x * 2"}'

# Dispatch to it
curl -X POST http://localhost:8080/dispatch/doubler \
  -H 'Content-Type: application/json' \
  -d '{"input": {"x": 21}}'

# Multi-tenant namespaces
curl -X POST http://localhost:8080/namespaces \
  -H 'Content-Type: application/json' \
  -d '{"name": "tenant-abc"}'

curl -X POST http://localhost:8080/namespaces/tenant-abc/scripts \
  -H 'Content-Type: application/json' \
  -d '{"name": "worker", "code": "return globalThis.__sandcastle_input.x + 1"}'

curl -X POST http://localhost:8080/namespaces/tenant-abc/dispatch/worker \
  -H 'Content-Type: application/json' \
  -d '{"input": {"x": 41}}'
```

## Features

### Core Runtime
- **Sub-millisecond sandbox creation** — 600µs benchmarked via Wasmtime AOT compilation
- **Fuel metering** — deterministic instruction count caps (identical fuel across runs)
- **Epoch-based timeouts** — wall-clock deadline enforcement (~2ms precision)
- **Memory protection** — `trap_on_grow_failure` + `MemoryExceeded` status (not opaque traps)
- **Execution transcripts** — structured logs with console output, capability calls, fuel/memory usage
- **Promise/async support** — `return Promise.resolve(42)` and `return asyncFn()` resolve correctly
- **Web API polyfills** — `TextEncoder`/`TextDecoder`, `URL`/`URLSearchParams`, `atob`/`btoa`, `crypto.randomUUID`/`getRandomValues`, `setTimeout`, `structuredClone`, `performance.now`, `fetch()`
- **Module shims** — `require('lodash')`, `require('path')`, `require('uuid')`, `require('date-fns')`, `require('qs')` work out of the box with lightweight implementations
- **Streaming output** — `on_console` callback for real-time `console.log` delivery
- **Persistent sandboxes** — `create_persistent_sandbox()` for multi-turn agent conversations
- **LLM-friendly errors** — `require('express')` explains "this is a SandCastle sandbox" with available alternatives; `process.env` and `module.exports` stubs prevent crashes
- **Better error messages** — JS errors surface the actual error text, not just "error code 1"

### Host Capabilities
- **Typed capability bridge** — expose host APIs to sandboxed code with quota enforcement
- **Built-in KV store** — in-memory key-value storage (`DashMap`-backed), shareable across sandboxes
- **Persistent KV store** — SQLite-backed KV that survives process restarts (`--features persistent-kv`)
- **Built-in HTTP client** — real `reqwest`-backed HTTP with domain allowlists and response size caps
- **Secret/env injection** — `.with_env("API_KEY", "sk-...")` injects into `process.env` securely
- **Per-capability quotas** — max calls, payload size, transfer limits, concurrency caps (lock-free atomics)
- **Quota enforcement throws JS exceptions** — guest code can't silently ignore quota exhaustion

### Multi-Tenant Dispatch
- **Script registry** — pre-register named scripts, dispatch by name
- **Dispatch namespaces** — Cloudflare Workers for Platforms-style multi-tenant isolation
- **Per-namespace concurrency** — independent resource limits per tenant
- **Hot reload** — `--watch` flag auto-registers scripts on file changes

### Code Mode
- **Replace N tool calls with 1 code execution** — up to 80% token reduction
- **`createCodeTool()`** — converts tool definitions into a single LLM tool
- **`TwoPassExecutor`** — collect tool calls in sandbox, execute host-side, replay with results
- **`generateTypes()`** — auto-generate TypeScript declarations from tool schemas for LLM context

### TypeScript SDK (`@grayhaven/sandcastle`)
- **ESM-first**, zero runtime dependencies, Bun toolchain
- **Typed errors** — `TimeoutError`, `GuestError`, `FuelExhaustedError`, `MemoryExceededError`
- **Subprocess + HTTP modes** — spawn CLI or talk to server
- **Namespace client** — `sc.namespace("tenant").dispatch("worker", input)`
- **Guest type declarations** — feed to your LLM so it knows the sandbox API

### CLI
- **`sandcastle run`** — execute a script
- **`sandcastle serve`** — HTTP server with REST API + hot reload
- **`sandcastle init`** — scaffold a new project
- **`sandcastle repl`** — interactive JS REPL with multi-line support
- **`sandcastle codegen`** — generate TypeScript declarations from capability definitions
- **`sandcastle info`** — print runtime info

## Code Mode

Code Mode replaces sequential tool calls with a single code execution. Instead of the LLM making N separate `tool_use` calls (N round-trips), it writes one function that chains all N calls — cutting token usage by up to 80%.

This is SandCastle's answer to [Cloudflare's Code Mode](https://blog.cloudflare.com/sandboxing-ai-agents-100x-faster/).

```typescript
import { createCodeTool, TwoPassExecutor } from "sandcastle/codemode";

const tools = [
  {
    name: "getUser",
    description: "Get user by ID",
    inputSchema: { type: "object", properties: { id: { type: "number" } }, required: ["id"] },
    execute: async ({ id }) => db.getUser(id),
  },
  {
    name: "sendEmail",
    description: "Send an email",
    inputSchema: { type: "object", properties: { to: { type: "string" }, body: { type: "string" } }, required: ["to", "body"] },
    execute: async (input) => mailer.send(input),
  },
];

const executor = new TwoPassExecutor();
const codemode = createCodeTool({ tools, executor });

// Give `codemode` to your LLM as a single tool.
// Claude writes code like:
//   async () => {
//     const user = await codemode.getUser({ id: 42 });
//     await codemode.sendEmail({ to: user.email, body: "Hello!" });
//     return { sent: true };
//   }
```

**How the TwoPassExecutor works:**
1. **Pass 1**: Run the code in a sandbox with a collector proxy — `codemode.*` calls are recorded
2. **Pass 2**: Execute recorded tool calls host-side with real implementations
3. **Pass 3**: Re-run the code with results pre-populated so it completes

## Using with AI Agents

SandCastle is designed to be a tool in an AI agent's toolkit:

```typescript
import { SandCastle } from "@grayhaven/sandcastle";

const sandbox = new SandCastle();

// Simple mode: LLM writes + executes code directly
const tool = {
  name: "run_code",
  description: "Execute JavaScript in a secure sandbox",
  execute: async ({ code, input }) => {
    const result = await sandbox.execute({ code, input });
    if (result.ok) return JSON.stringify(result.output.value);
    return `Error: ${result.status.message}`;
  },
};

// Code Mode: LLM writes code that chains multiple tool calls
import { createCodeTool, TwoPassExecutor } from "sandcastle/codemode";
const codemode = createCodeTool({ tools: myTools, executor: new TwoPassExecutor() });
// Give `codemode` to your LLM — it replaces all of `myTools` with a single tool
```

## Architecture

See [`docs/architecture.md`](docs/architecture.md) for detailed Mermaid diagrams.

```
Host Application
  │
  ├── SandCastle Runtime (Wasmtime engine, AOT-compiled module)
  │     ├── Sandbox (Wasmtime Store + QuickJS WASM)
  │     ├── Script Registry (named, pre-compiled scripts)
  │     └── Dispatch Namespaces (multi-tenant isolation)
  │
  ├── Host Capabilities (KV, HTTP, custom)
  │
  └── Delivery: Library | CLI | HTTP Server
```

## Building from Source

```bash
# Prerequisites: Rust 1.82+, wasm32-wasip1 target
rustup target add wasm32-wasip1

# Build everything
make build

# Or step by step:
cd guest && ./build.sh          # Build QuickJS WASM guest (~823KB)
cargo build --release            # Build runtime + CLI

# Run tests
cargo test                       # 54 Rust tests
cd sdk/typescript && bun test    # 118 TypeScript tests

# Run benchmarks
cargo bench -p sandcastle
```

## Project Structure

```
sandcastle/
├── crates/
│   ├── sandcastle/          # Core library
│   │   └── src/
│   │       ├── runtime.rs       # Wasmtime engine + module management
│   │       ├── sandbox.rs       # Sandbox lifecycle + host functions
│   │       ├── capability.rs    # Host capability trait + bridge
│   │       ├── capabilities/    # Built-in KV + HTTP capabilities
│   │       ├── registry.rs      # Named script registry
│   │       ├── namespace.rs     # Dispatch namespaces
│   │       ├── transcript.rs    # Execution transcript + replay
│   │       ├── pool.rs          # Warm pool metrics
│   │       ├── limits.rs        # Resource limit types
│   │       ├── types.rs         # Shared types
│   │       └── error.rs         # Error hierarchy
│   ├── sandcastle-cli/      # CLI (run, serve, init, repl, codegen)
│   └── sandcastle-macros/   # #[sandcastle::capability] proc macro
├── guest/                   # QuickJS WASM guest runtime
├── sdk/typescript/          # TypeScript SDK
│   ├── src/
│   │   ├── client.ts            # SandCastle class
│   │   ├── codemode/            # Code Mode SDK
│   │   ├── core/                # Errors, subprocess, HTTP transport
│   │   ├── types/               # Public type definitions
│   │   └── guest/index.d.ts     # Guest-side type declarations
│   └── test/                # 204 tests (unit + integration + agent)
├── docs/                    # Architecture diagrams (Mermaid)
└── examples/                # Example scripts
```

## Security Model

The sandbox boundary exists between untrusted guest code and the trusted host. Guest code has:
- **No network access** — HTTP is a mediated host capability
- **No filesystem access** — artifacts are virtual, mounted by the host
- **No ambient authority** — every external effect goes through typed, quota-enforced capabilities
- **Instruction limits** — fuel metering prevents infinite loops
- **Memory limits** — per-sandbox memory caps enforced by Wasmtime
- **Wall-clock timeouts** — epoch-based interruption
- **Input validation** — all guest-to-host parameters validated and capped at 16MB

## License

Apache 2.0
