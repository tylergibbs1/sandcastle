/**
 * Code Mode edge-case experiments — 25 tests pushing the boundaries.
 *
 * Tests adversarial prompts, complex data transforms, error cascades,
 * large payloads, recursive patterns, multi-step reasoning, and
 * scenarios where Code Mode might break.
 */
import { describe, expect, it } from "bun:test";
import Anthropic from "@anthropic-ai/sdk";
import type { ToolDefinition } from "../src/codemode/index.js";
import { createCodeTool, TwoPassExecutor } from "../src/codemode/index.js";

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE =
  "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";

const apiKey = process.env.ANTHROPIC_API_KEY;
let hasBinary = false;
try { hasBinary = Bun.file(BINARY_PATH).size > 0; } catch {}
const canRun = !!apiKey && hasBinary;
const run = canRun ? it : it.skip;

// ---------------------------------------------------------------------------
// Rich simulated backend
// ---------------------------------------------------------------------------

const DB = {
  users: [
    { id: 1, name: "Alice", email: "alice@co.com", role: "admin", active: true, credits: 500 },
    { id: 2, name: "Bob", email: "bob@co.com", role: "user", active: true, credits: 150 },
    { id: 3, name: "Charlie", email: "charlie@co.com", role: "user", active: false, credits: 0 },
    { id: 4, name: "Diana", email: "diana@co.com", role: "moderator", active: true, credits: 320 },
    { id: 5, name: "Eve", email: "eve@co.com", role: "user", active: true, credits: 75 },
    { id: 6, name: "Frank", email: "frank@co.com", role: "admin", active: true, credits: 1000 },
    { id: 7, name: "Grace", email: "grace@co.com", role: "user", active: true, credits: 200 },
    { id: 8, name: "Hank", email: "hank@co.com", role: "user", active: false, credits: 50 },
  ],
  products: [
    { sku: "A001", name: "Basic Plan", price: 9.99, category: "subscription", stock: 9999 },
    { sku: "A002", name: "Pro Plan", price: 29.99, category: "subscription", stock: 9999 },
    { sku: "A003", name: "Enterprise Plan", price: 99.99, category: "subscription", stock: 9999 },
    { sku: "B001", name: "Widget", price: 4.99, category: "physical", stock: 100 },
    { sku: "B002", name: "Gadget", price: 12.50, category: "physical", stock: 0 },
    { sku: "B003", name: "Sprocket", price: 2.99, category: "physical", stock: 500 },
    { sku: "C001", name: "API Credits 1K", price: 10.00, category: "credits", stock: 9999 },
    { sku: "C002", name: "API Credits 10K", price: 80.00, category: "credits", stock: 9999 },
  ],
  orders: [] as Array<{ id: string; userId: number; sku: string; qty: number; total: number; status: string }>,
  notifications: [] as Array<{ userId: number; channel: string; message: string }>,
  auditLog: [] as Array<{ action: string; actor: string; details: unknown; ts: number }>,
};

let orderSeq = 0;

