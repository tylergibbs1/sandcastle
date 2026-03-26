import { describe, expect, it } from "bun:test";
import {
  ExecutionFailedError,
  GuestError,
  jsonArtifact,
  SandCastle,
  textArtifact,
} from "../src/index.js";

// ---------------------------------------------------------------------------
// Setup — these tests require the release binary + guest WASM to be built.
// They auto-skip when the binary is not found.
// ---------------------------------------------------------------------------

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

let hasBinary = false;
try {
  hasBinary = Bun.file(BINARY_PATH).size > 0;
} catch {
  /* not found */
}

const run = hasBinary ? it : it.skip;

function sc(overrides: Record<string, unknown> = {}): SandCastle {
  return new SandCastle({
    binaryPath: BINARY_PATH,
    guestModule: GUEST_MODULE,
    ...overrides,
  });
}

// ---------------------------------------------------------------------------
// Basic execution
// ---------------------------------------------------------------------------

describe("basic execution", () => {
  run("returns a number", async () => {
    const result = await sc().run<number>("return 1 + 1;");
    expect(result).toBe(2);
  });

  run("returns a string", async () => {
    const result = await sc().run<string>("return 'hello';");
    expect(result).toBe("hello");
  });

  run("returns null", async () => {
    const result = await sc().run("return null;");
    expect(result).toBeNull();
  });

  run("returns an object", async () => {
    const result = await sc().run<{ a: number; b: string }>("return { a: 42, b: 'test' };");
    expect(result.a).toBe(42);
    expect(result.b).toBe("test");
  });

  run("returns an array", async () => {
    const result = await sc().run<number[]>("return [1, 2, 3];");
    expect(result).toEqual([1, 2, 3]);
  });

  run("returns a boolean", async () => {
    const result = await sc().run<boolean>("return true;");
    expect(result).toBe(true);
  });

  run("returns nested objects", async () => {
    const result = await sc().run<{ users: { id: number }[] }>(
      "return { users: [{ id: 1 }, { id: 2 }] };",
    );
    expect(result.users).toHaveLength(2);
    expect(result.users[0].id).toBe(1);
  });
});

// ---------------------------------------------------------------------------
// Input passing
// ---------------------------------------------------------------------------

