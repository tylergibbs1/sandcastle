/**
 * Real-world agent scenarios.
 *
 * Each test simulates a realistic AI agent task: the kind of thing
 * a LangChain/CrewAI agent would actually do in production. Claude
 * receives a task, writes JavaScript, executes it in a SandCastle
 * sandbox, and returns a result the test validates.
 *
 * These are NOT canned — Claude decides what code to write each run.
 */
import { describe, expect, it } from "bun:test";
import { SandCastle } from "../src/index.js";

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

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
    /* no .env */
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
// Agent harness
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

const sandbox = new SandCastle({ binaryPath: BINARY_PATH, guestModule: GUEST_MODULE });

async function chat(
  messages: Array<{ role: string; content: unknown }>,
  tools: unknown[],
  system?: string,
): Promise<Message> {
  const body: Record<string, unknown> = {
    model: "claude-haiku-4-5-20251001",
    max_tokens: 2048,
    messages,
    tools,
  };
  if (system) body.system = system;

  const res = await fetch("https://api.anthropic.com/v1/messages", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey!,
      "anthropic-version": "2023-06-01",
    },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`Anthropic API ${res.status}: ${text}`);
  }
  return (await res.json()) as Message;
}

const SANDBOX_TOOL = {
  name: "run_code",
  description:
    "Execute JavaScript code in a secure WASM sandbox (QuickJS engine). " +
    "The sandbox has no network, no filesystem, no npm packages. " +
    "Read input via `globalThis.__sandcastle_input`. " +
    "You MUST use `return <value>` as the last statement to produce output. " +
    "Available globals: JSON, Math, Date, console, TextEncoder, TextDecoder, atob, btoa. " +
    "No fetch, no URL, no crypto, no setTimeout, no require/import.",
  input_schema: {
    type: "object",
    properties: {
      code: {
        type: "string",
        description: "JavaScript source code. Must end with a `return` statement.",
      },
      input: {
        description: "JSON-serializable input, accessible as `globalThis.__sandcastle_input`.",
      },
    },
    required: ["code"],
  },
};

async function agentRun(
  prompt: string,
  system?: string,
  maxTurns = 5,
): Promise<{ text: string; toolCalls: number; allToolResults: string[] }> {
  const messages: Array<{ role: string; content: unknown }> = [
    { role: "user", content: prompt },
  ];

  let toolCalls = 0;
  const allToolResults: string[] = [];

  for (let turn = 0; turn < maxTurns; turn++) {
    const response = await chat(messages, [SANDBOX_TOOL], system);
    messages.push({ role: "assistant", content: response.content });

    if (response.stop_reason !== "tool_use") {
      const text = response.content
        .filter((b): b is TextBlock => b.type === "text")
        .map((b) => b.text)
        .join("\n");
      return { text, toolCalls, allToolResults };
    }

    const toolUses = response.content.filter((b): b is ToolUseBlock => b.type === "tool_use");
    const toolResults = [];

    for (const tu of toolUses) {
      toolCalls++;
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

      allToolResults.push(content);
      toolResults.push({ type: "tool_result", tool_use_id: tu.id, content });
    }

    messages.push({ role: "user", content: toolResults });
  }

  throw new Error("Agent loop exceeded max turns");
}

// ---------------------------------------------------------------------------
// Real-world scenarios
// ---------------------------------------------------------------------------

