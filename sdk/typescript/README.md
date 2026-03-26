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

## Features

- Sub-millisecond sandbox creation (~600us, 1,700 ops/sec sustained)
- Fuel metering, memory limits, wall-clock timeouts
- Host capability bridge (KV, HTTP, custom) with quota enforcement
- Input/output artifacts (virtual filesystem)
- Execution transcripts with console output
- Promise/async support (`return Promise.all([...])` works)
- Built-in polyfills: TextEncoder/TextDecoder, URL, atob/btoa, crypto
- Memory protection: `MemoryExceeded` status instead of opaque WASM traps
- 140 integration tests, 80 experiments

## License

Apache-2.0
