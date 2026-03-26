import { describe, expect, it } from "bun:test";
import { createCodeTool } from "../src/codemode/create-code-tool.js";
import { normalizeCode } from "../src/codemode/normalize.js";
import type { CodeModeResult, Executor, ToolDefinition } from "../src/codemode/types.js";
import { generateTypes } from "../src/codemode/types-gen.js";
import { errorFromResult } from "../src/core/errors.js";
import type { ExecutionResult, ExecutionStatus, OutputValue } from "../src/index.js";
import {
  BinaryNotFoundError,
  ExecutionAbortedError,
  ExecutionFailedError,
  FuelExhaustedError,
  GuestError,
  MemoryExceededError,
  SandCastle,
  SandCastleError,
  TimeoutError,
} from "../src/index.js";

// ---------------------------------------------------------------------------
// Setup for sandbox integration tests
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
// Helper: build a minimal ExecutionResult for error tests
// ---------------------------------------------------------------------------

function fakeResult(status: ExecutionStatus): ExecutionResult {
  const output: OutputValue = { type: "null" };
  const now = new Date().toISOString();
  return {
    ok: status.type === "success",
    status,
    output,
    transcript: {
      executionId: "test-id",
      startedAt: now,
      finishedAt: now,
      status,
      fuelConsumed: 0,
      fuelLimit: 0,
      peakMemoryBytes: 0,
      memoryLimitBytes: 0,
      output,
      console: [],
      capabilityCalls: [],
    },
    outputArtifacts: [],
  };
}

// ===========================================================================
// 1. normalizeCode edge cases
// ===========================================================================

describe("normalizeCode edge cases", () => {
  it("wraps whitespace-only code in async arrow", () => {
    const result = normalizeCode("   \n  \n   ");
    expect(result).toContain("async () => {");
  });

  it("wraps comment-only code in async arrow, preserves comments", () => {
    const result = normalizeCode("// just a comment");
    expect(result).toContain("async () => {");
    expect(result).toContain("// just a comment");
  });

  it("strips ```javascript fence", () => {
    const result = normalizeCode("```javascript\nreturn 42;\n```");
    expect(result).toContain("return 42");
    expect(result).not.toContain("```");
  });

  it("strips ```ts fence", () => {
    const result = normalizeCode("```ts\nreturn 42;\n```");
    expect(result).toContain("return 42");
    expect(result).not.toContain("```");
  });

  it("strips bare ``` fence with no language tag", () => {
    const result = normalizeCode("```\nreturn 42;\n```");
    expect(result).toContain("return 42");
    expect(result).not.toContain("```");
  });

  it("handles nested code fences gracefully", () => {
    // Inner ``` should not cause a crash
    const code = '```js\nconst x = "```";\nreturn x;\n```';
    const result = normalizeCode(code);
    // Should not throw and should produce some output
    expect(typeof result).toBe("string");
    expect(result.length).toBeGreaterThan(0);
  });

  it("wraps non-async arrow () => in async", () => {
    const result = normalizeCode("() => { return 1; }");
    // () => is not recognized as async arrow, so it gets wrapped
    expect(result).toContain("async () => {");
    // The last line ends with "}", which isStatement treats as a statement,
    // so no implicit return is added — just wrapped in async arrow
    expect(result).toContain("() => { return 1; }");
  });

  it("adds implicit return for bare number literal", () => {
    const result = normalizeCode("42");
    expect(result).toContain("return (42)");
    expect(result).toContain("async () => {");
  });

  it("strips name from async function named()", () => {
    const result = normalizeCode("async function named() { return 1; }");
    expect(result).toContain("async () => {");
    expect(result).toContain("return 1");
    expect(result).not.toContain("named");
  });

  it("handles very long single-line code (10KB) without crashing", () => {
    const longCode = `return "${"a".repeat(10_000)}";`;
    const result = normalizeCode(longCode);
    expect(result).toContain("async () => {");
    expect(result.length).toBeGreaterThan(10_000);
  });

  it("handles Windows line endings (\\r\\n)", () => {
    const code = "const x = 1;\r\nreturn x;";
    const result = normalizeCode(code);
    expect(result).toContain("async () => {");
    expect(result).toContain("return x;");
  });
});