describe("real-world agent scenarios", () => {
  // Scenario 1: Sales data pipeline
  // An agent ingests raw CSV-like sales data, cleans it, computes metrics,
  // and produces a summary report — a common BI/analytics agent task.
  run(
    "sales analytics pipeline",
    async () => {
      const salesData = [
        { rep: "Alice", region: "West", q1: 50000, q2: 62000, q3: 58000, q4: 71000 },
        { rep: "Bob", region: "East", q1: 45000, q2: 48000, q3: 51000, q4: 53000 },
        { rep: "Charlie", region: "West", q1: 60000, q2: 55000, q3: 63000, q4: 68000 },
        { rep: "Diana", region: "East", q1: 70000, q2: 75000, q3: 72000, q4: 80000 },
        { rep: "Eve", region: "Central", q1: 40000, q2: 42000, q3: 44000, q4: 46000 },
      ];

      const { text, toolCalls, allToolResults } = await agentRun(
        "I have quarterly sales data for 5 reps. Using the run_code tool, analyze this data and tell me:\n" +
          "1. Total annual revenue across all reps\n" +
          "2. The top performer by total annual sales\n" +
          "3. Which region had the highest total sales\n" +
          "4. The quarter with the highest overall sales\n" +
          "Pass the data as input to the tool.",
        `You are a data analyst agent. Always use the run_code tool to perform calculations. Pass the sales data as the 'input' parameter, then read it via globalThis.__sandcastle_input in your code.

Here is the data to pass as input: ${JSON.stringify(salesData)}`,
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Diana has the highest total: 70k+75k+72k+80k = 297k
      expect(text).toContain("Diana");
      // Total across all reps: 241k+197k+246k+297k+172k = 1,153,000
      expect(text).toMatch(/1[,.]?153[,.]?000|1153/);
    },
    60_000,
  );

  // Scenario 2: JSON schema validation
  // An agent validates user-submitted data against business rules —
  // a common workflow automation / form processing task.
  run(
    "data validation agent",
    async () => {
      const submissions = [
        { id: 1, email: "alice@example.com", age: 28, plan: "pro" },
        { id: 2, email: "not-an-email", age: 17, plan: "enterprise" },
        { id: 3, email: "bob@test.com", age: 35, plan: "free" },
        { id: 4, email: "charlie@co.uk", age: -5, plan: "pro" },
        { id: 5, email: "diana@example.com", age: 42, plan: "invalid_plan" },
      ];

      const { text, toolCalls, allToolResults } = await agentRun(
        "I have 5 user registration submissions. Using the run_code tool, validate each one against these rules:\n" +
          "- email must contain '@' and '.'\n" +
          "- age must be >= 18 and <= 120\n" +
          "- plan must be one of: 'free', 'pro', 'enterprise'\n\n" +
          "Return which submissions are valid and which are invalid (with reasons).\n\n" +
          `Submissions: ${JSON.stringify(submissions)}`,
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // ID 1 and 3 are valid; 2 (bad email + underage), 4 (negative age), 5 (invalid plan) are invalid
      expect(text).toMatch(/[Ii]nvalid|[Ff]ail/);
      // Should identify the specific issues
      const lower = text.toLowerCase();
      expect(lower).toMatch(/age|email|plan/);
    },
    60_000,
  );

  // Scenario 3: Multi-step text processing
  // An agent parses, transforms, and reformats structured text —
  // a common ETL / content processing agent task.
  run(
    "log parsing and anomaly detection",
    async () => {
      const logs = [
        "2026-03-25T10:00:00Z INFO  request_handler: GET /api/users 200 12ms",
        "2026-03-25T10:00:01Z INFO  request_handler: POST /api/orders 201 45ms",
        "2026-03-25T10:00:02Z WARN  request_handler: GET /api/products 200 2501ms",
        "2026-03-25T10:00:03Z ERROR request_handler: POST /api/payments 500 89ms",
        "2026-03-25T10:00:04Z INFO  request_handler: GET /api/users 200 15ms",
        "2026-03-25T10:00:05Z INFO  request_handler: GET /api/orders 200 22ms",
        "2026-03-25T10:00:06Z ERROR request_handler: DELETE /api/users/42 403 5ms",
        "2026-03-25T10:00:07Z INFO  request_handler: PUT /api/users/1 200 3200ms",
        "2026-03-25T10:00:08Z INFO  request_handler: GET /api/health 200 2ms",
        "2026-03-25T10:00:09Z ERROR request_handler: POST /api/upload 413 1ms",
      ];

      const { text, toolCalls } = await agentRun(
        "I have application logs. Using the run_code tool, parse these logs and report:\n" +
          "1. Total number of requests\n" +
          "2. Count by status code (how many 200s, 500s, etc.)\n" +
          "3. Average response time in ms\n" +
          "4. Any anomalies (responses > 2000ms or error status codes)\n\n" +
          `Logs:\n${logs.join("\n")}`,
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Should report 10 total requests
      expect(text).toContain("10");
      // Should identify the slow requests (2501ms, 3200ms) and errors (500, 403, 413)
      expect(text).toMatch(/500|error|anomal/i);
      expect(text).toMatch(/2501|3200|slow/i);
    },
    60_000,
  );

  // Scenario 4: Algorithm implementation
  // An agent implements a non-trivial algorithm from a description —
  // tests that Claude can write correct, multi-step code in the sandbox.
  run(
    "implement and run a scheduling algorithm",
    async () => {
      const tasks = [
        { name: "Deploy API", duration: 3, deadline: 10, priority: 1 },
        { name: "Write tests", duration: 5, deadline: 8, priority: 2 },
        { name: "Code review", duration: 2, deadline: 5, priority: 1 },
        { name: "Update docs", duration: 1, deadline: 12, priority: 3 },
        { name: "Fix bug #42", duration: 4, deadline: 6, priority: 1 },
        { name: "Refactor auth", duration: 6, deadline: 15, priority: 2 },
      ];

      const { text, toolCalls, allToolResults } = await agentRun(
        "I have a list of engineering tasks with durations, deadlines, and priorities (1=highest). " +
          "Using the run_code tool, implement a greedy scheduling algorithm that:\n" +
          "1. Sorts tasks by priority (ascending), then by deadline (ascending) for ties\n" +
          "2. Schedules each task sequentially (start time = end time of previous task, starting at t=0)\n" +
          "3. Flags any task that would miss its deadline\n" +
          "4. Returns the full schedule with start time, end time, and on-time status for each task\n\n" +
          `Tasks: ${JSON.stringify(tasks)}`,
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // The result should have a schedule with all 6 tasks
      const resultStr = allToolResults.join(" ");
      // Should contain the task names in the output
      expect(resultStr).toMatch(/Deploy|Code review|Fix bug/);
      // Claude should report which tasks are on time vs late
      expect(text).toMatch(/on.?time|late|miss|deadline/i);
    },
    60_000,
  );

  // Scenario 5: Multi-turn iterative refinement
  // An agent writes code, sees an error or unexpected result, and
  // self-corrects — the core value prop of code execution in agent loops.
  run(
    "multi-turn self-correction on bad data",
    async () => {
      const messyData = {
        users: [
          { name: "Alice", balance: "1,234.56" },
          { name: "Bob", balance: "$2,345.67" },
          { name: "Charlie", balance: "invalid" },
          { name: "Diana", balance: "  3456.78  " },
          { name: null, balance: "100.00" },
        ],
      };

      const { text, toolCalls } = await agentRun(
        "I have messy user data with inconsistent balance formats. Using the run_code tool, " +
          "clean this data: parse the balance strings into numbers (strip $, commas, whitespace), " +
          "handle invalid values by setting them to 0, handle null names by setting them to 'Unknown', " +
          "then return the cleaned data sorted by balance descending, plus the total of all balances.\n\n" +
          `Data: ${JSON.stringify(messyData)}`,
        "You are a data cleaning agent. If your code throws an error, read the error message, " +
          "fix your code, and try again. Always pass the data as the input parameter.",
      );

      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Should have cleaned and totaled the balances
      // Alice: 1234.56, Bob: 2345.67, Charlie: 0, Diana: 3456.78, Unknown: 100
      // Total: 7137.01
      const lower = text.toLowerCase();
      expect(lower).toMatch(/diana|3456|sorted|total/);
      // Should have handled the null name
      expect(lower).toMatch(/unknown|null|missing/);
    },
    60_000,
  );
});
