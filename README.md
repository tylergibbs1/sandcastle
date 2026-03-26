<p align="center">
  <img src="logo.svg" alt="SandCastle" width="200" />
</p>

<h1 align="center">SandCastle</h1>

<p align="center">Lightweight WASM-based sandbox runtime for AI agent code execution.</p>

SandCastle lets AI agents execute JavaScript in secure, isolated sandboxes with sub-5ms cold starts and <8MB memory per sandbox. It uses WebAssembly (Wasmtime) as the isolation layer and QuickJS as the JavaScript engine.

```
Sandbox creation:  <1ms p50, <5ms p99
Memory per sandbox: <8MB baseline
Guest WASM module:  ~823KB
```

## Why

AI agents need to run code. The alternatives are slow (Docker ~500ms), platform-locked (Cloudflare Workers), or insecure (eval). SandCastle gives you fast, portable, secure sandboxes you can embed anywhere.

| Solution | Startup | Memory | Portable | Security |
|----------|---------|--------|----------|----------|
| Docker | ~500ms | ~100MB+ | Yes | Namespace isolation |
| Cloudflare Workers | ~3ms | ~5MB | Cloudflare only | V8 isolates |
| E2B | ~500ms | ~100MB+ | Hosted only | Firecracker |
| **SandCastle** | **<5ms** | **<8MB** | **Anywhere** | **WASM sandbox** |

## Quick Start

### Rust (library mode)

```rust
use sandcastle::runtime::{Config, SandCastle};
use sandcastle::sandbox::ExecutionRequest;

let guest_module = std::fs::read("guest-js.wasm")?;
let runtime = SandCastle::new(Config::new(guest_module))?;

let result = runtime.execute(
    ExecutionRequest::new("return 1 + 1;")
).await?;

assert_eq!(result.output, OutputValue::Json(serde_json::json!(2)));
```

### TypeScript SDK

```bash
bun add sandcastle  # or npm install sandcastle
```

```typescript
import { SandCastle } from "sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");
// result === 2
```

### CLI

```bash
sandcastle run script.js --input '{"name": "Alice"}'
```

### HTTP Server

```bash
sandcastle serve --http 0.0.0.0:8080

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
```

## Features

### Core Runtime
- **Sub-millisecond sandbox creation** via Wasmtime AOT compilation
- **Fuel metering** — cap instruction count per execution
- **Epoch-based timeouts** — wall-clock deadline enforcement
- **Memory limits** — per-sandbox memory caps
- **Execution transcripts** — structured logs of every execution with console output, capability calls, fuel/memory usage

### Host Capabilities
- **Typed capability bridge** — expose host APIs to sandboxed code with quota enforcement
- **Built-in KV store** — in-memory key-value storage (`DashMap`-backed)
- **Built-in HTTP client** — real `reqwest`-backed HTTP with domain allowlists
- **Per-capability quotas** — max calls, payload size, transfer limits, concurrency caps

### Multi-Tenant Dispatch
- **Script registry** — pre-register named scripts, dispatch by name
- **Dispatch namespaces** — Cloudflare-style multi-tenant isolation
- **Per-namespace concurrency** — independent resource limits per tenant

### TypeScript SDK
- **ESM-first**, zero runtime dependencies
- **Typed errors** — `TimeoutError`, `GuestError`, `FuelExhaustedError`, etc.
- **Subprocess + HTTP modes** — spawn CLI or talk to server
- **Namespace client** — `sc.namespace("tenant").dispatch("worker", input)`
- **Guest type declarations** — feed to your LLM so it knows the sandbox API

## Architecture

```
Host Application
  │
  ├── SandCastle Runtime (Wasmtime engine, AOT-compiled module)
  │     │
  │     ├── Sandbox (Wasmtime Store + QuickJS WASM)
  │     │     ├── Guest JS code (interpreted by QuickJS)
  │     │     ├── Host capability bridge (MessagePack RPC)
  │     │     └── Virtual filesystem (input/output artifacts)
  │     │
  │     ├── Script Registry (named, pre-compiled scripts)
  │     └── Dispatch Namespaces (multi-tenant isolation)
  │
  ├── Host Capabilities
  │     ├── KV Store (DashMap)
  │     ├── HTTP Client (reqwest)
  │     └── Custom (implement the Capability trait)
  │
  └── HTTP Server (axum) or CLI or Library embed
```

## Code Mode

Code Mode replaces sequential tool calls with a single code execution. Instead of the LLM making N separate `tool_use` calls (N round-trips), it writes one function that chains all N calls — cutting token usage by up to 80%.

This is SandCastle's answer to [Cloudflare's Code Mode](https://blog.cloudflare.com/sandboxing-ai-agents-100x-faster/).

```typescript
import { createCodeTool, TwoPassExecutor } from "sandcastle/codemode";

// 1. Define your tools
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

// 2. Create the code tool
const executor = new TwoPassExecutor();
const codemode = createCodeTool({ tools, executor });

// 3. Give it to your LLM as a single tool
// Claude writes code like:
//   async () => {
//     const user = await codemode.getUser({ id: 42 });
//     await codemode.sendEmail({ to: user.email, body: "Hello!" });
//     return { sent: true };
//   }
```

The `TwoPassExecutor` works with SandCastle's subprocess mode:
1. **Pass 1**: Run the code in a sandbox, collect all `codemode.*` tool calls
2. **Pass 2**: Execute tool calls host-side with real implementations
3. **Pass 3**: Re-run with results pre-populated so the code completes

## Using with AI Agents

SandCastle is designed to be a tool in an AI agent's toolkit:

```typescript
import { SandCastle } from "sandcastle";

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

Feed the guest type declarations (`sandcastle/guest`) to your LLM so it knows what APIs are available inside the sandbox.

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
cargo test                       # 52 Rust tests
cd sdk/typescript && bun test    # 146 TypeScript tests
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
│   ├── sandcastle-cli/      # CLI binary + HTTP server
│   └── sandcastle-macros/   # #[sandcastle::capability] proc macro
├── guest/                   # QuickJS WASM guest runtime
├── sdk/typescript/          # TypeScript SDK
│   ├── src/
│   │   ├── client.ts            # SandCastle class
│   │   ├── core/errors.ts       # Typed error hierarchy
│   │   ├── core/subprocess.ts   # CLI transport
│   │   ├── core/http.ts         # HTTP transport
│   │   ├── types/               # Public type definitions
│   │   └── guest/index.d.ts     # Guest-side type declarations
│   └── test/                # 115 tests (unit + integration + agent)
├── proto/                   # gRPC protobuf definitions
├── benches/                 # Benchmark suite
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

## License

Apache 2.0