const tools: ToolDefinition[] = [
  {
    name: "getUser",
    description: "Get user by ID",
    inputSchema: { type: "object", properties: { id: { type: "number" } }, required: ["id"] },
    execute: async (input) => {
      const { id } = input as { id: number };
      return DB.users.find(u => u.id === id) ?? { error: "User not found" };
    },
  },
  {
    name: "listUsers",
    description: "List users. Optional filters: role, active",
    inputSchema: {
      type: "object",
      properties: {
        role: { type: "string", description: "admin, user, moderator" },
        active: { type: "boolean" },
      },
    },
    execute: async (input) => {
      let users = [...DB.users];
      const { role, active } = (input ?? {}) as { role?: string; active?: boolean };
      if (role) users = users.filter(u => u.role === role);
      if (active !== undefined) users = users.filter(u => u.active === active);
      return users;
    },
  },
  {
    name: "updateUser",
    description: "Update user fields. Returns updated user.",
    inputSchema: {
      type: "object",
      properties: {
        id: { type: "number" },
        credits: { type: "number" },
        active: { type: "boolean" },
        role: { type: "string" },
      },
      required: ["id"],
    },
    execute: async (input) => {
      const { id, ...updates } = input as { id: number; credits?: number; active?: boolean; role?: string };
      const user = DB.users.find(u => u.id === id);
      if (!user) return { error: "User not found" };
      Object.assign(user, updates);
      return user;
    },
  },
  {
    name: "getProduct",
    description: "Get product by SKU",
    inputSchema: { type: "object", properties: { sku: { type: "string" } }, required: ["sku"] },
    execute: async (input) => {
      const { sku } = input as { sku: string };
      return DB.products.find(p => p.sku === sku) ?? { error: "Product not found" };
    },
  },
  {
    name: "listProducts",
    description: "List products. Optional filter by category.",
    inputSchema: {
      type: "object",
      properties: { category: { type: "string" } },
    },
    execute: async (input) => {
      const { category } = (input ?? {}) as { category?: string };
      let products = [...DB.products];
      if (category) products = products.filter(p => p.category === category);
      return products;
    },
  },
  {
    name: "createOrder",
    description: "Place an order. Checks stock and user credits.",
    inputSchema: {
      type: "object",
      properties: {
        userId: { type: "number" },
        sku: { type: "string" },
        qty: { type: "number" },
      },
      required: ["userId", "sku", "qty"],
    },
    execute: async (input) => {
      const { userId, sku, qty } = input as { userId: number; sku: string; qty: number };
      const user = DB.users.find(u => u.id === userId);
      if (!user) return { error: "User not found" };
      if (!user.active) return { error: "User account is inactive" };
      const product = DB.products.find(p => p.sku === sku);
      if (!product) return { error: "Product not found" };
      if (product.stock < qty) return { error: `Insufficient stock: ${product.stock} available` };
      const total = +(product.price * qty).toFixed(2);
      if (user.credits < total) return { error: `Insufficient credits: ${user.credits} available, need ${total}` };
      product.stock -= qty;
      user.credits -= total;
      const order = { id: `ORD-${++orderSeq}`, userId, sku, qty, total, status: "confirmed" };
      DB.orders.push(order);
      return order;
    },
  },
  {
    name: "sendNotification",
    description: "Send notification to user via channel (email, sms, push)",
    inputSchema: {
      type: "object",
      properties: {
        userId: { type: "number" },
        channel: { type: "string", description: "email, sms, or push" },
        message: { type: "string" },
      },
      required: ["userId", "channel", "message"],
    },
    execute: async (input) => {
      const n = input as { userId: number; channel: string; message: string };
      DB.notifications.push(n);
      return { sent: true, id: `notif_${DB.notifications.length}` };
    },
  },
  {
    name: "logAudit",
    description: "Write to audit log",
    inputSchema: {
      type: "object",
      properties: {
        action: { type: "string" },
        actor: { type: "string" },
        details: {},
      },
      required: ["action", "actor"],
    },
    execute: async (input) => {
      const { action, actor, details } = input as { action: string; actor: string; details?: unknown };
      DB.auditLog.push({ action, actor, details: details ?? null, ts: Date.now() });
      return { logged: true };
    },
  },
  {
    name: "mathCompute",
    description: "Perform a math operation: sum, average, median, percentile on an array of numbers",
    inputSchema: {
      type: "object",
      properties: {
        operation: { type: "string", description: "sum, average, median, min, max, percentile" },
        values: { type: "array", items: { type: "number" } },
        p: { type: "number", description: "percentile value (0-100), only for percentile op" },
      },
      required: ["operation", "values"],
    },
    execute: async (input) => {
      const { operation, values, p } = input as { operation: string; values: number[]; p?: number };
      const sorted = [...values].sort((a, b) => a - b);
      switch (operation) {
        case "sum": return { result: values.reduce((a, b) => a + b, 0) };
        case "average": return { result: values.reduce((a, b) => a + b, 0) / values.length };
        case "median": {
          const mid = Math.floor(sorted.length / 2);
          return { result: sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2 };
        }
        case "min": return { result: Math.min(...values) };
        case "max": return { result: Math.max(...values) };
        case "percentile": {
          const idx = Math.ceil((p ?? 50) / 100 * sorted.length) - 1;
          return { result: sorted[Math.max(0, idx)] };
        }
        default: return { error: `Unknown operation: ${operation}` };
      }
    },
  },
];

// ---------------------------------------------------------------------------
// Agent harness
// ---------------------------------------------------------------------------

const client = new Anthropic();

interface Result {
  text: string;
  roundTrips: number;
  toolCalls: number;
  ms: number;
}