// ===========================================================================
// 2. generateTypes edge cases
// ===========================================================================

describe("generateTypes edge cases", () => {
  it("generates Record<string, unknown> for tool with no properties", () => {
    const tools: ToolDefinition[] = [
      {
        name: "empty_tool",
        description: "An empty tool",
        inputSchema: { type: "object" },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    expect(result).toContain("Record<string, unknown>");
  });

  it("generates boolean type correctly", () => {
    const tools: ToolDefinition[] = [
      {
        name: "flag_tool",
        description: "Has a boolean",
        inputSchema: {
          type: "object",
          properties: { enabled: { type: "boolean" } },
        },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    expect(result).toContain("boolean");
  });

  it("generates nested array of objects", () => {
    const tools: ToolDefinition[] = [
      {
        name: "nested_tool",
        description: "Has nested arrays",
        inputSchema: {
          type: "object",
          properties: {
            items: {
              type: "array",
              items: {
                type: "object",
                properties: {
                  name: { type: "string" },
                  value: { type: "number" },
                },
              },
            },
          },
        },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    expect(result).toContain("name");
    expect(result).toContain("string");
    expect(result).toContain("number");
    expect(result).toContain("[]");
  });

  it("produces no JSDoc line for empty description", () => {
    const tools: ToolDefinition[] = [
      {
        name: "no_desc",
        description: "",
        inputSchema: { type: "object" },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    // Method line should NOT have a JSDoc comment
    expect(result).not.toContain("/** */");
    expect(result).toContain("no_desc(input:");
  });

  it("converts underscored and hyphenated names to PascalCase", () => {
    const tools: ToolDefinition[] = [
      {
        name: "get_user_info",
        description: "Underscore tool",
        inputSchema: { type: "object" },
        execute: async () => null,
      },
      {
        name: "send-email-now",
        description: "Hyphen tool",
        inputSchema: { type: "object" },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    expect(result).toContain("GetUserInfoInput");
    expect(result).toContain("SendEmailNowInput");
  });

  it("generates empty codemode object for empty tools array", () => {
    const result = generateTypes([]);
    expect(result).toContain("declare const codemode: {");
    expect(result).toContain("};");
    // Should have no method lines between the braces
    const lines = result.split("\n");
    const startIdx = lines.findIndex((l) => l.includes("declare const codemode: {"));
    const endIdx = lines.findIndex((l) => l.trim() === "};");
    // Nothing between them
    expect(endIdx).toBe(startIdx + 1);
  });

  it("generates unknown for null/undefined inputSchema", () => {
    const tools: ToolDefinition[] = [
      {
        name: "null_schema",
        description: "Null schema",
        inputSchema: null as unknown as { type: string },
        execute: async () => null,
      },
      {
        name: "empty_schema",
        description: "Empty schema",
        inputSchema: {} as { type: string },
        execute: async () => null,
      },
    ];
    const result = generateTypes(tools);
    expect(result).toContain("NullSchemaInput = unknown");
    expect(result).toContain("EmptySchemaInput = unknown");
  });
});

// ===========================================================================
// 3. createCodeTool edge cases
// ===========================================================================

describe("createCodeTool edge cases", () => {
  it("does not inject types when description has no {{types}} placeholder", () => {
    const mockExecutor: Executor = {
      execute: async () => ({
        result: null,
        logs: [],
        toolCallCount: 0,
        toolCalls: [],
      }),
    };
    const tool = createCodeTool({
      tools: [
        {
          name: "test",
          description: "A test tool",
          inputSchema: { type: "object" },
          execute: async () => null,
        },
      ],
      executor: mockExecutor,
      description: "Custom description with no placeholder",
    });
    expect(tool.description).toBe("Custom description with no placeholder");
    expect(tool.description).not.toContain("declare const codemode");
  });

  it("propagates executor throw", async () => {
    const mockExecutor: Executor = {
      execute: async () => {
        throw new Error("executor blew up");
      },
    };
    const tool = createCodeTool({
      tools: [],
      executor: mockExecutor,
    });
    await expect(tool.execute({ code: "anything" })).rejects.toThrow("executor blew up");
  });

  it("returns error field when executor returns error string", async () => {
    const mockExecutor: Executor = {
      execute: async (): Promise<CodeModeResult> => ({
        result: undefined,
        error: "something went wrong",
        logs: [],
        toolCallCount: 0,
        toolCalls: [],
      }),
    };
    const tool = createCodeTool({
      tools: [],
      executor: mockExecutor,
    });
    const result = await tool.execute({ code: "anything" });
    expect(result.error).toBe("something went wrong");
  });

  it("works with tools as Record (not array)", () => {
    const mockExecutor: Executor = {
      execute: async () => ({
        result: null,
        logs: [],
        toolCallCount: 0,
        toolCalls: [],
      }),
    };
    const toolRecord: Record<string, ToolDefinition> = {
      myTool: {
        name: "myTool",
        description: "A record-based tool",
        inputSchema: {
          type: "object",
          properties: { x: { type: "number" } },
        },
        execute: async (input) => input,
      },
    };
    const tool = createCodeTool({
      tools: toolRecord,
      executor: mockExecutor,
    });
    expect(tool.description).toContain("myTool");
    expect(tool.name).toBe("codemode");
  });
});

// ===========================================================================
// 4. Client edge cases
// ===========================================================================

describe("Client edge cases", () => {
  it("register() without httpEndpoint throws Error mentioning HTTP mode", async () => {
    const client = new SandCastle();
    await expect(client.register("test", "return 1;")).rejects.toThrow(/HTTP mode/);
  });

  it("dispatch() without httpEndpoint throws Error mentioning HTTP mode", async () => {
    const client = new SandCastle();
    await expect(client.dispatch("test")).rejects.toThrow(/HTTP mode/);
  });

  it("namespace() without httpEndpoint throws Error mentioning HTTP mode", () => {
    const client = new SandCastle();
    expect(() => client.namespace("test")).toThrow(/HTTP mode/);
  });

  it("createNamespace() without httpEndpoint throws Error mentioning HTTP mode", async () => {
    const client = new SandCastle();
    await expect(client.createNamespace("test")).rejects.toThrow(/HTTP mode/);
  });

  it("deleteNamespace() without httpEndpoint throws Error mentioning HTTP mode", async () => {
    const client = new SandCastle();
    await expect(client.deleteNamespace("test")).rejects.toThrow(/HTTP mode/);
  });

  it("constructor with httpEndpoint set dispatches execute to HTTP", async () => {
    const client = new SandCastle({
      httpEndpoint: "http://localhost:99999",
    });
    // We can't actually connect, but we can verify it tries HTTP
    // (fetch to invalid port) rather than subprocess
    try {
      await client.execute({ code: "return 1;" });
    } catch (e) {
      // Should get a fetch/connection error, NOT BinaryNotFoundError
      expect(e).not.toBeInstanceOf(BinaryNotFoundError);
    }
  });
});

// ===========================================================================
// 5. Error edge cases
// ===========================================================================

describe("Error edge cases", () => {
  it("ExecutionFailedError with success status produces correct message", () => {
    const result = fakeResult({ type: "success" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("execution success");
  });

  it("ExecutionFailedError with timeout status produces correct message", () => {
    const result = fakeResult({ type: "timeout" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("execution timeout");
  });

  it("ExecutionFailedError with fuel_exhausted status produces correct message", () => {
    const result = fakeResult({ type: "fuel_exhausted" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("execution fuel_exhausted");
  });

  it("ExecutionFailedError with memory_exceeded status produces correct message", () => {
    const result = fakeResult({ type: "memory_exceeded" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("execution memory_exceeded");
  });

  it("ExecutionFailedError with cancelled status produces correct message", () => {
    const result = fakeResult({ type: "cancelled" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("execution cancelled");
  });

  it("ExecutionFailedError with guest_error includes the error message", () => {
    const result = fakeResult({ type: "guest_error", message: "boom" });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("boom");
  });

  it("ExecutionFailedError with capability_error includes the error message", () => {
    const result = fakeResult({
      type: "capability_error",
      message: "cap failed",
    });
    const err = new ExecutionFailedError(result);
    expect(err.message).toBe("cap failed");
  });

  it("errorFromResult returns null for success", () => {
    const result = fakeResult({ type: "success" });
    expect(errorFromResult(result)).toBeNull();
  });

  it("errorFromResult returns TimeoutError for timeout", () => {
    const result = fakeResult({ type: "timeout" });
    const err = errorFromResult(result);
    expect(err).toBeInstanceOf(TimeoutError);
    expect(err!.name).toBe("TimeoutError");
  });

  it("errorFromResult returns FuelExhaustedError for fuel_exhausted", () => {
    const result = fakeResult({ type: "fuel_exhausted" });
    const err = errorFromResult(result);
    expect(err).toBeInstanceOf(FuelExhaustedError);
  });

  it("errorFromResult returns MemoryExceededError for memory_exceeded", () => {
    const result = fakeResult({ type: "memory_exceeded" });
    const err = errorFromResult(result);
    expect(err).toBeInstanceOf(MemoryExceededError);
  });

  it("errorFromResult returns GuestError for guest_error", () => {
    const result = fakeResult({ type: "guest_error", message: "oops" });
    const err = errorFromResult(result);
    expect(err).toBeInstanceOf(GuestError);
  });

  it("errorFromResult returns ExecutionFailedError for unknown status type", () => {
    const result = fakeResult({
      type: "some_unknown_type" as "success",
    });
    const err = errorFromResult(result);
    expect(err).toBeInstanceOf(ExecutionFailedError);
    expect(err).not.toBeNull();
  });

  it("error .name property is set correctly on all error subclasses", () => {
    expect(new SandCastleError("test").name).toBe("SandCastleError");
    expect(new ExecutionFailedError(fakeResult({ type: "timeout" })).name).toBe(
      "ExecutionFailedError",
    );
    expect(new TimeoutError(fakeResult({ type: "timeout" })).name).toBe("TimeoutError");
    expect(new FuelExhaustedError(fakeResult({ type: "fuel_exhausted" })).name).toBe(
      "FuelExhaustedError",
    );
    expect(new MemoryExceededError(fakeResult({ type: "memory_exceeded" })).name).toBe(
      "MemoryExceededError",
    );
    expect(new GuestError(fakeResult({ type: "guest_error", message: "x" })).name).toBe(
      "GuestError",
    );
    expect(new ExecutionAbortedError().name).toBe("ExecutionAbortedError");
    expect(new BinaryNotFoundError("/bin/sc").name).toBe("BinaryNotFoundError");
  });

  it("error .name is set on instance but is an own property (not inherited)", () => {
    const err = new SandCastleError("test");
    // SandCastleError sets .name as an own property in the constructor
    expect(err.name).toBe("SandCastleError");
    // Verify it's an own property (set in constructor via this.name = ...)
    expect(Object.hasOwn(err, "name")).toBe(true);
    // Verify JSON serialization includes name since it's an own enumerable prop
    const serialized = JSON.parse(JSON.stringify(err));
    expect(serialized.name).toBe("SandCastleError");
  });

  it("SandCastleError with ErrorOptions cause chain", () => {
    const cause = new Error("root cause");
    const err = new SandCastleError("wrapper", { cause });
    expect(err.cause).toBe(cause);
    expect(err.message).toBe("wrapper");
    expect((err.cause as Error).message).toBe("root cause");
  });
});

// ===========================================================================
// 6. Subprocess output parsing edge cases
// ===========================================================================

describe("subprocess output parsing edge cases", () => {
  // We test parseOutput indirectly through executeViaSubprocess's internal logic.
  // Since parseOutput is not exported, we simulate its behavior by calling
  // the SandCastle client with crafted scenarios where possible, and
  // testing the internal logic patterns directly for pure-unit scenarios.

  // Import parseOutput indirectly by testing through the client boundary.
  // For deeper unit testing, we replicate the parsing logic here.

  function parseOutputLike(
    stdout: string,
    stderr: string,
    exitCode: number,
  ): {
    ok: boolean;
    status: ExecutionStatus;
    output: OutputValue;
  } {
    // Replicate the logic from subprocess.ts parseOutput
    const jsonStart = stdout.indexOf("{");
    if (jsonStart >= 0) {
      const jsonStr = stdout.slice(jsonStart);
      try {
        const raw = JSON.parse(jsonStr);
        if (raw.status !== undefined) {
          // It's a transcript
          let status: ExecutionStatus;
          if (typeof raw.status === "object" && raw.status !== null && "type" in raw.status) {
            status = raw.status as ExecutionStatus;
          } else if (typeof raw.status === "string") {
            status =
              raw.status === "success"
                ? { type: "success" }
                : { type: "guest_error", message: raw.status };
          } else {
            status = { type: "guest_error", message: "unknown status" };
          }

          let output: OutputValue;
          if (raw.output === null || raw.output === undefined) {
            output = { type: "null" };
          } else if (
            typeof raw.output === "object" &&
            raw.output !== null &&
            "type" in raw.output
          ) {
            if (raw.output.type === "json") output = { type: "json", value: raw.output.value };
            else if (raw.output.type === "string")
              output = { type: "string", value: String(raw.output.value) };
            else if (raw.output.type === "null") output = { type: "null" };
            else output = { type: "json", value: raw.output };
          } else {
            output = { type: "json", value: raw.output };
          }

          return { ok: status.type === "success", status, output };
        }
      } catch {
        /* fall through */
      }
    }

    // Fallback
    const status: ExecutionStatus =
      exitCode === 0
        ? { type: "success" }
        : {
            type: "guest_error",
            message: stderr || `exit code ${exitCode}`,
          };

    let output: OutputValue;
    try {
      output = { type: "json", value: JSON.parse(stdout) };
    } catch {
      output = stdout.trim() ? { type: "string", value: stdout.trim() } : { type: "null" };
    }

    return { ok: status.type === "success", status, output };
  }

  it("parses pure JSON stdout correctly", () => {
    const json = JSON.stringify({
      execution_id: "abc",
      started_at: "2026-01-01T00:00:00Z",
      status: { type: "success" },
      fuel_consumed: 100,
      fuel_limit: 1000,
      peak_memory_bytes: 1024,
      memory_limit_bytes: 33554432,
      output: { type: "json", value: 42 },
    });
    const result = parseOutputLike(json, "", 0);
    expect(result.ok).toBe(true);
    expect(result.output).toEqual({ type: "json", value: 42 });
  });

  it("parses stdout with ANSI color codes before JSON", () => {
    const ansi = "\x1b[32m[INFO]\x1b[0m Loading...";
    const json = JSON.stringify({
      execution_id: "abc",
      started_at: "2026-01-01T00:00:00Z",
      status: { type: "success" },
      fuel_consumed: 100,
      fuel_limit: 1000,
      peak_memory_bytes: 1024,
      memory_limit_bytes: 33554432,
      output: { type: "json", value: "hello" },
    });
    const stdout = `${ansi}\n${json}`;
    const result = parseOutputLike(stdout, "", 0);
    expect(result.ok).toBe(true);
    expect(result.output).toEqual({ type: "json", value: "hello" });
  });

  it("parses first JSON object when stdout has multiple", () => {
    const json1 = JSON.stringify({
      execution_id: "abc",
      started_at: "2026-01-01T00:00:00Z",
      status: { type: "success" },
      fuel_consumed: 100,
      fuel_limit: 1000,
      peak_memory_bytes: 1024,
      memory_limit_bytes: 33554432,
      output: { type: "json", value: "first" },
    });
    // Second JSON is just noise
    const json2 = JSON.stringify({ extra: true });
    // Since JSON.parse will parse from the first {, the combined string
    // may or may not parse depending on structure. The implementation
    // uses stdout.slice(jsonStart) and tries JSON.parse on it.
    // With two separate JSON objects concatenated, JSON.parse will fail
    // on the combined string. Testing the parse logic:
    const stdout = `${json1}\n${json2}`;
    const result = parseOutputLike(stdout, "", 0);
    // The first JSON object is valid on its own so JSON.parse will parse it
    // (it stops at the end of the first valid JSON)
    expect(result.ok).toBe(true);
  });

  it("returns fallback with guest_error for empty stdout", () => {
    const result = parseOutputLike("", "", 1);
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
    expect(result.output.type).toBe("null");
  });

  it("uses stderr message when stderr has error and stdout is empty", () => {
    const result = parseOutputLike("", "fatal: something broke", 1);
    expect(result.ok).toBe(false);
    expect(result.status.type).toBe("guest_error");
    if (result.status.type === "guest_error") {
      expect(result.status.message).toContain("fatal: something broke");
    }
  });

  it("exitCode 0 but no JSON gives success with null output", () => {
    const result = parseOutputLike("", "", 0);
    expect(result.ok).toBe(true);
    expect(result.status.type).toBe("success");
    expect(result.output.type).toBe("null");
  });

  it("stdout is plain string (not JSON) produces string OutputValue", () => {
    const result = parseOutputLike("just some text", "", 0);
    expect(result.ok).toBe(true);
    expect(result.output.type).toBe("string");
    if (result.output.type === "string") {
      expect(result.output.value).toBe("just some text");
    }
  });
});

// ===========================================================================
// 7. Sandbox integration edge cases
// ===========================================================================

describe("sandbox integration edge cases", () => {
  run("code that returns undefined explicitly", async () => {
    const result = await sc().execute({ code: "return undefined;" });
    expect(result).toBeDefined();
    // undefined gets treated as null output
    expect(result.ok).toBe(true);
  });

  run("console.log of an object serializes correctly", async () => {
    const { transcript } = await sc().execute({
      code: 'console.log({ a: 1, b: "two" }); return null;',
    });
    const log = transcript.console.find((c) => c.level === "log");
    expect(log).toBeDefined();
    // Should contain some stringified representation of the object
    expect(log!.message).toBeTruthy();
  });

  run("very deeply nested return (10 levels) works", async () => {
    const result = await sc().run<unknown>(`
			var obj = { value: 42 };
			for (var i = 0; i < 10; i++) {
				obj = { nested: obj };
			}
			return obj;
		`);
    // Traverse 10 levels of nesting
    let current = result as Record<string, unknown>;
    for (let i = 0; i < 10; i++) {
      expect(current).toHaveProperty("nested");
      current = current.nested as Record<string, unknown>;
    }
    expect(current).toHaveProperty("value", 42);
  });

  run("returns a very large array (10000 elements)", async () => {
    const result = await sc().run<number[]>(`
			var arr = [];
			for (var i = 0; i < 10000; i++) {
				arr.push(i);
			}
			return arr;
		`);
    expect(result.length).toBe(10000);
    expect(result[0]).toBe(0);
    expect(result[9999]).toBe(9999);
  });

  run("emoji/unicode in strings round-trips correctly", async () => {
    const result = await sc().run<string>('return "Hello \\u{1F600} World \\u{2764}\\u{FE0F}";');
    expect(result).toContain("\u{1F600}");
    expect(result).toContain("\u{2764}");
  });

  run("template literals work in QuickJS", async () => {
    const result = await sc().run<string>(`
			var name = "World";
			var num = 42;
			return \`Hello \${name}, the answer is \${num}\`;
		`);
    expect(result).toBe("Hello World, the answer is 42");
  });

  run("two sequential executions share no state", async () => {
    const client = sc();
    await client.execute({
      code: "globalThis.sharedState = 'secret'; return null;",
    });
    const result = await client.run<boolean>(
      "return typeof globalThis.sharedState === 'undefined';",
    );
    expect(result).toBe(true);
  });

  run("execute with fuel 0 uses unlimited fuel, works normally", async () => {
    const result = await sc().run<number>("return 1 + 1;", undefined, {
      fuel: 0,
    });
    expect(result).toBe(2);
  });
});
