import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const ivm = require("isolated-vm");

// ---------------------------------------------------------------------------
// Pooled V8 executor (mirrors src/core/v8.ts hot path)
// ---------------------------------------------------------------------------

const pool = [];
const POOL_MAX = 8;

function acquireIsolate() {
  while (pool.length > 0) {
    const entry = pool.pop();
    if (!entry.isolate.isDisposed) return entry;
  }
  return null;
}

function releaseIsolate(entry) {
  entry.uses++;
  if (pool.length < POOL_MAX && !entry.isolate.isDisposed) {
    pool.push(entry);
  } else if (!entry.isolate.isDisposed) {
    entry.isolate.dispose();
  }
}

function newIsolate() {
  return { isolate: new ivm.Isolate({ memoryLimit: 128 }), createdAt: Date.now(), uses: 0 };
}

async function execute(code, input, usePool = false) {
  let entry = null;
  let isolate;
  let ownsIsolate = true;

  if (usePool) {
    entry = acquireIsolate() ?? newIsolate();
    isolate = entry.isolate;
    ownsIsolate = false;
  } else {
    isolate = new ivm.Isolate({ memoryLimit: 128 });
  }

  try {
    const context = await isolate.createContext();
    const jail = context.global;

    // Input
    if (input !== undefined) {
      const copy = new ivm.ExternalCopy(input);
      await jail.set("__input", copy);
      copy.release();
    }

    // Single eval: console stub + input copy + user code (saves 1-2 eval round-trips)
    const inputSetup = input !== undefined ? "const input = __input.copy();" : "";
    const wrapped = `const console={log(){},warn(){},error(){},debug(){}};${inputSetup}(() => { try { return JSON.stringify({ok:true,value:(()=>{${code}})()}); } catch(e) { return JSON.stringify({ok:false,error:e.message}); } })()`;
    const raw = await context.eval(wrapped, { timeout: 10000 });
    context.release();
    return JSON.parse(String(raw));
  } finally {
    if (entry && usePool) {
      releaseIsolate(entry);
    } else if (ownsIsolate && !isolate.isDisposed) {
      isolate.dispose();
    }
  }
}

// ---------------------------------------------------------------------------
// Benchmarks
// ---------------------------------------------------------------------------

// Warmup
for (let i = 0; i < 10; i++) await execute("return 1", undefined, true);

const ITERS = 1000;

// --- Unpooled ---
let start = performance.now();
for (let i = 0; i < ITERS; i++) await execute("return 1 + 1");
let elapsed = performance.now() - start;
console.log(`Unpooled: ${ITERS} iters in ${elapsed.toFixed(0)}ms (${(elapsed/ITERS).toFixed(3)}ms/call)`);
console.log(`  Ops/sec: ${Math.round(ITERS / (elapsed / 1000))}`);

// --- Pooled ---
start = performance.now();
for (let i = 0; i < ITERS; i++) await execute("return 1 + 1", undefined, true);
elapsed = performance.now() - start;
console.log(`Pooled: ${ITERS} iters in ${elapsed.toFixed(0)}ms (${(elapsed/ITERS).toFixed(3)}ms/call)`);
console.log(`  Ops/sec: ${Math.round(ITERS / (elapsed / 1000))}`);

// --- JSON processing (pooled) ---
const jsonInput = { items: Array.from({length: 100}, (_, i) => ({ id: i, value: i * 10 })) };
start = performance.now();
for (let i = 0; i < 200; i++) {
  await execute("return input.items.filter(x => x.value > 500).length", jsonInput, true);
}
elapsed = performance.now() - start;
console.log(`JSON processing (pooled): 200 iters in ${elapsed.toFixed(0)}ms`);
console.log(`  Ops/sec: ${Math.round(200 / (elapsed / 1000))}`);

// Cleanup pool
for (const e of pool) if (!e.isolate.isDisposed) e.isolate.dispose();
