<p align="center">
  <img src="logo.svg" alt="SandCastle" width="200" />
</p>

<h1 align="center">SandCastle</h1>

<p align="center">The fastest way to run untrusted JavaScript safely.</p>

SandCastle is a JavaScript sandbox that just works. Install it, run code, get results. No binaries to download, no Docker, no configuration. It auto-detects your runtime (Bun or Node.js) and picks the fastest isolation backend.

```
Bun:     66,000 ops/sec  (zero dependencies)
Node.js: 281,000 ops/sec (via isolated-vm)
```

## Quick Start

```bash
# Bun (zero dependencies)
bun add @grayhaven/sandcastle

# Node.js (installs isolated-vm for V8 sandboxing)
npm install @grayhaven/sandcastle isolated-vm
```

```ts
import { evaluate, run } from "@grayhaven/sandcastle";

await evaluate("1 + 1");                              // 2
await evaluate("x * y", { x: 6, y: 7 });              // 42
await run("return items.filter(x => x > 2)", [1,2,3,4]); // [3, 4]
```

No constructor. No setup. It works out of the box.

## API

### `evaluate(expression, globals?)` — eval an expression

No `return` needed. Inject variables as globals.

```ts
await evaluate("Math.max(1, 5, 3)");                   // 5
await evaluate("name.toUpperCase()", { name: "alice" }); // "ALICE"
await evaluate("items.length", { items: [1, 2, 3] });   // 3
```

### `run(code, input?)` — run a code block

Use `return` to produce output. Second argument becomes `input` in the sandbox.

```ts
await run("return 1 + 1");                              // 2
await run("return input.x * 2", { x: 21 });             // 42
await run("return input.map(x => x * 10)", [1, 2, 3]);  // [10, 20, 30]
```

### `new SandCastle(options?)` — full control

```ts
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle({
  defaults: { timeoutMs: 5_000, memoryMb: 64 },
  pool: { maxIsolates: 8 },
  hostFunctions: {
    getPrice: (ticker) => prices[ticker],
  },
  onConsole: (level, msg) => console.log(`[sandbox] ${msg}`),
});

await sc.run("return getPrice('AAPL')");
```

### `.eval(expression, globals?)` — expression evaluation

```ts
await sc.eval("x + y", { x: 40, y: 2 });               // 42
await sc.eval("[1,2,3].map(x => x * 2)");               // [2, 4, 6]
```

### `.wrap(code)` — reusable sandboxed function

Turn sandbox code into a function you call like any other function.

```ts
const double = sc.wrap<number, [number]>("return args[0] * 2");
await double(21);   // 42
await double(5);    // 10

const greet = sc.wrap<string>("return `Hello, ${name}!`");
await greet({ name: "Alice" });  // "Hello, Alice!"
```

### `.session()` — persistent state across calls

Variables, functions, and state carry across calls. Like a REPL.

```ts
const session = await sc.session();
await session.run("var counter = 0");
await session.run("counter++");
await session.run("counter++");
await session.eval("counter");  // 2

await session.run("function double(x) { return x * 2 }");
await session.eval("double(21)");  // 42
session.dispose();
```

### `.batch(items)` — parallel execution

```ts
const results = await sc.batch([
  "return 1 + 1",
  "return 2 + 2",
  "return 3 + 3",
]);
// [2, 4, 6]
```

### `.test(code)` — boolean check, never throws

```ts
await sc.test("return 1 + 1");            // true
await sc.test("throw new Error('no')");   // false
await sc.test("while(true){}");           // false (timeout)
```

### `.execute(options)` — full result with metadata

```ts
const result = await sc.execute({ code: 'console.log("hi"); return 42' });

result.ok          // true
result.value       // 42
result.ms          // 0.3
result.logs        // [{ level: "log", message: "hi", ts: 0 }]
result.memoryBytes // 2048000
result.status      // { type: "success" }
result.transcript  // full execution transcript
```

### Presets

```ts
// Tight limits (32MB, 1s timeout) — for untrusted user code
const sc = SandCastle.strict();

// Generous limits (512MB, 60s timeout, large pool) — for internal tools
const sc = SandCastle.permissive();
```

### Middleware

```ts
sc.use({
  beforeExecute(ctx) {
    console.log("Starting execution...");
  },
  afterExecute(ctx, result) {
    metrics.record("sandbox_ms", result.ms);
    metrics.record("sandbox_memory", result.memoryBytes);
  },
  onError(ctx, error) {
    logger.error("Sandbox failed", error);
  },
});
```

### Globals injection

Pass variables directly into the sandbox scope:

```ts
await sc.run("return greeting + ' ' + name", {
  globals: { greeting: "Hello", name: "World" },
});
// "Hello World"

// Globals and input work together
await sc.run("return input.x + bonus", {
  globals: { bonus: 10 },
  input: { x: 32 },
});
// 42
```

### Host functions

Expose Node.js/Bun functions to sandboxed code:

```ts
const sc = new SandCastle({
  hostFunctions: {
    fetchPrice: (ticker) => prices[ticker],
    log: (msg) => console.log("[sandbox]", msg),
    readConfig: (key) => config[key],
  },
});

await sc.run("log(fetchPrice('AAPL')); return readConfig('max_retries')");
```

### Streaming console

Get `console.log` output in real-time as code executes:

```ts
const sc = new SandCastle({
  onConsole: (level, message, ts) => {
    process.stderr.write(`[${level}] ${message}\n`);
  },
});
```

Sandbox code also supports `console.time()`, `console.timeEnd()`, and `console.timeLog()`.

### Async/await

Top-level `await` works naturally:

```ts
await sc.run(`
  const data = await Promise.resolve({ name: "test" });
  return data.name;
`);
// "test"
```

### Typed errors

```ts
import { TimeoutError, MemoryExceededError, GuestError } from "@grayhaven/sandcastle";

try {
  await sc.run("while(true){}");
} catch (e) {
  if (e instanceof TimeoutError) {
    // e.result has the full execution result
    // e.guestStack has the sandbox stack trace
  }
}
```

### AbortSignal

```ts
const controller = new AbortController();
setTimeout(() => controller.abort(), 100);

await sc.execute({
  code: "while(true){}",
  signal: controller.signal,
});
```

## How it works

SandCastle auto-detects your runtime and picks the best backend:

| Runtime | Backend | Dependencies | Performance |
|---------|---------|-------------|-------------|
| **Bun** | Native Worker threads (JSC) | None | 66,000 ops/sec |
| **Node.js** | V8 isolates (isolated-vm) | `isolated-vm` | 281,000 ops/sec |

**On Bun:** Each execution runs in a separate Worker thread with its own JavaScriptCore context. Zero npm dependencies — just `bun add @grayhaven/sandcastle` and go.

**On Node.js:** Uses [isolated-vm](https://github.com/nickelpackers/isolated-vm) for in-process V8 isolates with context reuse and `evalSync` for minimal overhead.

Both backends provide:
- Separate JS context per execution (no shared globals)
- Timeout enforcement
- Memory limits
- Console capture
- Structured error reporting

### Pooling

Enable isolate/worker pooling for maximum throughput:

```ts
const sc = new SandCastle({ pool: { maxIsolates: 8 } });
```

This reuses warm isolates/workers across calls instead of creating new ones, which is where the 66K-281K ops/sec numbers come from.

## Using with AI Agents

SandCastle is designed for AI agent code execution:

```ts
const sandbox = new SandCastle();

const tool = {
  name: "run_code",
  description: "Execute JavaScript in a secure sandbox",
  execute: async ({ code, input }) => {
    const result = await sandbox.execute({ code, input });
    if (result.ok) return result.value;
    return `Error: ${result.status.message}`;
  },
};
```

### Code Mode

Replace N sequential tool calls with 1 code execution — up to 80% token reduction:

```ts
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

const codemode = createCodeTool({ tools, executor: new TwoPassExecutor() });
// Give `codemode` to your LLM as a single tool
```

## Deployment

### Bun / Node.js (default)

Nothing to deploy — the sandbox runs in-process. Just install and use.

```ts
// Next.js API route
import { run } from "@grayhaven/sandcastle";

export async function POST(req: Request) {
  const { code, input } = await req.json();
  return Response.json(await run(code, input));
}
```

### HTTP server mode

For microservices or multi-language backends:

```bash
npx sandcastle serve --http 0.0.0.0:8080

curl -X POST http://localhost:8080/execute \
  -H 'Content-Type: application/json' \
  -d '{"code": "return input.x * 2", "input": {"x": 21}}'
```

```ts
const sc = new SandCastle({ httpEndpoint: "http://localhost:8080" });
```

### Docker

```bash
docker compose up -d
```

### Vercel / serverless

Works out of the box on Node.js 18+. On Bun-based serverless, zero dependencies.

## Comparison

| Solution | Install | Latency | Isolation | Dependencies |
|----------|---------|---------|-----------|-------------|
| **SandCastle (Bun)** | `bun add` | **15µs pooled** | Worker thread | **None** |
| **SandCastle (Node)** | `npm install` | **3.5µs pooled** | V8 isolate | `isolated-vm` |
| isolated-vm (raw) | `npm install` | ~0.5ms | V8 isolate | Native addon |
| Docker | Docker daemon | ~500ms | Container | Docker |
| E2B | API key | ~100ms | Firecracker VM | Network |
| `eval()` | Built-in | ~0.01ms | **None** | None |

## Security Model

Guest code runs in an isolated context with no access to the host:
- **No filesystem** — no `fs`, no `require('fs')`
- **No network** — no `fetch`, no sockets (unless you expose them via host functions)
- **No process access** — no `process.exit`, no `child_process`
- **Timeout enforcement** — infinite loops are killed
- **Memory limits** — configurable per-sandbox caps
- **Host functions are opt-in** — the sandbox can only call what you explicitly expose

## License

Apache 2.0
