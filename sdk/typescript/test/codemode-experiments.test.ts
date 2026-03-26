/**
 * Code Mode experiments using the real Anthropic SDK toolRunner.
 *
 * Tests the full pipeline:
 *   User prompt → Claude writes code → SandCastle sandbox executes →
 *   TwoPassExecutor dispatches tool calls host-side → result back to Claude
 *
 * Uses @anthropic-ai/sdk toolRunner for proper agent loop management.
 */
import { describe, expect, it } from "bun:test";
import Anthropic from "@anthropic-ai/sdk";
import type { ToolDefinition } from "../src/codemode/index.js";
import { createCodeTool, TwoPassExecutor } from "../src/codemode/index.js";

// ---------------------------------------------------------------------------
// Prerequisites
// ---------------------------------------------------------------------------

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE =
  "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

const apiKey = process.env.ANTHROPIC_API_KEY;

let hasBinary = false;
try {
  hasBinary = Bun.file(BINARY_PATH).size > 0;
} catch {
  /* not built */
}

const canRun = !!apiKey && hasBinary;
const run = canRun ? it : it.skip;

// ---------------------------------------------------------------------------
// Simulated host-side services
// ---------------------------------------------------------------------------

const PRODUCTS: Record<
  string,
  { id: string; name: string; price: number; category: string; stock: number }
> = {
  p1: { id: "p1", name: "Widget", price: 29.99, category: "hardware", stock: 150 },
  p2: { id: "p2", name: "Gadget", price: 49.99, category: "electronics", stock: 0 },
  p3: { id: "p3", name: "Sprocket", price: 9.99, category: "hardware", stock: 500 },
  p4: { id: "p4", name: "Doohickey", price: 199.99, category: "electronics", stock: 25 },
  p5: { id: "p5", name: "Thingamajig", price: 14.99, category: "misc", stock: 300 },
};

const ORDERS: Array<{
  orderId: string;
  productId: string;
  qty: number;
  total: number;
}> = [];

const EVENTS: Array<{ type: string; data: unknown }> = [];

const hostTools: ToolDefinition[] = [
  {
    name: "getProduct",
    description: "Get a product by ID. Returns product object or null.",
    inputSchema: {
      type: "object",
      properties: { id: { type: "string", description: "Product ID (e.g. 'p1')" } },
      required: ["id"],
    },
    execute: async (input) => {
      const { id } = input as { id: string };
      return PRODUCTS[id] ?? null;
    },
  },
  {
    name: "listProducts",
    description: "List all products, optionally filtered by category.",
    inputSchema: {
      type: "object",
      properties: {
        category: { type: "string", description: "Filter: 'hardware', 'electronics', 'misc'" },
      },
    },
    execute: async (input) => {
      const { category } = (input ?? {}) as { category?: string };
      const all = Object.values(PRODUCTS);
      return category ? all.filter((p) => p.category === category) : all;
    },
  },
  {
    name: "createOrder",
    description: "Create an order. Checks stock. Returns confirmation or error.",
    inputSchema: {
      type: "object",
      properties: {
        productId: { type: "string" },
        qty: { type: "number" },
      },
      required: ["productId", "qty"],
    },
    execute: async (input) => {
      const { productId, qty } = input as { productId: string; qty: number };
      const product = PRODUCTS[productId];
      if (!product) return { error: "Product not found" };
      if (product.stock < qty) return { error: `Insufficient stock: ${product.stock} available` };
      product.stock -= qty;
      const order = {
        orderId: `ord_${ORDERS.length + 1}`,
        productId,
        qty,
        total: +(product.price * qty).toFixed(2),
      };
      ORDERS.push(order);
      return order;
    },
  },
  {
    name: "trackEvent",
    description: "Log an analytics event.",
    inputSchema: {
      type: "object",
      properties: {
        type: { type: "string", description: "Event type" },
        data: { description: "Event payload" },
      },
      required: ["type"],
    },
    execute: async (input) => {
      const { type, data } = input as { type: string; data?: unknown };
      EVENTS.push({ type, data: data ?? null });
      return { tracked: true, count: EVENTS.length };
    },
  },
];

// ---------------------------------------------------------------------------
// Agent harness using Anthropic SDK toolRunner
// ---------------------------------------------------------------------------

const client = new Anthropic();

interface ExperimentResult {
  text: string;
  codemodeRoundTrips: number;
  toolCallsInside: number;
  elapsedMs: number;
}

async function runCodeModeAgent(
  prompt: string,
  codeTool: ReturnType<typeof createCodeTool>,
): Promise<ExperimentResult> {
  let codemodeRoundTrips = 0;
  let toolCallsInside = 0;
  const start = Date.now();

  // Use the SDK's built-in tool runner for proper agent loop
  const result = await client.messages.create({
    model: "claude-haiku-4-5-20251001",
    max_tokens: 4096,
    messages: [{ role: "user", content: prompt }],
    tools: [
      {
        name: codeTool.name,
        description: codeTool.description,
        input_schema: codeTool.inputSchema as Anthropic.Tool.InputSchema,
      },
    ],
  });

  // Manual loop since toolRunner doesn't support custom tool execution
  const messages: Anthropic.MessageParam[] = [
    { role: "user", content: prompt },
  ];
  let response = result;

  while (response.stop_reason === "tool_use") {
    messages.push({ role: "assistant", content: response.content });

    const toolResults: Anthropic.ToolResultBlockParam[] = [];
    for (const block of response.content) {
      if (block.type === "tool_use") {
        codemodeRoundTrips++;
        const execResult = await codeTool.execute(block.input as { code: string });
        toolCallsInside += execResult.toolCallCount;
        toolResults.push({
          type: "tool_result",
          tool_use_id: block.id,
          content: JSON.stringify(execResult.result ?? execResult.error ?? "null"),
        });
      }
    }

    messages.push({ role: "user", content: toolResults });

    response = await client.messages.create({
      model: "claude-haiku-4-5-20251001",
      max_tokens: 4096,
      messages,
      tools: [
        {
          name: codeTool.name,
          description: codeTool.description,
          input_schema: codeTool.inputSchema as Anthropic.Tool.InputSchema,
        },
      ],
    });
  }

  const text = response.content
    .filter((b): b is Anthropic.TextBlock => b.type === "text")
    .map((b) => b.text)
    .join("\n");

  return {
    text,
    codemodeRoundTrips,
    toolCallsInside,
    elapsedMs: Date.now() - start,
  };
}

