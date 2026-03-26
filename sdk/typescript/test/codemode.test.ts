import { describe, expect, it } from "bun:test";
import type { ToolDefinition } from "../src/codemode/index.js";
import {
  createCodeTool,
  generateTypes,
  normalizeCode,
  TwoPassExecutor,
} from "../src/codemode/index.js";

// ---------------------------------------------------------------------------
// normalizeCode
// ---------------------------------------------------------------------------

describe("normalizeCode", () => {
  it("strips markdown code fences", () => {
    const result = normalizeCode("```js\nconst x = 1;\nreturn x;\n```");
    expect(result).toContain("const x = 1");
    expect(result).not.toContain("```");
  });

  it("strips fences with language tag", () => {
    const result = normalizeCode("```typescript\nreturn 42;\n```");
    expect(result).toContain("return 42");
    expect(result).not.toContain("```");
  });

  it("passes through async arrow functions as-is", () => {
    const code = "async () => { return 42; }";
    expect(normalizeCode(code)).toBe(code);
  });

  it("converts async function to arrow", () => {
    const result = normalizeCode("async function run() { return 42; }");
    expect(result).toContain("async () =>");
    expect(result).toContain("return 42");
  });

  it("wraps bare statements in async arrow", () => {
    const result = normalizeCode("const x = 1;\nconst y = 2;\nreturn x + y;");
    expect(result).toContain("async () =>");
    expect(result).toContain("return x + y");
  });

  it("adds implicit return for trailing expression", () => {
    const result = normalizeCode("const x = 42\nx");
    expect(result).toContain("return (x)");
  });

  it("does not add return for lines ending with semicolons", () => {
    const result = normalizeCode("const x = 42;");
    expect(result).not.toContain("return (const");
  });

  it("handles empty string", () => {
    const result = normalizeCode("");
    expect(result).toContain("async () =>");
  });
});

// ---------------------------------------------------------------------------
// generateTypes
// ---------------------------------------------------------------------------

describe("generateTypes", () => {
  const tools: ToolDefinition[] = [
    {
      name: "getWeather",
      description: "Get weather for a location",
      inputSchema: {
        type: "object",
        properties: {
          location: { type: "string", description: "City name" },
          unit: { type: "string", enum: ["celsius", "fahrenheit"] },
        },
        required: ["location"],
      },
      execute: async () => ({}),
    },
    {
      name: "sendEmail",
      description: "Send an email",
      inputSchema: {
        type: "object",
        properties: {
          to: { type: "string" },
          subject: { type: "string" },
          body: { type: "string" },
        },
        required: ["to", "subject", "body"],
      },
      execute: async () => ({}),
    },
  ];

  it("generates input type for each tool", () => {
    const types = generateTypes(tools);
    expect(types).toContain("type GetWeatherInput");
    expect(types).toContain("type SendEmailInput");
  });

  it("generates codemode object declaration", () => {
    const types = generateTypes(tools);
    expect(types).toContain("declare const codemode:");
  });

  it("generates method signatures", () => {
    const types = generateTypes(tools);
    expect(types).toContain("getWeather(input: GetWeatherInput): Promise<unknown>");
    expect(types).toContain("sendEmail(input: SendEmailInput): Promise<unknown>");
  });

  it("includes JSDoc descriptions", () => {
    const types = generateTypes(tools);
    expect(types).toContain("/** Get weather for a location */");
    expect(types).toContain("/** Send an email */");
  });

  it("handles string enums", () => {
    const types = generateTypes(tools);
    expect(types).toMatch(/"celsius" \| "fahrenheit"/);
  });

  it("marks required properties without ?", () => {
    const types = generateTypes(tools);
    expect(types).toContain("location: string");
    expect(types).toContain("to: string");
  });

  it("marks optional properties with ?", () => {
    const types = generateTypes(tools);
    expect(types).toContain("unit?:");
  });

  it("handles array types", () => {
    const types = generateTypes([
      {
        name: "processList",
        description: "",
        inputSchema: {
          type: "object",
          properties: {
            items: { type: "array", items: { type: "number" } },
          },
          required: ["items"],
        },
        execute: async () => ({}),
      },
    ]);
    expect(types).toContain("number[]");
  });

  it("handles nested objects", () => {
    const types = generateTypes([
      {
        name: "createUser",
        description: "",
        inputSchema: {
          type: "object",
          properties: {
            name: { type: "string" },
            address: {
              type: "object",
              properties: {
                city: { type: "string" },
                zip: { type: "string" },
              },
              required: ["city"],
            },
          },
          required: ["name"],
        },
        execute: async () => ({}),
      },
    ]);
    expect(types).toContain("city: string");
  });

  it("accepts record-style tool definitions", () => {
    const types = generateTypes({
      myTool: {
        name: "myTool",
        description: "A tool",
        inputSchema: { type: "object", properties: { x: { type: "number" } }, required: ["x"] },
        execute: async () => ({}),
      },
    });
    expect(types).toContain("type MyToolInput");
    expect(types).toContain("myTool(input: MyToolInput)");
  });
});

// ---------------------------------------------------------------------------
// createCodeTool
// ---------------------------------------------------------------------------

