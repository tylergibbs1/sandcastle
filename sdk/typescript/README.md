# @grayhaven/sandcastle

The fastest way to run untrusted JavaScript safely. Zero-config, Bun-first.

```bash
bun add @grayhaven/sandcastle    # zero dependencies
npm install @grayhaven/sandcastle isolated-vm  # Node.js
```

```ts
import { evaluate, run } from "@grayhaven/sandcastle";

await evaluate("1 + 1");                    // 2
await evaluate("x * y", { x: 6, y: 7 });    // 42
await run("return input.x * 2", { x: 21 }); // 42
```

**Performance:** 66,000 ops/sec (Bun) / 380,000 ops/sec (Node.js) with pooling enabled.

See the [full documentation](https://github.com/tylergibbs1/sandcastle#readme) for the complete API reference.
