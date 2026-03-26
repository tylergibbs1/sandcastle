# @grayhaven/sandcastle

TypeScript SDK for the [SandCastle](https://github.com/tylergibbs1/sandcastle) sandbox runtime — lightweight WASM-based sandboxes for AI agent code execution.

## Install

```bash
bun add @grayhaven/sandcastle
# or
npm install @grayhaven/sandcastle
```

## Quick Start

```typescript
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");
// result === 2
```

### With Input

```typescript
const result = await sc.run<string>(
  'const i = globalThis.__sandcastle_input; return "Hello " + i.name;',
  { name: "World" }
);
// result === "Hello World"
```

### Code Mode (reduce LLM tool calls by 80%)

```typescript
import { createCodeTool, TwoPassExecutor } from "@grayhaven/sandcastle/codemode";

const executor = new TwoPassExecutor();
const tool = createCodeTool({ tools: [...yourTools], executor });
// Give this single tool to your LLM instead of N separate tools
```

### Guest Type Declarations

Feed these to your LLM so it knows the sandbox API:

```typescript
import type {} from "@grayhaven/sandcastle/guest";
```

## Requirements

The `sandcastle` CLI must be installed and in your PATH. See the [main repo](https://github.com/tylergibbs1/sandcastle) for installation instructions.

## What works out of the box

LLM-generated code just works — no configuration needed:

```javascript
const _ = require('lodash');          // Built-in shim (groupBy, sortBy, get, etc.)
const { format } = require('date-fns'); // Built-in shim
const data = await fetch(url);        // Delegates to HTTP capability
setTimeout(() => {}, 100);            // Runs immediately (no event loop)
const clone = structuredClone(obj);   // JSON round-trip
const id = crypto.randomUUID();       // Polyfilled
const key = process.env.API_KEY;      // Injected by host via .with_env()
```

## Features

- Sub-millisecond sandbox creation (~600us, 1,700 ops/sec sustained)
- ES2024+ support (QuickJS-NG) — `Object.groupBy`, `Set.intersection`, iterator helpers
- Fuel metering, memory limits, wall-clock timeouts
- Host capability bridge (KV, HTTP, custom) with quota enforcement
- `fetch()` polyfill — delegates to HTTP capability when registered
- Module shims — `lodash`, `path`, `uuid`, `date-fns`, `qs` work via `require()`
- Streaming output — real-time `console.log` callback (Rust API)
- Persistent sandboxes — multi-turn state via input passing
- Persistent KV — SQLite-backed storage that survives restarts (Rust API, `--features persistent-kv`)
- Secret injection — `process.env` populated from host-side config (Rust API)
- Input/output artifacts (virtual filesystem)
- Promise/async support (`return Promise.all([...])` works)
- Polyfills: TextEncoder/TextDecoder, URL, atob/btoa, crypto, setTimeout, structuredClone, performance.now
- Memory protection: `MemoryExceeded` status instead of opaque WASM traps
- LLM-friendly errors: `require('express')` explains alternatives instead of crashing
- 140 integration tests, 85 experiments

## License

Apache-2.0
