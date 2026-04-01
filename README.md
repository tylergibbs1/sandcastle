<p align="center">
  <img src="logo.svg" alt="SandCastle" width="200" />
</p>

<h1 align="center">SandCastle</h1>

<p align="center">Lightweight WASM-based sandbox runtime for AI agent code execution.</p>

SandCastle lets you execute JavaScript in secure, isolated sandboxes with **~61µs cold starts** and ~1.3MB peak memory per sandbox. It uses WebAssembly (Wasmtime) as the isolation layer and QuickJS-NG (ES2024+) as the JavaScript engine, with build-time pre-initialization via [wizer](https://github.com/nickelpack/nickel-runtime/tree/main/crates/wizer) to eliminate per-execution JS engine startup overhead.

For most developers, the best entry point is the TypeScript SDK.

## Quick Start

```bash
npm install @grayhaven/sandcastle
```

```ts
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");

console.log(result); // 2
```

What happens on install:

- On macOS and Linux, the package tries to download the `sandcastle` CLI automatically during `postinstall`.
- On unsupported platforms, or if the download is skipped, you can install the CLI manually from source.
- The SDK throws a `BinaryNotFoundError` with next steps if the CLI is unavailable.

Primary docs:

- [TypeScript quick start](docs/quickstart-typescript.md)
- [CLI guide](docs/cli.md)
- [Rust library mode](docs/rust.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Architecture](docs/architecture.md)

Support matrix:

- Node.js: 18+
- Auto binary download: macOS and Linux
- Manual install required: unsupported platforms, offline installs, or when `postinstall` is skipped

## Why

SandCastle is optimized for agent-style code execution: short-lived JavaScript runs, bounded side effects, fast startup, and explicit capability control.

```
                        Apple Silicon (M4)     AWS c7g.xlarge (Graviton3)
Sandbox creation:       61µs (16,500 ops/sec)  ~170µs (~5,800 ops/sec)
Simple expression:      62µs (16,100 ops/sec)  ~175µs (~5,700 ops/sec)
JSON processing:        847µs (100 items)      ~1.6ms
500 concurrent:         7.5ms (15µs/sandbox)   ~25ms (~50µs/sandbox)
Peak memory:            ~1.3MB per sandbox     ~1.3MB per sandbox
Guest WASM module:      ~1.1MB (pre-initialized)
```

**8,000x faster than Docker** (~500ms) and **1,600x faster than E2B** (~100ms).

AI agents need to run code. The usual options are containers (slow), V8 isolates (heavier), or `eval` (unsafe). SandCastle is a different tradeoff: WASM-based sandboxes that are fast, lightweight, and capability-driven.

| Solution | Startup | Memory | Binary/Runtime | JS Compat | Security |
|----------|---------|--------|----------------|-----------|----------|
| Docker | ~500ms | ~100MB+ | Docker daemon | Full Node.js | Namespace isolation |
| E2B | ~100-200ms | ~512MB min | Hosted / KVM | Full Node.js | Firecracker |
| V8 isolate (self-hosted) | ~3-5ms | ~5MB | ~50MB (V8 lib) | Full ES2024+ | V8 isolate boundary |
| Cloudflare Workers | ~3ms | ~5MB | Cloudflare only | Full ES2024+ | V8 isolates |
| **SandCastle** | **~61µs local** | **~1.3MB** | **~1.1MB WASM** | **ES2024+ (QuickJS-NG)** | **WASM spec boundary** |

### Containers vs SandCastle

Containers boot an entire OS kernel to run `return 1 + 1`. That's 100-500ms startup and 100MB+ memory — fine for long-running services, but AI agents make 10-50 tool calls per conversation. At 500ms per sandbox, that's 5-25 seconds of waiting. At 61µs, it's 0.6-3.1ms.

### V8 isolates vs SandCastle

V8 isolates are the closest alternative — they're in-process and don't need containers. The tradeoffs:

- **V8 wins on JIT performance** — faster for compute-heavy loops due to JIT compilation
- **SandCastle wins on startup** (0.061ms vs 3-5ms), **memory** (1.3MB vs 5MB), **binary size** (1.1MB vs ~50MB for the V8 library), and **embedding simplicity** (Wasmtime's API is small and clean; V8's is notoriously complex)
- **JS compatibility is now comparable** — SandCastle uses QuickJS-NG which supports ES2024+ including `Object.groupBy`, `Promise.withResolvers`, `Array.fromAsync`, `Set` methods, iterator helpers, and more

If you need Node.js APIs or heavy compute (JIT matters), use V8. If you need fast, lightweight sandboxes for AI agent code — data transforms, API orchestration, JSON processing — SandCastle is purpose-built for that.

### How the sandbox works

QuickJS (a C JavaScript engine) compiles to WASM and runs *inside* the sandbox. At build time, [wizer](https://github.com/nickelpack/nickel-runtime/tree/main/crates/wizer) pre-initializes the QuickJS runtime, context, and all polyfills, snapshotting the result into the WASM module's data segment. Each execution instantiates a fresh copy-on-write clone of this pre-initialized state — no JS engine startup, no polyfill evaluation. Guest code has zero access to the host — no filesystem, no network, no syscalls. Everything goes through explicit host function imports that you control and meter. The sandbox boundary is the WASM spec itself.

### Research context

This approach is validated by recent academic work:

- **[LLM-in-Sandbox](https://arxiv.org/abs/2601.16206)** (Feb 2026) shows LLMs spontaneously use sandbox environments to handle long contexts, achieving **8x token reduction** (100K → 13K tokens). SandCastle's Code Mode implements this pattern.
- **[CodeAgents](https://arxiv.org/html/2507.03254v1)** (Jul 2025) validates codified multi-agent reasoning — replacing N tool calls with a single code execution — the same pattern as our `createCodeTool()` / `TwoPassExecutor`.
- **[Fault-Tolerant Sandboxing](https://arxiv.org/abs/2512.12806)** (Dec 2025) uses filesystem snapshots for safety (14.5% overhead). SandCastle achieves stronger isolation at 800x lower cost because WASM provides memory isolation without snapshotting.
- **[Systems Security for Agentic Computing](https://arxiv.org/html/2512.01295v1)** (Dec 2025) argues for capability-based, least-privilege agent security — which is exactly SandCastle's architecture (typed capability bridge, per-capability quotas, domain allowlists).

### Performance optimization research

SandCastle's performance is the result of systematic research informed by academic literature:

- **[Wasm-level snapshotting](https://bytecodealliance.org/articles/making-javascript-run-fast-on-webassembly)** — Pre-initializing the QuickJS runtime, polyfills, and static bridge code via wizer eliminates ~2ms of per-execution JS engine startup, yielding a **49x throughput improvement** (338 → 16,500 ops/sec).
- **[Spectre mitigation analysis](https://arxiv.org/html/2404.12621v1)** — Disabling Cranelift's spectre mitigations for heap/table access (safe because our sandbox lacks high-resolution timing side-channels) provides a **23% throughput improvement**.
- **Synchronous WASM execution** — Switching from async to sync Wasmtime calls eliminates stack-switching overhead for the sequential execution path.
- **Copy-on-write instantiation** — Wasmtime's `memory_init_cow` + `InstancePre` pre-linking enables microsecond-scale instance creation from a pre-compiled module template.
- **Pooling allocator with affine slots** — Pre-allocated memory pool with slot affinity reuses the same memory region for repeated module instantiations, resetting via `memset` instead of `mmap`/`munmap` syscalls.

## Other Entry Points

### CLI

```bash
npx sandcastle --help
```

### Scaffold a project

```bash
mkdir my-project && cd my-project
npx sandcastle init
npx sandcastle run scripts/hello.js --input '{"name": "Alice"}'
```

### Rust library mode

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

### CLI

```bash
npx sandcastle run script.js --input '{"name": "Alice"}'
npx sandcastle doctor
npx sandcastle repl
npx sandcastle codegen capabilities.json -o types.d.ts
```

### HTTP Server

```bash
npx sandcastle serve --http 0.0.0.0:8080 --watch ./scripts

curl -X POST http://localhost:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{"code": "return input.x * 2", "input": {"x": 21}}'
```

### Deploy with Docker (one command)

```bash
docker compose up -d
```

Or without Compose:

```bash
docker build -t sandcastle . && docker run -p 8080:8080 sandcastle
```

The Docker image is ~120MB and starts in <1 second. The server exposes all REST endpoints on port 8080.

## Features

### Core Runtime
- **~61µs sandbox creation** — 16,500 ops/sec via wizer pre-initialization, pooling allocator with affine slot reuse, and Wasmtime AOT compilation
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
- **`sandcastle doctor`** — diagnose guest module resolution and runtime readiness
- **`sandcastle init`** — scaffold a new project
- **`sandcastle repl`** — interactive JS REPL with multi-line support
- **`sandcastle codegen`** — generate TypeScript declarations from capability definitions
- **`sandcastle info`** — print runtime info

## Code Mode

Code Mode replaces sequential tool calls with a single code execution. Instead of the LLM making N separate `tool_use` calls (N round-trips), it writes one function that chains all N calls — cutting token usage by up to 80%.

This is SandCastle's answer to [Cloudflare's Code Mode](https://blog.cloudflare.com/sandboxing-ai-agents-100x-faster/).

```typescript
import { createCodeTool, TwoPassExecutor } from "@grayhaven/sandcastle/codemode";

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
import { createCodeTool, TwoPassExecutor } from "@grayhaven/sandcastle/codemode";
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
# Prerequisites: Rust 1.85+, wasm32-wasip1 target
rustup target add wasm32-wasip1

# Optional: install wizer for pre-initialization (10x faster sandbox creation)
cargo install wizer --all-features

# Build everything
make build

# Or step by step:
cd guest && ./build.sh          # Build QuickJS-NG WASM guest (~1.1MB with wizer)
cargo build --release            # Build runtime + CLI

# Run tests
make test                        # 144 Rust tests
make test-cli                    # 34 CLI integration tests
cd sdk/typescript && bun test    # TypeScript + Code Mode tests

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
├── guest/                   # QuickJS WASM guest runtime (wizer pre-initialized)
├── sdk/typescript/          # TypeScript SDK
│   ├── src/
│   │   ├── client.ts            # SandCastle class
│   │   ├── codemode/            # Code Mode SDK
│   │   ├── core/                # Errors, subprocess, HTTP transport
│   │   ├── types/               # Public type definitions
│   │   └── guest/index.d.ts     # Guest-side type declarations
│   └── test/                # Unit, integration, and Code Mode agent tests
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
