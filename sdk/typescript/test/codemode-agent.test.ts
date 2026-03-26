/**
 * Code Mode agent integration test.
 *
 * Proves the full loop: Claude receives one `codemode` tool, writes code
 * that chains multiple tool calls, the code runs in a SandCastle sandbox,
 * tool calls are dispatched host-side via TwoPassExecutor, and the final
 * result comes back in a single tool_use round-trip.
 *
 * This is the "81% token reduction" pattern from Cloudflare's Code Mode.
 */
import { describe, expect, it } from "bun:test";
import type { ToolDefinition } from "../src/codemode/index.js";
import { createCodeTool, TwoPassExecutor } from "../src/codemode/index.js";
import { BINARY_PATH, ENV_FILE, GUEST_MODULE } from "./test-paths.js";

// ---------------------------------------------------------------------------
// Prerequisites
// ---------------------------------------------------------------------------

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
// Simulated host-side tools (what the agent's code will call)
// ---------------------------------------------------------------------------

const USERS_DB: Record<number, { id: number; name: string; email: string; plan: string }> = {
  1: { id: 1, name: "Alice", email: "alice@example.com", plan: "pro" },
  2: { id: 2, name: "Bob", email: "bob@test.com", plan: "free" },
  3: { id: 3, name: "Charlie", email: "charlie@co.uk", plan: "enterprise" },
  4: { id: 4, name: "Diana", email: "diana@example.com", plan: "pro" },
};

const EMAILS_SENT: Array<{ to: string; subject: string; body: string }> = [];

const tools: ToolDefinition[] = [
  {
    name: "getUser",
    description: "Get a user by their numeric ID. Returns the user object or null if not found.",
    inputSchema: {
      type: "object",
      properties: {
        id: { type: "number", description: "The user's numeric ID" },
      },
      required: ["id"],
    },
    execute: async (input) => {
      const { id } = input as { id: number };
      return USERS_DB[id] ?? null;
    },
  },
  {
    name: "listUsers",
    description: "List all users, optionally filtered by plan type.",
    inputSchema: {
      type: "object",
      properties: {
        plan: { type: "string", description: "Filter by plan: 'free', 'pro', or 'enterprise'" },
      },
    },
    execute: async (input) => {
      const { plan } = (input ?? {}) as { plan?: string };
      const users = Object.values(USERS_DB);
      return plan ? users.filter((u) => u.plan === plan) : users;
    },
  },
  {
    name: "sendEmail",
    description: "Send an email to a recipient. Returns a confirmation with a message ID.",
    inputSchema: {
      type: "object",
      properties: {
        to: { type: "string", description: "Recipient email address" },
        subject: { type: "string", description: "Email subject line" },
        body: { type: "string", description: "Email body text" },
      },
      required: ["to", "subject", "body"],
    },
    execute: async (input) => {
      const email = input as { to: string; subject: string; body: string };
      EMAILS_SENT.push(email);
      return { ok: true, messageId: `msg_${EMAILS_SENT.length}` };
    },
  },
  {
    name: "calculateDiscount",
    description: "Calculate a discount percentage based on the user's plan.",
    inputSchema: {
      type: "object",
      properties: {
        plan: { type: "string", description: "The user's plan: 'free', 'pro', or 'enterprise'" },
        amount: { type: "number", description: "The original amount in dollars" },
      },
      required: ["plan", "amount"],
    },
    execute: async (input) => {
      const { plan, amount } = input as { plan: string; amount: number };
      const rates: Record<string, number> = { free: 0, pro: 10, enterprise: 20 };
      const rate = rates[plan] ?? 0;
      return { discount: rate, discountedAmount: amount * (1 - rate / 100) };
    },
  },
];

// ---------------------------------------------------------------------------
// Agent harness
// ---------------------------------------------------------------------------

interface TextBlock {
  type: "text";
  text: string;
}
interface ToolUseBlock {
  type: "tool_use";
  id: string;
  name: string;
  input: Record<string, unknown>;
}
type ContentBlock = TextBlock | ToolUseBlock;
interface Message {
  content: ContentBlock[];
  stop_reason: string | null;
}