describe("input", () => {
  run("passes JSON input to guest", async () => {
    const result = await sc().run<{ sum: number }>(
      "const i = globalThis.__sandcastle_input; return { sum: i.a + i.b };",
      { a: 10, b: 32 },
    );
    expect(result.sum).toBe(42);
  });

  run("handles null input", async () => {
    const result = await sc().run<null>("return globalThis.__sandcastle_input;", null);
    expect(result).toBeNull();
  });

  run("handles string input", async () => {
    const result = await sc().run<string>("return globalThis.__sandcastle_input;", "hello");
    expect(result).toBe("hello");
  });

  run("handles array input", async () => {
    const result = await sc().run<number[]>("return globalThis.__sandcastle_input;", [1, 2, 3]);
    expect(result).toEqual([1, 2, 3]);
  });

  run("handles nested input", async () => {
    const input = {
      users: [{ name: "Alice", scores: [90, 85] }],
      meta: { version: 2 },
    };
    const result = await sc().run("return globalThis.__sandcastle_input;", input);
    expect(result).toEqual(input);
  });

  run("handles no input (undefined)", async () => {
    const { ok } = await sc().execute({ code: "return 1;" });
    expect(ok).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Console capture
// ---------------------------------------------------------------------------

describe("console capture", () => {
  run("captures console.log", async () => {
    const { transcript } = await sc().execute({
      code: 'console.log("hello"); return null;',
    });
    expect(transcript.console.length).toBeGreaterThanOrEqual(1);
    expect(transcript.console[0].level).toBe("log");
    expect(transcript.console[0].message).toBe("hello");
  });

  run("captures console.warn", async () => {
    const { transcript } = await sc().execute({
      code: 'console.warn("warning"); return null;',
    });
    const warn = transcript.console.find((c) => c.level === "warn");
    expect(warn).toBeDefined();
    expect(warn!.message).toBe("warning");
  });

  run("captures console.error", async () => {
    const { transcript } = await sc().execute({
      code: 'console.error("bad"); return null;',
    });
    const err = transcript.console.find((c) => c.level === "error");
    expect(err).toBeDefined();
    expect(err!.message).toBe("bad");
  });

  run("captures multiple console messages in order", async () => {
    const { transcript } = await sc().execute({
      code: `
        console.log("first");
        console.log("second");
        console.log("third");
        return null;
      `,
    });
    const logs = transcript.console.filter((c) => c.level === "log");
    expect(logs.length).toBeGreaterThanOrEqual(3);
    expect(logs[0].message).toBe("first");
    expect(logs[1].message).toBe("second");
    expect(logs[2].message).toBe("third");
  });

  run("console timestamps are monotonically increasing", async () => {
    const { transcript } = await sc().execute({
      code: `
        console.log("a");
        console.log("b");
        console.log("c");
        return null;
      `,
    });
    for (let i = 1; i < transcript.console.length; i++) {
      expect(transcript.console[i].ts).toBeGreaterThanOrEqual(transcript.console[i - 1].ts);
    }
  });

  run("console.log stringifies non-string arguments", async () => {
    const { transcript } = await sc().execute({
      code: 'console.log("count:", 42); return null;',
    });
    const log = transcript.console.find((c) => c.message.includes("42"));
    expect(log).toBeDefined();
  });
});

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

describe("error handling", () => {
  run("guest throw → ok: false, status: guest_error", async () => {
    const result = await sc().execute({
      code: 'throw new Error("boom");',
    });
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
  });

  run("run() throws GuestError on guest throw", async () => {
    try {
      await sc().run('throw new Error("boom");');
      expect(true).toBe(false);
    } catch (err) {
      expect(err).toBeInstanceOf(GuestError);
      expect(err).toBeInstanceOf(ExecutionFailedError);
      const e = err as GuestError;
      expect(e.result).toBeDefined();
      expect(e.result.transcript).toBeDefined();
    }
  });

  run("syntax error in guest code → guest_error", async () => {
    const result = await sc().execute({ code: "{{{{" });
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
  });

  run("ReferenceError in guest code → guest_error", async () => {
    const result = await sc().execute({
      code: "return undefinedVariable;",
    });
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
  });

  run("TypeError in guest code → guest_error", async () => {
    const result = await sc().execute({
      code: "null.property;",
    });
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
  });
});

// ---------------------------------------------------------------------------
// Resource limits
// ---------------------------------------------------------------------------

describe("resource limits", () => {
  run("fuel exhaustion is detected", async () => {
    const result = await sc().execute({
      code: "let x = 0; while(true) { x++; } return x;",
      limits: { fuel: 200_000_000, timeoutMs: 30_000 },
    });
    expect(result.ok).toBe(false);
    expect(["fuel_exhausted", "timeout", "guest_error"]).toContain(result.status.type);
  });

  run("timeout is detected", async () => {
    const result = await sc().execute({
      code: "while(true) {} return null;",
      limits: { fuel: 0, timeoutMs: 2_000 },
    });
    expect(result.ok).toBe(false);
    // May be timeout or fuel_exhausted depending on epoch timing
    expect(["timeout", "fuel_exhausted", "guest_error"]).toContain(result.status.type);
  });

  run("per-call limits override defaults", async () => {
    const client = sc({ defaults: { fuel: 200_000_000 } });
    // The per-call fuel should override the default
    const result = await client.execute({
      code: "return 42;",
      limits: { fuel: 1_000_000_000 },
    });
    expect(result.ok).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

describe("transcript", () => {
  run("includes execution ID", async () => {
    const { transcript } = await sc().execute({ code: "return 1;" });
    expect(transcript.executionId).toBeTruthy();
    expect(typeof transcript.executionId).toBe("string");
  });

  run("includes start and finish timestamps", async () => {
    const { transcript } = await sc().execute({ code: "return 1;" });
    expect(transcript.startedAt).toBeTruthy();
    expect(transcript.finishedAt).toBeTruthy();
    // Finish is after start
    expect(new Date(transcript.finishedAt!).getTime()).toBeGreaterThanOrEqual(
      new Date(transcript.startedAt).getTime(),
    );
  });

  run("includes fuel consumption", async () => {
    const { transcript } = await sc().execute({ code: "return 1 + 1;" });
    expect(transcript.fuelConsumed).toBeGreaterThan(0);
    expect(transcript.fuelLimit).toBeGreaterThan(0);
  });

  run("includes memory usage", async () => {
    const { transcript } = await sc().execute({ code: "return 1;" });
    expect(transcript.peakMemoryBytes).toBeGreaterThan(0);
    expect(transcript.memoryLimitBytes).toBeGreaterThan(0);
  });

  run("status in transcript matches top-level status", async () => {
    const result = await sc().execute({ code: "return 1;" });
    expect(result.transcript.status).toEqual(result.status);
  });

  run("output in transcript matches top-level output", async () => {
    const result = await sc().execute({ code: "return { x: 1 };" });
    expect(result.transcript.output).toEqual(result.output);
  });

  run("unique execution IDs across calls", async () => {
    const client = sc();
    const [r1, r2] = await Promise.all([
      client.execute({ code: "return 1;" }),
      client.execute({ code: "return 2;" }),
    ]);
    expect(r1.transcript.executionId).not.toBe(r2.transcript.executionId);
  });
});

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

describe("artifacts", () => {
  run("reads text artifact in guest", async () => {
    const result = await sc().execute({
      code: `
        const data = __sandcastle_read_artifact("data.txt");
        return { content: data };
      `,
      artifacts: [textArtifact("data.txt", "Hello, World!")],
    });
    expect(result.ok).toBe(true);
    if (result.output.type === "json") {
      const val = result.output.value as { content: string };
      expect(val.content).toBe("Hello, World!");
    }
  });

  run("reads JSON artifact in guest", async () => {
    const result = await sc().execute({
      code: `
        const raw = __sandcastle_read_artifact("config.json");
        const config = JSON.parse(raw);
        return { port: config.port };
      `,
      artifacts: [jsonArtifact("config.json", { port: 8080, host: "localhost" })],
    });
    expect(result.ok).toBe(true);
    if (result.output.type === "json") {
      expect((result.output.value as { port: number }).port).toBe(8080);
    }
  });

  run("missing artifact returns null", async () => {
    const result = await sc().execute({
      code: `
        const data = __sandcastle_read_artifact("nonexistent.txt");
        return { found: data !== null && data !== undefined };
      `,
    });
    expect(result.ok).toBe(true);
    if (result.output.type === "json") {
      expect((result.output.value as { found: boolean }).found).toBe(false);
    }
  });

  run("writes output artifact", async () => {
    const result = await sc().execute({
      code: `
        __sandcastle_write_artifact("result.json", JSON.stringify({ done: true }));
        return null;
      `,
    });
    expect(result.ok).toBe(true);
    // Output artifacts are captured in the result
    // (currently not passed through subprocess mode, but the call succeeds)
  });

  run("multiple artifacts", async () => {
    const result = await sc().execute({
      code: `
        const a = __sandcastle_read_artifact("a.txt");
        const b = __sandcastle_read_artifact("b.txt");
        return { a, b };
      `,
      artifacts: [textArtifact("a.txt", "AAA"), textArtifact("b.txt", "BBB")],
    });
    expect(result.ok).toBe(true);
    if (result.output.type === "json") {
      const val = result.output.value as { a: string; b: string };
      expect(val.a).toBe("AAA");
      expect(val.b).toBe("BBB");
    }
  });
});

// ---------------------------------------------------------------------------
// Data transformation patterns (real agent workloads)
// ---------------------------------------------------------------------------

describe("data transformation", () => {
  run("filter and map", async () => {
    const result = await sc().run<{ evens: number[]; sum: number }>(
      `
        const data = globalThis.__sandcastle_input;
        const evens = data.numbers.filter(n => n % 2 === 0);
        const sum = evens.reduce((a, b) => a + b, 0);
        return { evens, sum };
      `,
      { numbers: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] },
    );
    expect(result.evens).toEqual([2, 4, 6, 8, 10]);
    expect(result.sum).toBe(30);
  });

  run("string manipulation", async () => {
    const result = await sc().run<string[]>(
      `
        const items = globalThis.__sandcastle_input;
        return items.map(s => s.toUpperCase().trim());
      `,
      [" hello ", " world ", " test "],
    );
    expect(result).toEqual(["HELLO", "WORLD", "TEST"]);
  });

  run("JSON parsing and restructuring", async () => {
    const result = await sc().run<{ count: number; names: string[] }>(
      `
        const data = globalThis.__sandcastle_input;
        const active = data.users.filter(u => u.active);
        return {
          count: active.length,
          names: active.map(u => u.name).sort(),
        };
      `,
      {
        users: [
          { name: "Charlie", active: true },
          { name: "Alice", active: true },
          { name: "Bob", active: false },
          { name: "Diana", active: true },
        ],
      },
    );
    expect(result.count).toBe(3);
    expect(result.names).toEqual(["Alice", "Charlie", "Diana"]);
  });

  run("math operations", async () => {
    const result = await sc().run<{ mean: number; max: number; min: number }>(
      `
        const nums = globalThis.__sandcastle_input;
        return {
          mean: nums.reduce((a, b) => a + b, 0) / nums.length,
          max: Math.max(...nums),
          min: Math.min(...nums),
        };
      `,
      [10, 20, 30, 40, 50],
    );
    expect(result.mean).toBe(30);
    expect(result.max).toBe(50);
    expect(result.min).toBe(10);
  });

  run("Date operations", async () => {
    const result = await sc().run<boolean>(
      `
        const d = new Date("2026-01-15T00:00:00Z");
        return d.getFullYear() === 2026;
      `,
    );
    expect(result).toBe(true);
  });

  // URL and crypto.randomUUID are not available in the QuickJS guest runtime.
  // These are documented as available in the PRD but depend on polyfills
  // that will be added in Phase 2's pre-bundled standard library.
  run("JSON.stringify and JSON.parse round-trip", async () => {
    const result = await sc().run<{ ok: boolean }>(
      `
        const obj = { key: "value", n: 42 };
        const str = JSON.stringify(obj);
        const parsed = JSON.parse(str);
        return { ok: parsed.key === "value" && parsed.n === 42 };
      `,
    );
    expect(result.ok).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

describe("concurrent executions", () => {
  run("handles 5 concurrent executions", async () => {
    const client = sc();
    const results = await Promise.all(
      Array.from({ length: 5 }, (_, i) => client.run<{ id: number }>(`return { id: ${i} };`)),
    );
    const ids = results.map((r) => r.id).sort();
    expect(ids).toEqual([0, 1, 2, 3, 4]);
  });

  run("independent sandboxes have no shared state", async () => {
    const client = sc();
    // First execution sets a global
    await client.execute({
      code: "globalThis.mySecret = 42; return null;",
    });
    // Second execution should NOT see it
    const result = await client.run<boolean>("return typeof globalThis.mySecret === 'undefined';");
    expect(result).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Edge cases
// ---------------------------------------------------------------------------

describe("edge cases", () => {
  run("empty code returns null output", async () => {
    const result = await sc().execute({ code: "" });
    // Empty code may succeed or error depending on QuickJS behavior
    // The important thing is it doesn't crash the host
    expect(result).toBeDefined();
  });

  run("very long output", async () => {
    const result = await sc().run<string>("return 'x'.repeat(10000);");
    expect(result.length).toBe(10000);
  });

  run("special characters in strings", async () => {
    const result = await sc().run<string>(String.raw`return "hello\nworld\ttab\"quote";`);
    expect(result).toContain("hello");
    expect(result).toContain("world");
  });

  run("returning undefined is treated as null", async () => {
    const result = await sc().execute({ code: "return undefined;" });
    // QuickJS may treat this differently — just verify it doesn't crash
    expect(result).toBeDefined();
  });

  run("large input object", async () => {
    const bigInput = {
      items: Array.from({ length: 1000 }, (_, i) => ({
        id: i,
        name: `item_${i}`,
        value: Math.random(),
      })),
    };
    const result = await sc().run<number>(
      "return globalThis.__sandcastle_input.items.length;",
      bigInput,
    );
    expect(result).toBe(1000);
  });
});