async function agent(prompt: string, ct: ReturnType<typeof createCodeTool>, system?: string): Promise<Result> {
  const tool: Anthropic.Tool = {
    name: ct.name,
    description: ct.description,
    input_schema: ct.inputSchema as Anthropic.Tool.InputSchema,
  };

  const msgs: Anthropic.MessageParam[] = [{ role: "user", content: prompt }];
  let roundTrips = 0;
  let toolCalls = 0;
  const t0 = Date.now();

  for (let i = 0; i < 20; i++) {
    const r = await client.messages.create({
      model: "claude-opus-4-6",
      max_tokens: 4096,
      messages: msgs,
      tools: [tool],
      ...(system ? { system } : {}),
    });
    msgs.push({ role: "assistant", content: r.content });

    if (r.stop_reason !== "tool_use") {
      const text = r.content.filter((b): b is Anthropic.TextBlock => b.type === "text").map(b => b.text).join("\n");
      return { text, roundTrips, toolCalls, ms: Date.now() - t0 };
    }

    const results: Anthropic.ToolResultBlockParam[] = [];
    for (const b of r.content) {
      if (b.type === "tool_use") {
        roundTrips++;
        const res = await ct.execute(b.input as { code: string });
        toolCalls += res.toolCallCount;
        results.push({
          type: "tool_result",
          tool_use_id: b.id,
          content: JSON.stringify(res.result ?? res.error ?? "null"),
        });
      }
    }
    msgs.push({ role: "user", content: results });
  }
  throw new Error("Max turns");
}

function reset() {
  DB.orders.length = 0;
  DB.notifications.length = 0;
  DB.auditLog.length = 0;
  orderSeq = 0;
  // Reset user credits
  DB.users[0].credits = 500; DB.users[1].credits = 150; DB.users[2].credits = 0;
  DB.users[3].credits = 320; DB.users[4].credits = 75; DB.users[5].credits = 1000;
  DB.users[6].credits = 200; DB.users[7].credits = 50;
  // Reset stock
  DB.products[3].stock = 100; DB.products[4].stock = 0; DB.products[5].stock = 500;
  // Reset active
  DB.users[2].active = false; DB.users[7].active = false;
}

function log(name: string, r: Result) {
  console.log(`  [${r.roundTrips}rt/${r.toolCalls}tc/${r.ms}ms] ${name}`);
  console.log(`    ${r.text.slice(0, 200).replace(/\n/g, " ")}${r.text.length > 200 ? "..." : ""}`);
}

// ---------------------------------------------------------------------------
// 25 Experiments
// ---------------------------------------------------------------------------