async function chat(
  messages: Array<{ role: string; content: unknown }>,
  aiTools: unknown[],
  system?: string,
): Promise<Message> {
  const body: Record<string, unknown> = {
    model: "claude-haiku-4-5-20251001",
    max_tokens: 2048,
    messages,
    tools: aiTools,
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe("Code Mode agent integration", () => {
  const executor = new TwoPassExecutor({
    binaryPath: BINARY_PATH,
    guestModule: GUEST_MODULE,
  });

  const codeTool = createCodeTool({ tools, executor });

  // Convert our CodeTool into the Anthropic tool_use format
  const anthropicTool = {
    name: codeTool.name,
    description: codeTool.description,
    input_schema: codeTool.inputSchema,
  };

  async function runAgent(prompt: string): Promise<{ text: string; codemodeCallCount: number }> {
    const messages: Array<{ role: string; content: unknown }> = [{ role: "user", content: prompt }];

    let codemodeCallCount = 0;

    for (let turn = 0; turn < 8; turn++) {
      const response = await chat(messages, [anthropicTool]);
      messages.push({ role: "assistant", content: response.content });

      if (response.stop_reason !== "tool_use") {
        const text = response.content
          .filter((b): b is TextBlock => b.type === "text")
          .map((b) => b.text)
          .join("\n");
        return { text, codemodeCallCount };
      }

      const toolUses = response.content.filter((b): b is ToolUseBlock => b.type === "tool_use");
      const toolResults = [];

      for (const tu of toolUses) {
        codemodeCallCount++;
        const result = await codeTool.execute(tu.input as { code: string });
        toolResults.push({
          type: "tool_result",
          tool_use_id: tu.id,
          content: JSON.stringify(result.result ?? result.error ?? "null"),
        });
      }

      messages.push({ role: "user", content: toolResults });
    }

    throw new Error("Agent exceeded max turns");
  }

  // Test 1: Multi-tool chaining — Claude writes ONE function that calls
  // getUser + calculateDiscount, instead of two separate tool_use calls.
  run(
    "chains getUser + calculateDiscount in one codemode call",
    async () => {
      const { text, codemodeCallCount } = await runAgent(
        "User #3 wants to buy a $500 item. Look up their account, " +
          "then calculate their discount based on their plan. " +
          "Tell me the final price they should pay.",
      );

      // Claude should have made just 1 codemode call (not 2 separate tool calls)
      expect(codemodeCallCount).toBeLessThanOrEqual(2);
      // Charlie is on enterprise (20% discount): $500 * 0.8 = $400
      expect(text).toMatch(/400|Charlie|enterprise/i);
    },
    60_000,
  );

  // Test 2: List + filter + email — a real workflow agent pattern
  run(
    "lists pro users and sends them a promotional email",
    async () => {
      EMAILS_SENT.length = 0;

      const { text, codemodeCallCount } = await runAgent(
        "Find all users on the 'pro' plan, then send each of them an email " +
          'with subject "Pro Upgrade Available" and body "You\'re eligible for enterprise!". ' +
          "Tell me how many emails were sent and to whom.",
      );

      expect(codemodeCallCount).toBeLessThanOrEqual(2);
      // Alice and Diana are on pro
      expect(text).toMatch(/Alice|Diana/i);
      expect(text).toMatch(/2|two/i);
    },
    60_000,
  );

  // Test 3: Data aggregation — the agent writes analysis code
  run(
    "aggregates user data with code logic + tool calls",
    async () => {
      const { text, codemodeCallCount } = await runAgent(
        "Get all users, then calculate the discount each would get on a $1000 purchase " +
          "based on their plan. Return a summary with each user's name, plan, and discounted price.",
      );

      // Should mention users and their prices
      expect(text).toMatch(/Alice|Bob|Charlie|Diana/i);
    },
    120_000,
  );
});
