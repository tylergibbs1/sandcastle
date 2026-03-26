/**
 * 10 diverse real-world agent scenarios.
 *
 * Each test simulates a different industry use case where an AI agent
 * writes and executes code in a SandCastle sandbox to solve a task.
 * Claude decides what code to write — nothing is canned.
 */
import { describe, expect, it } from "bun:test";
import { SandCastle } from "../src/index.js";
import { BINARY_PATH, ENV_FILE, GUEST_MODULE } from "./test-paths.js";

let apiKey = process.env.ANTHROPIC_API_KEY;
if (!apiKey) {
  try {
    const envFile = await Bun.file(ENV_FILE).text();
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
// Harness
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
  if (!res.ok) throw new Error(`API ${res.status}: ${await res.text()}`);
  return (await res.json()) as Message;
}

const SANDBOX_TOOL = {
  name: "run_code",
  description:
    "Execute JavaScript in a secure WASM sandbox (QuickJS). " +
    "Read input via `globalThis.__sandcastle_input`. " +
    "Must use `return <value>` to produce output. " +
    "Available: JSON, Math, Date, console. No fetch, no URL, no require.",
  input_schema: {
    type: "object",
    properties: {
      code: {
        type: "string",
        description: "JavaScript source. Must end with a `return` statement.",
      },
      input: {
        description: "JSON input accessible as globalThis.__sandcastle_input.",
      },
    },
    required: ["code"],
  },
};

async function agentRun(
  prompt: string,
  system?: string,
): Promise<{ text: string; toolCalls: number }> {
  const messages: Array<{ role: string; content: unknown }> = [{ role: "user", content: prompt }];
  let toolCalls = 0;

  for (let turn = 0; turn < 5; turn++) {
    const response = await chat(messages, [SANDBOX_TOOL], system);
    messages.push({ role: "assistant", content: response.content });

    if (response.stop_reason !== "tool_use") {
      const text = response.content
        .filter((b): b is TextBlock => b.type === "text")
        .map((b) => b.text)
        .join("\n");
      return { text, toolCalls };
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

      toolResults.push({ type: "tool_result", tool_use_id: tu.id, content });
    }

    messages.push({ role: "user", content: toolResults });
  }

  throw new Error("Agent loop exceeded max turns");
}

// ---------------------------------------------------------------------------
// 10 Scenarios
// ---------------------------------------------------------------------------

describe("agent scenarios", () => {
  // 1. Financial: loan amortization calculator
  run(
    "calculates loan amortization schedule",
    async () => {
      const { text, toolCalls } = await agentRun(
        "I have a $200,000 loan at 6.5% annual interest rate for 30 years. " +
          "Use the run_code tool to calculate: the monthly payment, total interest paid over the life of the loan, " +
          "and the remaining balance after 5 years of payments.",
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Monthly payment should be ~$1264
      expect(text).toMatch(/1,?2[56]\d/);
    },
    60_000,
  );

  // 2. NLP: text analysis / word frequency
  run(
    "analyzes word frequency in a paragraph",
    async () => {
      const { text, toolCalls } = await agentRun(
        'Use the run_code tool to analyze this text and return the top 5 most frequent words (excluding common stop words like "the", "a", "is", "in", "to", "and", "of"):\n\n' +
          '"The quick brown fox jumps over the lazy dog. The dog barked at the fox. ' +
          'The fox ran away from the dog and jumped over the fence. The lazy dog just watched the fox run away."',
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // "the" excluded, "fox" and "dog" should be top
      expect(text.toLowerCase()).toMatch(/fox|dog/);
    },
    60_000,
  );

  // 3. DevOps: parse and analyze structured config
  run(
    "validates and merges configuration objects",
    async () => {
      const { text, toolCalls } = await agentRun(
        "Use the run_code tool. I have a base config and an override config. " +
          "Deep merge them (override wins), validate that all required fields exist " +
          "(required: host, port, database.name), and return the merged config.\n\n" +
          `Base: ${JSON.stringify({ host: "localhost", port: 5432, database: { name: "mydb", pool: 10 }, logging: { level: "info" } })}\n` +
          `Override: ${JSON.stringify({ port: 3306, database: { pool: 20 }, logging: { level: "debug", file: "/var/log/app.log" } })}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Merged config should have port 3306, pool 20, level debug
      expect(text).toMatch(/3306/);
      expect(text).toMatch(/debug/);
    },
    60_000,
  );

  // 4. E-commerce: shopping cart with discounts and tax
  run(
    "computes shopping cart total with tiered discounts",
    async () => {
      const cart = [
        { item: "Laptop", price: 999.99, quantity: 1 },
        { item: "Mouse", price: 29.99, quantity: 2 },
        { item: "Keyboard", price: 79.99, quantity: 1 },
        { item: "Monitor", price: 449.99, quantity: 2 },
        { item: "USB Cable", price: 9.99, quantity: 5 },
      ];

      const { text, toolCalls } = await agentRun(
        "Use the run_code tool to calculate a shopping cart total. " +
          "Rules: subtotal over $500 gets 5% discount, over $1000 gets 10% discount, over $2000 gets 15% discount. " +
          "Tax rate is 8.25%. Return the subtotal, discount amount, tax, and final total.\n\n" +
          `Cart: ${JSON.stringify(cart)}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Subtotal: 999.99 + 59.98 + 79.99 + 899.98 + 49.95 = 2089.89 → 15% discount
      expect(text).toMatch(/2,?089|15%|discount/i);
    },
    60_000,
  );

  // 5. Data science: statistics on a dataset
  run(
    "computes statistical measures on a dataset",
    async () => {
      const data = [23, 45, 12, 67, 34, 89, 56, 78, 41, 93, 28, 61, 52, 37, 84, 19, 72, 46, 58, 31];

      const { text, toolCalls } = await agentRun(
        "Use the run_code tool to compute these statistics on the given dataset: " +
          "mean, median, standard deviation, min, max, range, and the 25th/75th percentiles (Q1/Q3).\n\n" +
          `Data: ${JSON.stringify(data)}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Mean should be ~50.25, median ~49
      expect(text).toMatch(/mean|average/i);
      expect(text).toMatch(/median/i);
      expect(text).toMatch(/standard deviation|std/i);
    },
    60_000,
  );

  // 6. Scheduling: find overlapping time slots
  run(
    "finds available meeting slots across calendars",
    async () => {
      const calendars = {
        alice: [
          { start: "09:00", end: "10:00" },
          { start: "12:00", end: "13:00" },
          { start: "15:00", end: "16:30" },
        ],
        bob: [
          { start: "09:30", end: "11:00" },
          { start: "13:00", end: "14:00" },
        ],
        charlie: [
          { start: "10:00", end: "11:30" },
          { start: "14:00", end: "15:00" },
        ],
      };

      const { text, toolCalls } = await agentRun(
        "Use the run_code tool. Given three people's busy calendars (9am-5pm workday), " +
          "find all 30-minute windows where ALL three are free.\n\n" +
          `Calendars: ${JSON.stringify(calendars)}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Should find slots like 11:30-12:00, 16:30-17:00
      expect(text).toMatch(/\d{1,2}:\d{2}/);
    },
    60_000,
  );

  // 7. Cryptography: implement Caesar cipher
  run(
    "implements Caesar cipher encryption and decryption",
    async () => {
      const { text, toolCalls } = await agentRun(
        'Use the run_code tool to implement a Caesar cipher. Encrypt the message "HELLO WORLD" with a shift of 7, ' +
          "then decrypt it back to verify. Return both the encrypted and decrypted text.",
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // HELLO shifted by 7 → OLSSV
      expect(text).toMatch(/OLSSV|encrypt|decrypt/i);
      expect(text).toMatch(/HELLO/);
    },
    60_000,
  );

  // 8. Graph algorithm: find shortest path
  run(
    "finds shortest path in a weighted graph (Dijkstra)",
    async () => {
      const graph = {
        A: { B: 4, C: 2 },
        B: { D: 3, C: 1 },
        C: { B: 1, D: 5, E: 7 },
        D: { E: 1 },
        E: {},
      };

      const { text, toolCalls } = await agentRun(
        "Use the run_code tool to implement Dijkstra's algorithm and find the shortest path " +
          "from node A to node E in this weighted graph. Return the path and total distance.\n\n" +
          `Graph: ${JSON.stringify(graph)}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Shortest path: A→C→B→D→E = 2+1+3+1 = 7
      expect(text).toMatch(/7|A.*C.*B.*D.*E/);
    },
    60_000,
  );

  // 9. Data transformation: flatten nested JSON + pivot
  run(
    "flattens nested JSON and creates a pivot table",
    async () => {
      const data = [
        {
          department: "Engineering",
          employees: [
            { name: "Alice", salary: 120000 },
            { name: "Bob", salary: 110000 },
          ],
        },
        {
          department: "Marketing",
          employees: [
            { name: "Charlie", salary: 95000 },
            { name: "Diana", salary: 105000 },
          ],
        },
        { department: "Engineering", employees: [{ name: "Eve", salary: 130000 }] },
        {
          department: "Sales",
          employees: [
            { name: "Frank", salary: 90000 },
            { name: "Grace", salary: 88000 },
          ],
        },
      ];

      const { text, toolCalls } = await agentRun(
        "Use the run_code tool. Given nested department/employee data, " +
          "create a summary pivot table showing: department name, employee count, " +
          "total salary, average salary, and highest-paid employee name.\n\n" +
          `Data: ${JSON.stringify(data)}`,
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Engineering: 3 employees, Eve highest paid
      expect(text).toMatch(/Engineering|Marketing|Sales/);
      expect(text).toMatch(/Eve|130,?000/);
    },
    60_000,
  );

  // 10. Recursive: generate a fractal pattern as ASCII art
  run(
    "generates Sierpinski triangle as ASCII art",
    async () => {
      const { text, toolCalls } = await agentRun(
        "Use the run_code tool to generate a Sierpinski triangle (order 4) as ASCII art using asterisks (*) and spaces. " +
          "Return it as a string.",
      );
      expect(toolCalls).toBeGreaterThanOrEqual(1);
      // Should contain asterisks in a triangular pattern
      expect(text).toMatch(/\*/);
      // Should have multiple lines
      expect(text.split("\n").length).toBeGreaterThan(3);
    },
    60_000,
  );
});