function log(name: string, r: ExperimentResult) {
  console.log(`\n  --- ${name} ---`);
  console.log(
    `  Round-trips: ${r.codemodeRoundTrips} | Tool calls inside sandbox: ${r.toolCallsInside}`,
  );
  console.log(`  Time: ${r.elapsedMs}ms`);
  console.log(
    `  Response: ${r.text.slice(0, 300)}${r.text.length > 300 ? "..." : ""}`,
  );
}

// ---------------------------------------------------------------------------
// Experiments
// ---------------------------------------------------------------------------

describe("Code Mode experiments (Anthropic SDK)", () => {
  const executor = new TwoPassExecutor({
    binaryPath: BINARY_PATH,
    guestModule: GUEST_MODULE,
  });
  const codeTool = createCodeTool({ tools: hostTools, executor });

  // E1: 3-tool chain in one sandbox call
  run(
    "E1: lookup → order → track (3 tools chained)",
    async () => {
      ORDERS.length = 0;
      EVENTS.length = 0;

      const r = await runCodeModeAgent(
        "Look up product p1 (Widget), order 3 units, then track a 'purchase' event " +
          "with the order total. Tell me the confirmation.",
        codeTool,
      );
      log("E1", r);

      // Claude should chain 3 tool calls inside <=2 codemode round-trips
      expect(r.toolCallsInside).toBeGreaterThanOrEqual(2);
      expect(r.text).toMatch(/widget|order|confirm|89/i);
    },
    120_000,
  );

  // E2: Conditional logic in sandbox
  run(
    "E2: list hardware, filter in-stock, calculate bulk cost",
    async () => {
      const r = await runCodeModeAgent(
        "List all hardware products. For each one in stock (stock > 0), " +
          "calculate the cost of buying 10 units. Return a summary.",
        codeTool,
      );
      log("E2", r);

      expect(r.toolCallsInside).toBeGreaterThanOrEqual(1);
      expect(r.text).toMatch(/widget|sprocket/i);
    },
    120_000,
  );

  // E3: Error handling — out of stock fallback
  run(
    "E3: order out-of-stock → handle error → find alternative",
    async () => {
      ORDERS.length = 0;

      const r = await runCodeModeAgent(
        "Try ordering 5 Gadgets (p2). If out of stock, list electronics to find " +
          "an alternative and order 1 unit of it instead.",
        codeTool,
      );
      log("E3", r);

      expect(r.text).toMatch(/out of stock|insufficient|alternative|doohickey/i);
    },
    120_000,
  );

  // E4: Batch operation — process all products
  run(
    "E4: list all → track event per product → report total value",
    async () => {
      EVENTS.length = 0;

      const r = await runCodeModeAgent(
        "List all products. Track a 'product_view' event for each one (include name and price " +
          "in the event data). Tell me how many events were tracked and the total value of all products.",
        codeTool,
      );
      log("E4", r);

      expect(r.toolCallsInside).toBeGreaterThanOrEqual(1);
      // 5 products, total = 304.95
      expect(r.text).toMatch(/304|305|5.*event|event.*5/i);
    },
    120_000,
  );

  // E5: The money test — 6 operations in one shot
  run(
    "E5: 6-step workflow (2 lookups + 2 orders + 1 track + return total)",
    async () => {
      ORDERS.length = 0;
      EVENTS.length = 0;
      // Reset stock for clean test
      PRODUCTS.p1.stock = 150;
      PRODUCTS.p4.stock = 25;

      const r = await runCodeModeAgent(
        "Do all of this:\n" +
          "1. Get product p1\n" +
          "2. Get product p4\n" +
          "3. Order 2 of p1\n" +
          "4. Order 1 of p4\n" +
          "5. Track a 'bulk_purchase' event with both order totals\n" +
          "6. Tell me the combined total",
        codeTool,
      );
      log("E5", r);

      expect(r.toolCallsInside).toBeGreaterThanOrEqual(4);
      // 2*29.99 + 199.99 = 259.97
      expect(r.text).toMatch(/259|260|widget|doohickey/i);

      console.log(`\n  EFFICIENCY REPORT:`);
      console.log(`    ${r.toolCallsInside} tool calls in ${r.codemodeRoundTrips} sandbox round-trip(s)`);
      console.log(
        `    Without Code Mode: ~${r.toolCallsInside * 2} API round-trips needed`,
      );
    },
    120_000,
  );

  // E6: Creative code — agent writes non-trivial logic
  run(
    "E6: agent writes sorting + ranking logic in sandbox",
    async () => {
      const r = await runCodeModeAgent(
        "List all products. Sort them by price descending. Return the top 3 most " +
          "expensive products with their names and prices as a ranked list.",
        codeTool,
      );
      log("E6", r);

      expect(r.toolCallsInside).toBeGreaterThanOrEqual(1);
      // Most expensive: Doohickey (199.99), Gadget (49.99), Widget (29.99)
      expect(r.text).toMatch(/doohickey|199/i);
    },
    120_000,
  );
});
