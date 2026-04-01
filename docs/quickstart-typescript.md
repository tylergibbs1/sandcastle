# TypeScript Quick Start

## Install

```bash
# Bun (zero dependencies)
bun add @grayhaven/sandcastle

# Node.js
npm install @grayhaven/sandcastle isolated-vm
```

## Run your first sandbox

```ts
import { evaluate, run } from "@grayhaven/sandcastle";

await evaluate("1 + 1");                    // 2
await evaluate("x * y", { x: 6, y: 7 });    // 42
await run("return input.name", { name: "Alice" }); // "Alice"
```

No constructor, no config, no binary download. It works immediately.

## More control

```ts
import { SandCastle } from "@grayhaven/sandcastle";

const sc = SandCastle.strict(); // tight limits for untrusted code

// Reusable sandboxed functions
const double = sc.wrap<number, [number]>("return args[0] * 2");
await double(21); // 42

// Persistent sessions
const session = await sc.session();
await session.run("var x = 1");
await session.eval("x + 1"); // 2
session.dispose();

// Parallel execution
await sc.batch(["return 1", "return 2", "return 3"]); // [1, 2, 3]
```

## How it works

- **On Bun:** runs code in Worker threads (JavaScriptCore). Zero npm dependencies.
- **On Node.js:** runs code in V8 isolates via `isolated-vm`.
- Both provide isolated execution contexts with timeout and memory enforcement.
- The SDK auto-detects your runtime and picks the right backend.

## Next steps

- [Full API reference](../README.md#api)
- [Using with AI agents](../README.md#using-with-ai-agents)
- [Code Mode](../README.md#code-mode)