describe("Code Mode edge experiments (25)", () => {
  const executor = new TwoPassExecutor({ binaryPath: BINARY_PATH, guestModule: GUEST_MODULE });
  const ct = createCodeTool({ tools, executor });

  // --- DATA AGGREGATION ---

  run("E01: aggregate credits by role", async () => {
    reset();
    const r = await agent("Get all active users and compute the total credits per role. Return a breakdown.", ct);
    log("E01", r); expect(r.text).toMatch(/admin|user|moderator/i);
  }, 60_000);

  run("E02: find users with credits below threshold", async () => {
    reset();
    const r = await agent("Find all active users with fewer than 100 credits. List their names and credit balances.", ct);
    log("E02", r); expect(r.text).toMatch(/eve|75/i);
  }, 60_000);

  run("E03: compute median and percentile of credits", async () => {
    reset();
    const r = await agent(
      "Get all active users, extract their credit values, then use mathCompute to find the median and the 90th percentile. Report both numbers.",
      ct
    );
    log("E03", r); expect(r.toolCalls).toBeGreaterThanOrEqual(2);
  }, 60_000);

  run("E04: cross-reference users and products", async () => {
    reset();
    const r = await agent(
      "List all subscription products and all admin users. For each admin, determine which is the most expensive subscription they can afford with their credits. Return a table.",
      ct
    );
    log("E04", r); expect(r.text).toMatch(/alice|frank|plan/i);
  }, 60_000);

  // --- ERROR HANDLING ---

  run("E05: order for inactive user — handle gracefully", async () => {
    reset();
    const r = await agent("Try to create an order for user 3 (Charlie) for product A001 qty 1. He's inactive. Report what happened.", ct);
    log("E05", r); expect(r.text).toMatch(/inactive|cannot|error/i);
  }, 60_000);

  run("E06: order exceeding credits — handle and suggest alternative", async () => {
    reset();
    const r = await agent(
      "User 5 (Eve, 75 credits) wants to buy a Pro Plan (A002, $29.99). If she can't afford it, find the most expensive subscription she CAN afford and order that instead.",
      ct
    );
    log("E06", r); expect(r.text).toMatch(/basic|9\.99|eve/i);
  }, 60_000);

  run("E07: out of stock + fallback + audit", async () => {
    reset();
    const r = await agent(
      "User 1 wants 5 Gadgets (B002). If out of stock, order 5 Sprockets (B003) instead. Log an audit entry for whichever action was taken.",
      ct
    );
    log("E07", r); expect(r.text).toMatch(/sprocket|audit|order|fallback/i);
  }, 60_000);

  run("E08: cascade of errors — all options fail", async () => {
    reset();
    const r = await agent(
      "User 3 (inactive) wants to order B002 (out of stock). Try the order, handle both errors, and report a summary of all issues encountered.",
      ct
    );
    log("E08", r); expect(r.text).toMatch(/inactive|stock|error|issue/i);
  }, 60_000);

  // --- MULTI-STEP WORKFLOWS ---

  run("E09: sign up flow — order + notify + audit", async () => {
    reset();
    const r = await agent(
      "User 2 (Bob) is upgrading to Pro Plan (A002). Create the order, send him an email notification saying 'Welcome to Pro!', and log an audit entry. Report everything.",
      ct
    );
    log("E09", r);
    expect(DB.orders.length).toBeGreaterThanOrEqual(1);
    expect(DB.notifications.length).toBeGreaterThanOrEqual(1);
    expect(DB.auditLog.length).toBeGreaterThanOrEqual(1);
  }, 60_000);

  run("E10: bulk orders for a team", async () => {
    reset();
    const r = await agent(
      "Order 1 unit of 'API Credits 1K' (C001) for each active user. Skip any user who can't afford it. Report successes and failures.",
      ct
    );
    log("E10", r); expect(r.text).toMatch(/order|success|fail|credit/i);
  }, 60_000);

  run("E11: notify all admins via multiple channels", async () => {
    reset();
    const r = await agent(
      "Find all admin users. Send each one a push notification AND an email saying 'System maintenance tonight at 11 PM'. Report how many notifications were sent.",
      ct
    );
    log("E11", r); expect(r.text).toMatch(/admin|notification|sent|4|maintenance/i);
  }, 60_000);

  run("E12: deactivate users with zero credits and notify them", async () => {
    reset();
    const r = await agent(
      "Find all users with 0 credits. Deactivate them (set active=false). Send each an email saying 'Your account has been suspended due to zero balance.' Log an audit entry for each.",
      ct
    );
    log("E12", r);
    const charlie = DB.users.find(u => u.id === 3);
    expect(charlie?.active).toBe(false);
  }, 60_000);

  // --- COMPLEX LOGIC ---

  run("E13: tiered pricing calculation", async () => {
    reset();
    const r = await agent(
      "Calculate tiered pricing for user 6 (Frank, 1000 credits): " +
      "first 100 credits at $0.10/credit, next 400 at $0.08/credit, remaining at $0.05/credit. " +
      "Use mathCompute with sum to verify. What's the total value of Frank's credits?",
      ct
    );
    log("E13", r); expect(r.text).toMatch(/\d+/); // should have a number
  }, 60_000);

  run("E14: leaderboard — rank users by credits", async () => {
    reset();
    const r = await agent(
      "Get all active users, sort them by credits descending, and return a numbered leaderboard with name and credits. Who's #1?",
      ct
    );
    log("E14", r); expect(r.text).toMatch(/frank|1000/i);
  }, 60_000);

  run("E15: conditional branching tree", async () => {
    reset();
    const r = await agent(
      "For user 4 (Diana, moderator, 320 credits): " +
      "If she's a moderator, check if she can afford Enterprise Plan (A003, $99.99). " +
      "If yes, order it and promote her to admin. " +
      "If no, order Pro Plan (A002) instead. " +
      "Either way, send her a push notification with the result.",
      ct
    );
    log("E15", r);
    expect(DB.orders.length).toBeGreaterThanOrEqual(1);
    expect(DB.notifications.length).toBeGreaterThanOrEqual(1);
  }, 60_000);

  // --- DATA TRANSFORMATION ---

  run("E16: generate report JSON from multiple sources", async () => {
    reset();
    const r = await agent(
      "Generate a JSON report with: total active users, total inactive users, " +
      "average credits of active users (use mathCompute), and a list of all subscription products with prices. " +
      "Return the complete report object.",
      ct
    );
    log("E16", r); expect(r.text).toMatch(/active|average|subscription/i);
  }, 60_000);

  run("E17: pivot table — products by category with stats", async () => {
    reset();
    const r = await agent(
      "List all products grouped by category. For each category, compute the average price using mathCompute. " +
      "Return a summary like: category -> count, avgPrice.",
      ct
    );
    log("E17", r); expect(r.text).toMatch(/subscription|physical|credits/i);
  }, 60_000);

  run("E18: user enrichment — join user + order data", async () => {
    reset();
    // Pre-create some orders
    DB.orders.push({ id: "ORD-pre1", userId: 1, sku: "A002", qty: 1, total: 29.99, status: "confirmed" });
    DB.orders.push({ id: "ORD-pre2", userId: 1, sku: "B001", qty: 5, total: 24.95, status: "confirmed" });
    DB.orders.push({ id: "ORD-pre3", userId: 2, sku: "A001", qty: 1, total: 9.99, status: "confirmed" });

    const r = await agent(
      "Get user 1 (Alice). She has some existing orders. List her details and compute her total spend across all orders. Use mathCompute for the sum.",
      ct
    );
    log("E18", r); expect(r.text).toMatch(/alice|54\.94|spend/i);
  }, 60_000);

  // --- ADVERSARIAL / TRICKY ---

  run("E19: empty result handling", async () => {
    reset();
    const r = await agent(
      "List all users with role 'superadmin'. There are none. Handle this gracefully and report that no users were found.",
      ct
    );
    log("E19", r); expect(r.text).toMatch(/no|none|found|empty|0/i);
  }, 60_000);

  run("E20: ambiguous request — agent must reason", async () => {
    reset();
    const r = await agent(
      "Which user can buy the most products? Consider their credits and product prices. Show your reasoning.",
      ct
    );
    log("E20", r); expect(r.text).toMatch(/frank|1000/i);
  }, 60_000);

  run("E21: idempotency — run same operation twice", async () => {
    reset();
    const r = await agent(
      "Order 1 Basic Plan (A001) for user 2 (Bob). Then try ordering the same thing again. " +
      "Did the second order succeed or fail? Report both results.",
      ct
    );
    log("E21", r); expect(r.text).toMatch(/order|confirm|credit/i);
  }, 60_000);

  run("E22: many tool calls in single sandbox (stress)", async () => {
    reset();
    const r = await agent(
      "For EVERY user (all 8), get their details AND send them a push notification saying 'Monthly report ready'. " +
      "That's 16 tool calls total. Do it all in one go and tell me how many notifications were sent.",
      ct
    );
    log("E22", r); expect(r.text).toMatch(/8|notification|sent|report/i);
  }, 120_000);

  // --- COMPLEX MULTI-STEP ---

  run("E23: full order pipeline with validation", async () => {
    reset();
    const r = await agent(
      "Process this shopping cart for user 1 (Alice, 500 credits):\n" +
      "- 2x Widget (B001) @ $4.99\n" +
      "- 1x API Credits 1K (C001) @ $10.00\n" +
      "Validate each item is in stock, check total doesn't exceed credits, " +
      "place each order, send a confirmation email, and log it all to audit. " +
      "Return the total spent and remaining credits.",
      ct
    );
    log("E23", r);
    expect(DB.orders.length).toBeGreaterThanOrEqual(2);
    expect(DB.notifications.length).toBeGreaterThanOrEqual(1);
  }, 120_000);

  run("E24: analytics pipeline — compute then act", async () => {
    reset();
    const r = await agent(
      "Compute the average credits across all active users (use mathCompute). " +
      "Find all active users below the average. " +
      "Send each below-average user an SMS saying 'Top up your credits for a bonus!' " +
      "Report: the average, who's below it, and how many notifications sent.",
      ct
    );
    log("E24", r); expect(r.text).toMatch(/average|below|notification|credits/i);
  }, 300_000);

  run("E25: full business workflow — onboard new customer", async () => {
    reset();
    const r = await agent(
      "Onboard user 7 (Grace, 200 credits) as a new Pro customer:\n" +
      "1. Verify she's active\n" +
      "2. Check she can afford Pro Plan (A002, $29.99)\n" +
      "3. Place the order\n" +
      "4. Update her role to 'moderator' as a Pro perk\n" +
      "5. Send her an email: 'Welcome to Pro! You now have moderator access.'\n" +
      "6. Send her a push notification: 'Pro Plan activated'\n" +
      "7. Log audit: 'pro_upgrade' by 'system'\n" +
      "Report every step's result.",
      ct
    );
    log("E25", r);
    const grace = DB.users.find(u => u.id === 7);
    expect(grace?.role).toBe("moderator");
    expect(DB.orders.length).toBeGreaterThanOrEqual(1);
    expect(DB.notifications.length).toBeGreaterThanOrEqual(2);
    expect(DB.auditLog.length).toBeGreaterThanOrEqual(1);
  }, 120_000);
});
