/**
 * Agent integration tests.
 *
 * These tests wire up Claude (via the Anthropic API) with SandCastle as a
 * tool, proving the end-to-end agent → sandbox → result loop works.
 *
 * Requires:
 *   - ANTHROPIC_API_KEY in the environment (or sourced from ../../.env)
 *   - The sandcastle release binary + guest WASM to be built
 *
 * Skipped automatically when either prerequisite is missing.
 */
import { describe, expect, it } from "bun:test";
import { SandCastle } from "../src/index.js";

// ---------------------------------------------------------------------------
// Prerequisites
// ---------------------------------------------------------------------------

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

// Source .env if key isn't already in environment
let apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
  try {
    const envFile = await Bun.file("../../.env").text();
    for (const line of envFile.split("\n")) {
      const match = line.match(/^ANTHROPIC_API_KEY=(.+)$/);
      if (match) {
        apiKey = match[1].trim();
        break;
      }
    }
  } catch {
    /* no .env file */
  }
}

let hasBinary = false;
try {
  hasBinary = Bun.file(BINARY_PATH).size > 0;
} catch {
  /* not built */
}

const canRun = !!apiKey && hasBinary;
const run = canRun ? it : it.skip;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

interface ToolUseBlock {
  type: "tool_use";
  id: string;
  name: string;
  input: Record<string, unknown>;
}

interface TextBlock {
  type: "text";
  text: string;
}

type ContentBlock = ToolUseBlock | TextBlock;

interface Message {
  id: string;
  role: string;
  content: ContentBlock[];
  stop_reason: string | null;
}

const sandbox = new SandCastle({
  binaryPath: BINARY_PATH,
  guestModule: GUEST_MODULE,
});

/** Call the Anthropic Messages API. */
async function chat(
  messages: Array<{ role: string; content: unknown }>,
  tools: unknown[],
): Promise<Message> {
  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey!,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify({
      model: "claude-haiku-4-5-20251001",
      max_tokens: 1024,
      messages,
      tools,
    }),
  });
  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Anthropic API ${res.status}: ${body}`);
  }
  return (await res.json()) as Message;
}

/** The SandCastle tool definition given to Claude. */
const SANDBOX_TOOL = {
  name: "run_code",
  description:
    "Execute JavaScript code in a secure WASM sandbox. " +
    "The code runs in QuickJS with no network or filesystem access. " +
    "Use `globalThis.__sandcastle_input` to read the JSON input. " +
    "The code must `return` a value — that value is the tool result. " +
    "console.log output is captured but not returned.",
  input_schema: {
    type: "object",
    properties: {
      code: {
        type: "string",
        description: "JavaScript source code to execute. Must use `return` to produce output.",
      },
      input: {
        description: "JSON input available as globalThis.__sandcastle_input inside the sandbox.",
      },
    },
    required: ["code"],
  },
};

/**
 * Run a single-turn agent loop:
 *   user prompt → Claude → tool_use → sandbox → tool_result → Claude → final answer
 */
async function agentRun(
  userPrompt: string,
  maxTurns = 3,
): Promise<{ finalText: string; toolCalls: number }> {
  const messages: Array<{ role: string; content: unknown }> = [
    { role: "user", content: userPrompt },
  ];

  let toolCalls = 0;

  for (let turn = 0; turn < maxTurns; turn++) {
    const response = await chat(messages, [SANDBOX_TOOL]);

    // Append assistant response
    messages.push({ role: "assistant", content: response.content });

    // If no tool use, we're done
    if (response.stop_reason !== "tool_use") {
      const textBlocks = response.content.filter((b): b is TextBlock => b.type === "text");
      return {
        finalText: textBlocks.map((b) => b.text).join("\n"),
        toolCalls,
      };
    }

    // Execute each tool call in the sandbox
    const toolUses = response.content.filter((b): b is ToolUseBlock => b.type === "tool_use");

    const toolResults = [];
    for (const tu of toolUses) {
      toolCalls++;
      if (tu.name !== "run_code") {
        toolResults.push({
          type: "tool_result",
          tool_use_id: tu.id,
          content: `Unknown tool: ${tu.name}`,
          is_error: true,
        });
        continue;
      }

      const { code, input } = tu.input as { code: string; input?: unknown };
      const result = await sandbox.execute({ code, input });

      let content: string;
      if (result.ok && result.output.type === "json") {
        content = JSON.stringify(result.output.value);
      } else if (result.ok && result.output.type === "string") {
        content = result.output.value;
      } else if (!result.ok) {
        const msg = "message" in result.status ? result.status.message : result.status.type;
        content = `Sandbox error: ${msg}`;
      } else {
        content = "null";
      }

      toolResults.push({
        type: "tool_result",
        tool_use_id: tu.id,
        content,
      });
    }

    messages.push({ role: "user", content: toolResults });
  }

  throw new Error("Agent loop exceeded max turns");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("agent integration (Claude + SandCastle)", () => {
  run(
    "Claude uses sandbox to compute arithmetic",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        "What is 17 * 31 + 42? Use the run_code tool to compute this.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // 17 * 31 + 42 = 569
      expect(finalText).toContain("569");
    },
    30_000,
  );

  run(
    "Claude uses sandbox to transform data",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        'I have this data: [{"name":"Alice","score":85},{"name":"Bob","score":92},{"name":"Charlie","score":78}]. ' +
          "Use the run_code tool to find who has the highest score and return their name.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      expect(finalText).toContain("Bob");
    },
    30_000,
  );

  run(
    "Claude uses sandbox to sort and filter",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        "Use the run_code tool to generate the first 10 Fibonacci numbers and return them as an array.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Should contain some fibonacci numbers
      expect(finalText).toMatch(/1.*1.*2.*3.*5.*8/);
    },
    30_000,
  );

  run(
    "Claude handles sandbox errors gracefully",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        "Use the run_code tool to run this exact code: `undefinedVariable.foo()`. " +
          "Then explain what went wrong.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Claude should explain the error rather than crashing
      expect(finalText.length).toBeGreaterThan(10);
    },
    30_000,
  );

  run(
    "Claude writes and executes multi-step code",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        "Use the run_code tool to: 1) create an array of 5 random-looking objects with name and age fields, " +
          "2) filter to only those with age > 25, 3) sort by age descending, " +
          "4) return the result. Tell me what you get.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      expect(finalText.length).toBeGreaterThan(20);
    },
    30_000,
  );

  run(
    "Claude passes input to the sandbox correctly",
    async () => {
      const { finalText, toolCalls } = await agentRun(
        'Use the run_code tool with the input {"x": 7, "y": 3} and write code that reads ' +
          "globalThis.__sandcastle_input and returns x raised to the power of y.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // 7^3 = 343
      expect(finalText).toContain("343");
    },
    30_000,
  );
});