describe("createCodeTool", () => {
  it("returns a tool named codemode", () => {
    const tool = createCodeTool({
      tools: [],
      executor: {
        execute: async () => ({ result: null, logs: [], toolCallCount: 0, toolCalls: [] }),
      },
    });
    expect(tool.name).toBe("codemode");
  });

  it("description includes generated types", () => {
    const tool = createCodeTool({
      tools: [
        {
          name: "greet",
          description: "Greet someone",
          inputSchema: {
            type: "object",
            properties: { name: { type: "string" } },
            required: ["name"],
          },
          execute: async () => ({}),
        },
      ],
      executor: {
        execute: async () => ({ result: null, logs: [], toolCallCount: 0, toolCalls: [] }),
      },
    });
    expect(tool.description).toContain("greet(input: GreetInput)");
    expect(tool.description).toContain("Greet someone");
  });

  it("input schema requires code string", () => {
    const tool = createCodeTool({
      tools: [],
      executor: {
        execute: async () => ({ result: null, logs: [], toolCallCount: 0, toolCalls: [] }),
      },
    });
    expect(tool.inputSchema.type).toBe("object");
    expect(tool.inputSchema.required).toEqual(["code"]);
    expect(tool.inputSchema.properties.code.type).toBe("string");
  });

  it("execute calls the executor", async () => {
    let executedCode = "";
    const tool = createCodeTool({
      tools: [],
      executor: {
        execute: async (code) => {
          executedCode = code;
          return { result: 42, logs: [], toolCallCount: 0, toolCalls: [] };
        },
      },
    });

    const result = await tool.execute({ code: "return 42;" });
    expect(result.result).toBe(42);
    expect(executedCode).toContain("return 42");
  });

  it("custom description with {{types}} placeholder", () => {
    const tool = createCodeTool({
      tools: [
        {
          name: "foo",
          description: "",
          inputSchema: { type: "object", properties: { x: { type: "number" } }, required: ["x"] },
          execute: async () => ({}),
        },
      ],
      executor: {
        execute: async () => ({ result: null, logs: [], toolCallCount: 0, toolCalls: [] }),
      },
      description: "Custom prefix.\n{{types}}\nCustom suffix.",
    });
    expect(tool.description).toContain("Custom prefix.");
    expect(tool.description).toContain("Custom suffix.");
    expect(tool.description).toContain("type FooInput");
  });
});

// ---------------------------------------------------------------------------
// TwoPassExecutor integration (requires binary)
// ---------------------------------------------------------------------------

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

let hasBinary = false;
try {
  hasBinary = Bun.file(BINARY_PATH).size > 0;
} catch {
  /* not built */
}
const run = hasBinary ? it : it.skip;

describe("TwoPassExecutor", () => {
  run("executes code without tool calls", async () => {
    const executor = new TwoPassExecutor({
      binaryPath: BINARY_PATH,
      guestModule: GUEST_MODULE,
    });

    const result = await executor.execute("return 1 + 1;", {});
    expect(result.result).toBe(2);
    expect(result.toolCallCount).toBe(0);
  });

  run("executes code with tool calls via two-pass", async () => {
    const executor = new TwoPassExecutor({
      binaryPath: BINARY_PATH,
      guestModule: GUEST_MODULE,
    });

    const result = await executor.execute(
      `async () => {
        const weather = await codemode.getWeather({ location: "SF" });
        return { weather };
      }`,
      {
        getWeather: async (input: unknown) => {
          const { location } = input as { location: string };
          return { temp: 72, condition: "sunny", location };
        },
      },
    );

    expect(result.toolCallCount).toBe(1);
    expect(result.toolCalls[0].tool).toBe("getWeather");
    const weather = (result.result as { weather: { temp: number } }).weather;
    expect(weather.temp).toBe(72);
  });

  run("handles multiple tool calls", async () => {
    const executor = new TwoPassExecutor({
      binaryPath: BINARY_PATH,
      guestModule: GUEST_MODULE,
    });

    const result = await executor.execute(
      `async () => {
        const a = await codemode.add({ x: 1, y: 2 });
        const b = await codemode.add({ x: 3, y: 4 });
        return { a, b };
      }`,
      {
        add: async (input: unknown) => {
          const { x, y } = input as { x: number; y: number };
          return x + y;
        },
      },
    );

    expect(result.toolCallCount).toBe(2);
    const r = result.result as { a: number; b: number };
    expect(r.a).toBe(3);
    expect(r.b).toBe(7);
  });

  run("captures console output", async () => {
    const executor = new TwoPassExecutor({
      binaryPath: BINARY_PATH,
      guestModule: GUEST_MODULE,
    });

    const result = await executor.execute('console.log("hello from codemode"); return "done";', {});
    expect(result.logs.some((l) => l.includes("hello from codemode"))).toBe(true);
  });

  run("reports tool execution errors", async () => {
    const executor = new TwoPassExecutor({
      binaryPath: BINARY_PATH,
      guestModule: GUEST_MODULE,
    });

    const result = await executor.execute(
      `async () => {
        const r = await codemode.failingTool({ x: 1 });
        return r;
      }`,
      {
        failingTool: async () => {
          throw new Error("tool exploded");
        },
      },
    );

    expect(result.toolCalls[0].error).toBe("tool exploded");
  });
});
