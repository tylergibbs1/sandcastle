# @grayhaven/sandcastle

TypeScript SDK for the [SandCastle](https://github.com/tylergibbs1/sandcastle) sandbox runtime.

## Install

```bash
npm install @grayhaven/sandcastle
# or
bun add @grayhaven/sandcastle
```

On macOS and Linux, the package tries to download the `sandcastle` CLI automatically during `postinstall`.

- Supported Node versions: 18+
- Automatic binary download: macOS and Linux
- Manual install required: unsupported platforms, offline installs, or when `postinstall` is skipped

## 5-Minute Quick Start

```typescript
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");

console.log(result); // 2
```

### With Input

```typescript
const result = await sc.run<string>(
  'return "Hello " + input.name;',
  { name: "World" }
);
// result === "Hello World"
```

If the binary is unavailable, the SDK throws `BinaryNotFoundError` with next steps. You can also verify the wrapper directly:

```bash
npx sandcastle --help
```

Or diagnose the SDK install directly:

```typescript
import { diagnoseInstallation } from "@grayhaven/sandcastle";

console.log(await diagnoseInstallation());
```

## Common Next Steps

- Need the CLI? See the top-level [CLI guide](../../docs/cli.md).
- Need troubleshooting? See [Troubleshooting](../../docs/troubleshooting.md).
- Need Rust embedding? See [Rust library mode](../../docs/rust.md).

## Code Mode

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

- ~61µs sandbox creation (16,500 ops/sec sustained)
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
- 144 integration tests + 34 CLI tests + 85 Rust experiments + 101 Code Mode experiments

## License

Apache-2.0
