/**
 * 50 more Code Mode experiments — deep stress test with claude-opus-4-6.
 *
 * Categories:
 *   E26-E35: Advanced data transforms & analytics
 *   E36-E45: Adversarial, ambiguous, & tricky prompts
 *   E46-E55: Complex multi-entity workflows
 *   E56-E65: Edge cases in tool interaction patterns
 *   E66-E75: Real-world agent scenarios
 */
import { describe, expect, it } from "bun:test";
import Anthropic from "@anthropic-ai/sdk";
import type { ToolDefinition } from "../src/codemode/index.js";
import { createCodeTool, TwoPassExecutor } from "../src/codemode/index.js";

const BINARY_PATH = "../../target/release/sandcastle";
const GUEST_MODULE = "../../guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm";
const apiKey = process.env.ANTHROPIC_API_KEY;
let hasBinary = false;
try { hasBinary = Bun.file(BINARY_PATH).size > 0; } catch {}
const canRun = !!apiKey && hasBinary;
const run = canRun ? it : it.skip;

// ---------------------------------------------------------------------------
// Rich simulated backend
// ---------------------------------------------------------------------------

interface User { id: number; name: string; email: string; role: string; active: boolean; credits: number; tags: string[]; joinedAt: string }
interface Product { sku: string; name: string; price: number; category: string; stock: number; rating: number }
interface Order { id: string; userId: number; sku: string; qty: number; total: number; status: string; createdAt: string }
interface Ticket { id: string; userId: number; subject: string; priority: string; status: string; assignedTo: number | null }

const DB = {
  users: [
    { id: 1, name: "Alice", email: "alice@co.com", role: "admin", active: true, credits: 500, tags: ["vip", "early-adopter"], joinedAt: "2023-01-15" },
    { id: 2, name: "Bob", email: "bob@co.com", role: "user", active: true, credits: 150, tags: ["trial"], joinedAt: "2024-06-01" },
    { id: 3, name: "Charlie", email: "charlie@co.com", role: "user", active: false, credits: 0, tags: ["churned"], joinedAt: "2023-03-20" },
    { id: 4, name: "Diana", email: "diana@co.com", role: "moderator", active: true, credits: 320, tags: ["vip"], joinedAt: "2023-07-10" },
    { id: 5, name: "Eve", email: "eve@co.com", role: "user", active: true, credits: 75, tags: ["trial"], joinedAt: "2024-11-01" },
    { id: 6, name: "Frank", email: "frank@co.com", role: "admin", active: true, credits: 1000, tags: ["vip", "early-adopter", "beta"], joinedAt: "2022-12-01" },
    { id: 7, name: "Grace", email: "grace@co.com", role: "user", active: true, credits: 200, tags: [], joinedAt: "2024-01-15" },
    { id: 8, name: "Hank", email: "hank@co.com", role: "user", active: false, credits: 50, tags: ["churned"], joinedAt: "2023-09-01" },
    { id: 9, name: "Iris", email: "iris@co.com", role: "user", active: true, credits: 425, tags: ["vip"], joinedAt: "2023-05-20" },
    { id: 10, name: "Jack", email: "jack@co.com", role: "moderator", active: true, credits: 280, tags: ["beta"], joinedAt: "2024-02-14" },
  ] as User[],
  products: [
    { sku: "A001", name: "Basic Plan", price: 9.99, category: "subscription", stock: 9999, rating: 4.2 },
    { sku: "A002", name: "Pro Plan", price: 29.99, category: "subscription", stock: 9999, rating: 4.7 },
    { sku: "A003", name: "Enterprise Plan", price: 99.99, category: "subscription", stock: 9999, rating: 4.9 },
    { sku: "B001", name: "Widget", price: 4.99, category: "physical", stock: 100, rating: 3.8 },
    { sku: "B002", name: "Gadget", price: 12.50, category: "physical", stock: 0, rating: 4.1 },
    { sku: "B003", name: "Sprocket", price: 2.99, category: "physical", stock: 500, rating: 3.5 },
    { sku: "C001", name: "API Credits 1K", price: 10.00, category: "credits", stock: 9999, rating: 4.5 },
    { sku: "C002", name: "API Credits 10K", price: 80.00, category: "credits", stock: 9999, rating: 4.6 },
    { sku: "D001", name: "Support Addon", price: 15.00, category: "addon", stock: 9999, rating: 4.0 },
    { sku: "D002", name: "Analytics Addon", price: 25.00, category: "addon", stock: 9999, rating: 4.3 },
  ] as Product[],
  orders: [] as Order[],
  tickets: [] as Ticket[],
  notifications: [] as Array<{ userId: number; channel: string; message: string }>,
  auditLog: [] as Array<{ action: string; actor: string; details: unknown }>,
};

let orderSeq = 0;
let ticketSeq = 0;

const tools: ToolDefinition[] = [
  {
    name: "getUser", description: "Get user by ID",
    inputSchema: { type: "object", properties: { id: { type: "number" } }, required: ["id"] },
    execute: async (i) => { const { id } = i as any; return DB.users.find(u => u.id === id) ?? { error: "Not found" }; },
  },
  {
    name: "listUsers", description: "List users. Filters: role, active, tag",
    inputSchema: { type: "object", properties: { role: { type: "string" }, active: { type: "boolean" }, tag: { type: "string" } } },
    execute: async (i) => {
      let u = [...DB.users]; const f = (i ?? {}) as any;
      if (f.role) u = u.filter(x => x.role === f.role);
      if (f.active !== undefined) u = u.filter(x => x.active === f.active);
      if (f.tag) u = u.filter(x => x.tags.includes(f.tag));
      return u;
    },
  },
  {
    name: "updateUser", description: "Update user fields (credits, active, role, tags)",
    inputSchema: { type: "object", properties: { id: { type: "number" }, credits: { type: "number" }, active: { type: "boolean" }, role: { type: "string" }, tags: { type: "array", items: { type: "string" } } }, required: ["id"] },
    execute: async (i) => { const { id, ...u } = i as any; const usr = DB.users.find(x => x.id === id); if (!usr) return { error: "Not found" }; Object.assign(usr, u); return usr; },
  },
  {
    name: "getProduct", description: "Get product by SKU",
    inputSchema: { type: "object", properties: { sku: { type: "string" } }, required: ["sku"] },
    execute: async (i) => { const { sku } = i as any; return DB.products.find(p => p.sku === sku) ?? { error: "Not found" }; },
  },
  {
    name: "listProducts", description: "List products. Filter by category, minRating, inStock (boolean)",
    inputSchema: { type: "object", properties: { category: { type: "string" }, minRating: { type: "number" }, inStock: { type: "boolean" } } },
    execute: async (i) => {
      let p = [...DB.products]; const f = (i ?? {}) as any;
      if (f.category) p = p.filter(x => x.category === f.category);
      if (f.minRating) p = p.filter(x => x.rating >= f.minRating);
      if (f.inStock) p = p.filter(x => x.stock > 0);
      return p;
    },
  },
  {
    name: "createOrder", description: "Place order. Validates stock & credits.",
    inputSchema: { type: "object", properties: { userId: { type: "number" }, sku: { type: "string" }, qty: { type: "number" } }, required: ["userId", "sku", "qty"] },
    execute: async (i) => {
      const { userId, sku, qty } = i as any;
      const user = DB.users.find(u => u.id === userId); if (!user) return { error: "User not found" };
      if (!user.active) return { error: "Inactive user" };
      const prod = DB.products.find(p => p.sku === sku); if (!prod) return { error: "Product not found" };
      if (prod.stock < qty) return { error: `Out of stock (${prod.stock} left)` };
      const total = +(prod.price * qty).toFixed(2);
      if (user.credits < total) return { error: `Insufficient credits (${user.credits} < ${total})` };
      prod.stock -= qty; user.credits = +(user.credits - total).toFixed(2);
      const o: Order = { id: `ORD-${++orderSeq}`, userId, sku, qty, total, status: "confirmed", createdAt: new Date().toISOString() };
      DB.orders.push(o); return o;
    },
  },
  {
    name: "listOrders", description: "List orders. Filter by userId, status",
    inputSchema: { type: "object", properties: { userId: { type: "number" }, status: { type: "string" } } },
    execute: async (i) => { let o = [...DB.orders]; const f = (i ?? {}) as any; if (f.userId) o = o.filter(x => x.userId === f.userId); if (f.status) o = o.filter(x => x.status === f.status); return o; },
  },
  {
    name: "createTicket", description: "Create support ticket",
    inputSchema: { type: "object", properties: { userId: { type: "number" }, subject: { type: "string" }, priority: { type: "string", description: "low, medium, high, critical" } }, required: ["userId", "subject", "priority"] },
    execute: async (i) => {
      const { userId, subject, priority } = i as any;
      const t: Ticket = { id: `TKT-${++ticketSeq}`, userId, subject, priority, status: "open", assignedTo: null };
      DB.tickets.push(t); return t;
    },
  },
  {
    name: "listTickets", description: "List tickets. Filter by userId, priority, status, assignedTo",
    inputSchema: { type: "object", properties: { userId: { type: "number" }, priority: { type: "string" }, status: { type: "string" }, assignedTo: { type: "number" } } },
    execute: async (i) => { let t = [...DB.tickets]; const f = (i ?? {}) as any; if (f.userId) t = t.filter(x => x.userId === f.userId); if (f.priority) t = t.filter(x => x.priority === f.priority); if (f.status) t = t.filter(x => x.status === f.status); if (f.assignedTo) t = t.filter(x => x.assignedTo === f.assignedTo); return t; },
  },
  {
    name: "updateTicket", description: "Update ticket fields (status, assignedTo, priority)",
    inputSchema: { type: "object", properties: { id: { type: "string" }, status: { type: "string" }, assignedTo: { type: "number" }, priority: { type: "string" } }, required: ["id"] },
    execute: async (i) => { const { id, ...u } = i as any; const t = DB.tickets.find(x => x.id === id); if (!t) return { error: "Not found" }; Object.assign(t, u); return t; },
  },
  {
    name: "sendNotification", description: "Send notification (email, sms, push)",
    inputSchema: { type: "object", properties: { userId: { type: "number" }, channel: { type: "string" }, message: { type: "string" } }, required: ["userId", "channel", "message"] },
    execute: async (i) => { const n = i as any; DB.notifications.push(n); return { sent: true }; },
  },
  {
    name: "logAudit", description: "Write audit log entry",
    inputSchema: { type: "object", properties: { action: { type: "string" }, actor: { type: "string" }, details: {} }, required: ["action", "actor"] },
    execute: async (i) => { const { action, actor, details } = i as any; DB.auditLog.push({ action, actor, details: details ?? null }); return { logged: true }; },
  },
  {
    name: "mathCompute", description: "Math: sum, average, median, min, max, percentile, stddev on number array",
    inputSchema: { type: "object", properties: { operation: { type: "string" }, values: { type: "array", items: { type: "number" } }, p: { type: "number" } }, required: ["operation", "values"] },
    execute: async (i) => {
      const { operation, values, p } = i as any; const s = [...values].sort((a: number, b: number) => a - b);
      const ops: Record<string, () => number> = {
        sum: () => values.reduce((a: number, b: number) => a + b, 0),
        average: () => values.reduce((a: number, b: number) => a + b, 0) / values.length,
        median: () => { const m = Math.floor(s.length / 2); return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2; },
        min: () => s[0], max: () => s[s.length - 1],
        percentile: () => s[Math.max(0, Math.ceil((p ?? 50) / 100 * s.length) - 1)],
        stddev: () => { const avg = values.reduce((a: number, b: number) => a + b, 0) / values.length; return Math.sqrt(values.reduce((s: number, v: number) => s + (v - avg) ** 2, 0) / values.length); },
      };
      return ops[operation] ? { result: +ops[operation]().toFixed(4) } : { error: "Unknown op" };
    },
  },
];

// ---------------------------------------------------------------------------
// Agent harness
// ---------------------------------------------------------------------------

const client = new Anthropic();

interface R { text: string; rt: number; tc: number; ms: number }

async function agent(prompt: string, ct: ReturnType<typeof createCodeTool>): Promise<R> {
  const tool: Anthropic.Tool = { name: ct.name, description: ct.description, input_schema: ct.inputSchema as Anthropic.Tool.InputSchema };
  const msgs: Anthropic.MessageParam[] = [{ role: "user", content: prompt }];
  let rt = 0, tc = 0; const t0 = Date.now();
  for (let i = 0; i < 20; i++) {
    const r = await client.messages.create({ model: "claude-opus-4-6", max_tokens: 4096, messages: msgs, tools: [tool] });
    msgs.push({ role: "assistant", content: r.content });
    if (r.stop_reason !== "tool_use") {
      return { text: r.content.filter((b): b is Anthropic.TextBlock => b.type === "text").map(b => b.text).join("\n"), rt, tc, ms: Date.now() - t0 };
    }
    const res: Anthropic.ToolResultBlockParam[] = [];
    for (const b of r.content) {
      if (b.type === "tool_use") { rt++; const x = await ct.execute(b.input as { code: string }); tc += x.toolCallCount; res.push({ type: "tool_result", tool_use_id: b.id, content: JSON.stringify(x.result ?? x.error ?? "null") }); }
    }
    msgs.push({ role: "user", content: res });
  }
  throw new Error("Max turns");
}

function reset() {
  DB.orders.length = 0; DB.tickets.length = 0; DB.notifications.length = 0; DB.auditLog.length = 0;
  orderSeq = 0; ticketSeq = 0;
  DB.users.forEach(u => { u.active = ![3, 8].includes(u.id); });
  DB.users[0].credits = 500; DB.users[1].credits = 150; DB.users[2].credits = 0; DB.users[3].credits = 320;
  DB.users[4].credits = 75; DB.users[5].credits = 1000; DB.users[6].credits = 200; DB.users[7].credits = 50;
  DB.users[8].credits = 425; DB.users[9].credits = 280;
  DB.products[3].stock = 100; DB.products[4].stock = 0; DB.products[5].stock = 500;
}

function log(n: string, r: R) {
  console.log(`  [${r.rt}rt/${r.tc}tc/${r.ms}ms] ${n}: ${r.text.slice(0, 150).replace(/\n/g, " ")}${r.text.length > 150 ? "..." : ""}`);
}

// ---------------------------------------------------------------------------
// 50 Experiments
// ---------------------------------------------------------------------------

describe("Code Mode 50 more experiments", () => {
  const executor = new TwoPassExecutor({ binaryPath: BINARY_PATH, guestModule: GUEST_MODULE });
  const ct = createCodeTool({ tools, executor });

  // === DATA TRANSFORMS & ANALYTICS (E26-E35) ===

  run("E26: group users by join year", async () => {
    reset(); const r = await agent("Group all users by the year they joined. Return a count per year.", ct);
    log("E26", r); expect(r.text).toMatch(/2023|2024|2022/i);
  }, 120_000);

  run("E27: stddev of credits", async () => {
    reset(); const r = await agent("Get all active users, compute standard deviation of their credits using mathCompute.", ct);
    log("E27", r); expect(r.tc).toBeGreaterThanOrEqual(2);
  }, 120_000);

  run("E28: top N products by rating", async () => {
    reset(); const r = await agent("Find the top 3 products by rating that are in stock. Return name, rating, price.", ct);
    log("E28", r); expect(r.text).toMatch(/enterprise|pro|credit/i);
  }, 120_000);

  run("E29: revenue projection", async () => {
    reset(); const r = await agent("If every active user bought the cheapest subscription (Basic Plan), what would total revenue be? Calculate it.", ct);
    log("E29", r); expect(r.text).toMatch(/\d+/);
  }, 120_000);

  run("E30: cohort analysis", async () => {
    reset(); const r = await agent("Group active users into cohorts: joined before 2024 vs 2024+. Compare average credits between cohorts.", ct);
    log("E30", r); expect(r.text).toMatch(/cohort|average|before|after|2024/i);
  }, 120_000);

  run("E31: find VIP users below median credits", async () => {
    reset(); const r = await agent("Get all users tagged 'vip'. Compute their median credits. List any VIP below the median.", ct);
    log("E31", r); expect(r.text).toMatch(/vip|median/i);
  }, 120_000);

  run("E32: product affordability matrix", async () => {
    reset(); const r = await agent("For each active user, list which subscription products they can afford. Format as a matrix.", ct);
    log("E32", r); expect(r.text).toMatch(/alice|bob|basic|pro/i);
  }, 120_000);

  run("E33: weighted score ranking", async () => {
    reset(); const r = await agent("Rank all products by a weighted score: (rating * 20) + (stock > 0 ? 10 : 0) - (price / 10). Show top 5.", ct);
    log("E33", r); expect(r.text).toMatch(/\d+/);
  }, 120_000);

  run("E34: churn risk scoring", async () => {
    reset(); const r = await agent("Score each active user for churn risk: low credits (<100) = +3, no tags = +2, 'trial' tag = +1. List users sorted by risk score.", ct);
    log("E34", r); expect(r.text).toMatch(/eve|grace|risk/i);
  }, 120_000);

  run("E35: time-based analysis", async () => {
    reset(); const r = await agent("How many users joined each quarter? (Q1=Jan-Mar, Q2=Apr-Jun, etc). Show the breakdown.", ct);
    log("E35", r); expect(r.text).toMatch(/Q[1-4]|quarter/i);
  }, 120_000);

  // === ADVERSARIAL & TRICKY (E36-E45) ===

  run("E36: contradictory requirements", async () => {
    reset(); const r = await agent("Find users who are both active AND inactive. Explain what you find.", ct);
    log("E36", r); expect(r.text).toMatch(/no|none|impossible|cannot|0/i);
  }, 120_000);

  run("E37: nonexistent product", async () => {
    reset(); const r = await agent("Get product with SKU 'Z999'. Handle the error and suggest searching for similar products.", ct);
    log("E37", r); expect(r.text).toMatch(/not found|error|no product/i);
  }, 120_000);

  run("E38: zero quantity order", async () => {
    reset(); const r = await agent("Try ordering 0 units of Widget (B001) for user 1. What happens?", ct);
    log("E38", r); expect(r.tc).toBeGreaterThanOrEqual(1);
  }, 120_000);

  run("E39: self-referential request", async () => {
    reset(); const r = await agent("How many tools do you have access to? List them and describe what each one does.", ct);
    log("E39", r); expect(r.text).toMatch(/getUser|listUsers|createOrder|tool/i);
  }, 120_000);

  run("E40: vague request requiring inference", async () => {
    reset(); const r = await agent("Which users should we focus on retaining? Explain your reasoning using the data.", ct);
    log("E40", r); expect(r.text).toMatch(/retain|churn|credit|risk|active/i);
  }, 120_000);

  run("E41: ordering for all users including inactive", async () => {
    reset(); const r = await agent("Try to order Basic Plan for ALL 10 users. Report exactly which ones succeed and which fail and why.", ct);
    log("E41", r); expect(r.text).toMatch(/inactive|charlie|hank|fail/i);
  }, 120_000);

  run("E42: update then read consistency", async () => {
    reset(); const r = await agent("Set user 5 (Eve) credits to 999. Then immediately read her back. Confirm the update stuck.", ct);
    log("E42", r); expect(r.text).toMatch(/999|eve|update|confirm/i);
  }, 120_000);

  run("E43: large number arithmetic", async () => {
    reset(); const r = await agent("Use mathCompute to sum these: [99999.99, 88888.88, 77777.77, 66666.66, 55555.55]. What's the total?", ct);
    log("E43", r); expect(r.text).toMatch(/388888|388889/i);
  }, 120_000);

  run("E44: deeply nested conditional", async () => {
    reset();
    const r = await agent(
      "For user 9 (Iris): if she's active AND has 'vip' tag AND credits > 400, order Enterprise Plan. " +
      "If active + vip but credits <= 400, order Pro Plan. If active but not vip, order Basic Plan. If inactive, skip. " +
      "Report which path was taken.",
      ct
    );
    log("E44", r); expect(r.text).toMatch(/iris|enterprise|pro|basic|plan/i);
  }, 120_000);

  run("E45: explain what you did", async () => {
    reset();
    const r = await agent("List all products rated above 4.5. Don't just give the answer — show your reasoning step by step.", ct);
    log("E45", r); expect(r.text).toMatch(/pro plan|enterprise|api credits|4\.[5-9]/i);
  }, 120_000);

  // === COMPLEX MULTI-ENTITY WORKFLOWS (E46-E55) ===

  run("E46: create tickets for inactive users", async () => {
    reset(); const r = await agent("Find all inactive users. Create a 'high' priority ticket for each: 'Account reactivation review for [name]'. Report the ticket IDs.", ct);
    log("E46", r); expect(DB.tickets.length).toBeGreaterThanOrEqual(2);
  }, 120_000);

  run("E47: assign tickets to moderators round-robin", async () => {
    reset();
    DB.tickets.push({ id: "TKT-A", userId: 1, subject: "Bug A", priority: "high", status: "open", assignedTo: null });
    DB.tickets.push({ id: "TKT-B", userId: 2, subject: "Bug B", priority: "medium", status: "open", assignedTo: null });
    DB.tickets.push({ id: "TKT-C", userId: 5, subject: "Bug C", priority: "low", status: "open", assignedTo: null });
    const r = await agent("Find all moderators. Assign all unassigned open tickets to them in round-robin order. Report assignments.", ct);
    log("E47", r); expect(r.text).toMatch(/diana|jack|assign/i);
  }, 120_000);

  run("E48: order + ticket + notify + audit", async () => {
    reset();
    const r = await agent(
      "User 1 (Alice) wants Enterprise Plan (A003). Place the order, create a 'low' priority ticket 'Onboarding: Alice', " +
      "send her an email 'Welcome to Enterprise!', and log audit 'enterprise_upgrade' by 'system'. Report all IDs.",
      ct
    );
    log("E48", r);
    expect(DB.orders.length).toBeGreaterThanOrEqual(1);
    expect(DB.tickets.length).toBeGreaterThanOrEqual(1);
  }, 120_000);

  run("E49: bulk credit top-up for trial users", async () => {
    reset(); const r = await agent("Find all users tagged 'trial'. Add 50 credits to each. Report before/after credits.", ct);
    log("E49", r); expect(r.text).toMatch(/bob|eve|trial|credit/i);
  }, 120_000);

  run("E50: cascade: order triggers notification triggers audit", async () => {
    reset();
    const r = await agent(
      "Order 1 Widget (B001) for user 7 (Grace). After the order succeeds, send her a push notification with the order total. " +
      "After the notification, log an audit entry with the order ID. Chain all 3 actions.",
      ct
    );
    log("E50", r); expect(DB.orders.length + DB.notifications.length + DB.auditLog.length).toBeGreaterThanOrEqual(3);
  }, 120_000);

  run("E51: multi-user order batch with mixed results", async () => {
    reset();
    const r = await agent(
      "Try ordering Pro Plan (A002) for users 2, 3, 5, and 6. Some will fail (inactive, insufficient credits). " +
      "Collect all results and return a table of userId, success/fail, reason.",
      ct
    );
    log("E51", r); expect(r.text).toMatch(/success|fail|insufficient|inactive/i);
  }, 120_000);

  run("E52: find best product per category", async () => {
    reset(); const r = await agent("For each product category, find the highest-rated product. Return category, product name, rating.", ct);
    log("E52", r); expect(r.text).toMatch(/subscription|physical|credits|addon/i);
  }, 120_000);

  run("E53: user spending report with orders", async () => {
    reset();
    // Pre-populate orders
    DB.orders.push({ id: "O-1", userId: 1, sku: "A002", qty: 1, total: 29.99, status: "confirmed", createdAt: "2024-01-15" });
    DB.orders.push({ id: "O-2", userId: 1, sku: "B001", qty: 10, total: 49.90, status: "confirmed", createdAt: "2024-02-01" });
    DB.orders.push({ id: "O-3", userId: 6, sku: "A003", qty: 1, total: 99.99, status: "confirmed", createdAt: "2024-01-20" });
    DB.orders.push({ id: "O-4", userId: 2, sku: "A001", qty: 1, total: 9.99, status: "confirmed", createdAt: "2024-03-01" });
    const r = await agent("Get all orders. Group by userId, compute total spend per user. Who spent the most?", ct);
    log("E53", r); expect(r.text).toMatch(/alice|79\.89|frank|99\.99|spend/i);
  }, 120_000);

  run("E54: reactivate churned users with credits", async () => {
    reset();
    const r = await agent(
      "Find users tagged 'churned'. Reactivate them (active=true), give them 100 bonus credits, " +
      "remove 'churned' tag and add 'reactivated' tag. Send each an email: 'Welcome back! 100 credits added.' Report changes.",
      ct
    );
    log("E54", r);
    const charlie = DB.users.find(u => u.id === 3);
    expect(charlie?.active).toBe(true);
  }, 120_000);

  run("E55: SLA check — find overdue tickets", async () => {
    reset();
    DB.tickets.push({ id: "TKT-OLD1", userId: 2, subject: "Login broken", priority: "critical", status: "open", assignedTo: null });
    DB.tickets.push({ id: "TKT-OLD2", userId: 5, subject: "Slow page", priority: "low", status: "open", assignedTo: 4 });
    DB.tickets.push({ id: "TKT-OLD3", userId: 1, subject: "Feature req", priority: "medium", status: "closed", assignedTo: 10 });
    const r = await agent("List all open tickets. Critical unassigned tickets are SLA violations. Report any violations and assign them to a moderator.", ct);
    log("E55", r); expect(r.text).toMatch(/critical|SLA|violation|assign|TKT-OLD1/i);
  }, 120_000);

  // === EDGE CASES IN TOOL PATTERNS (E56-E65) ===

  run("E56: tool call returns null", async () => {
    reset(); const r = await agent("Get user 999. It doesn't exist. Handle the null/error and report gracefully.", ct);
    log("E56", r); expect(r.text).toMatch(/not found|error|exist|999/i);
  }, 120_000);

  run("E57: empty list result", async () => {
    reset(); const r = await agent("List all users with role 'ceo'. Handle the empty result.", ct);
    log("E57", r); expect(r.text).toMatch(/no|none|empty|0|ceo/i);
  }, 120_000);

  run("E58: tool returns large payload", async () => {
    reset(); const r = await agent("List ALL users AND all products in a single codemode call. Return combined counts.", ct);
    log("E58", r); expect(r.text).toMatch(/10|user|product/i);
  }, 120_000);

  run("E59: sequential dependency chain", async () => {
    reset();
    const r = await agent(
      "Step 1: Get user 9 (Iris). Step 2: Use her credits value to get the product closest to that price. " +
      "Step 3: Order 1 unit of that product for her. Each step depends on the previous.",
      ct
    );
    log("E59", r); expect(r.tc).toBeGreaterThanOrEqual(2);
  }, 120_000);

  run("E60: parallel-style fetches", async () => {
    reset();
    const r = await agent("Get users 1, 2, 3, 4, 5 simultaneously (5 getUser calls), then return their names as a list.", ct);
    log("E60", r); expect(r.text).toMatch(/alice.*bob.*charlie|alice|bob/i);
  }, 120_000);

  run("E61: conditional tool call — only call if needed", async () => {
    reset();
    const r = await agent("Check if user 1 (Alice) has 'vip' tag. If yes, she already has benefits — just report that. If no, order Basic Plan for her.", ct);
    log("E61", r); expect(r.text).toMatch(/vip|already|benefit/i);
  }, 120_000);

  run("E62: retry pattern — first attempt fails, try alternative", async () => {
    reset();
    const r = await agent(
      "Try ordering Gadget (B002) for user 2. It's out of stock. Try Widget (B001) instead. If that also fails, try Sprocket (B003). Order whichever succeeds first.",
      ct
    );
    log("E62", r); expect(r.text).toMatch(/widget|sprocket|B001|B003/i);
  }, 120_000);

  run("E63: aggregation across tool calls", async () => {
    reset();
    const r = await agent(
      "Get products A001, A002, A003 individually (3 getProduct calls). Compute average price using mathCompute. Report it.",
      ct
    );
    log("E63", r); expect(r.tc).toBeGreaterThanOrEqual(3);
  }, 120_000);

  run("E64: write-then-verify pattern", async () => {
    reset();
    const r = await agent(
      "Update user 7 (Grace) to role 'moderator'. Then getUser 7 again to verify the role changed. Confirm it worked.",
      ct
    );
    log("E64", r); expect(r.text).toMatch(/moderator|grace|verify|confirm/i);
  }, 120_000);

  run("E65: tool error doesn't crash sandbox", async () => {
    reset();
    const r = await agent(
      "In a single codemode call: try getProduct('INVALID'), catch the error, then successfully getProduct('A001'). " +
      "Return both results — the error and the success.",
      ct
    );
    log("E65", r); expect(r.text).toMatch(/error|not found|basic plan|A001/i);
  }, 120_000);

  // === REAL-WORLD AGENT SCENARIOS (E66-E75) ===

  run("E66: customer support triage", async () => {
    reset();
    const r = await agent(
      "A customer (user 2, Bob) reports 'My page won't load'. Create a medium-priority ticket, " +
      "check if he has active orders, and send him a push notification: 'We received your report. Ticket: [ID]'.",
      ct
    );
    log("E66", r); expect(DB.tickets.length).toBeGreaterThanOrEqual(1);
  }, 120_000);

  run("E67: subscription renewal check", async () => {
    reset();
    const r = await agent(
      "Check all active users. For each user with credits < 50, send them an email: " +
      "'Your balance is low. Top up to continue your subscription.' List who was notified.",
      ct
    );
    log("E67", r); expect(r.text).toMatch(/eve|notification|low|balance/i);
  }, 120_000);

  run("E68: admin dashboard data", async () => {
    reset();
    DB.orders.push({ id: "O-A", userId: 1, sku: "A002", qty: 1, total: 29.99, status: "confirmed", createdAt: "2024-01-15" });
    DB.orders.push({ id: "O-B", userId: 6, sku: "C002", qty: 2, total: 160.00, status: "confirmed", createdAt: "2024-01-20" });
    const r = await agent(
      "Build an admin dashboard summary: total users, active/inactive split, total revenue from orders, " +
      "total products, out-of-stock count, and open ticket count.",
      ct
    );
    log("E68", r); expect(r.text).toMatch(/10|user|revenue|product/i);
  }, 120_000);

  run("E69: permission check before action", async () => {
    reset();
    const r = await agent(
      "User 2 (Bob, role: 'user') wants to update user 5's credits to 500. " +
      "Check if Bob is an admin first. If not, deny the request and explain why.",
      ct
    );
    log("E69", r); expect(r.text).toMatch(/deny|not.*admin|permission|unauthorized|cannot/i);
  }, 120_000);

  run("E70: inventory alert system", async () => {
    reset();
    const r = await agent(
      "Check all physical products. If any have stock <= 10 or are out of stock, send a push notification to all admins " +
      "listing the low-stock items. Report what was found.",
      ct
    );
    log("E70", r); expect(r.text).toMatch(/gadget|out of stock|admin|alert/i);
  }, 120_000);

  run("E71: loyalty program — upgrade VIPs", async () => {
    reset();
    const r = await agent(
      "Find all users tagged 'vip' with credits > 300. Upgrade their role to 'moderator' if they're not already admin. " +
      "Send each upgraded user an email about their new perks. Report changes.",
      ct
    );
    log("E71", r); expect(r.text).toMatch(/vip|upgrade|moderator|diana|iris/i);
  }, 120_000);

  run("E72: refund simulation", async () => {
    reset();
    DB.orders.push({ id: "O-REF", userId: 2, sku: "A002", qty: 1, total: 29.99, status: "confirmed", createdAt: "2024-01-15" });
    const r = await agent(
      "Process a refund for order O-REF (Bob's Pro Plan). Add 29.99 back to his credits, " +
      "log audit 'refund_processed' with the order ID, and notify Bob via email. What are his credits after?",
      ct
    );
    log("E72", r); expect(r.text).toMatch(/179\.99|180|bob|refund|credit/i);
  }, 120_000);

  run("E73: data export — CSV format", async () => {
    reset();
    const r = await agent("Export all active users as CSV format (id, name, email, credits). Return the CSV string.", ct);
    log("E73", r); expect(r.text).toMatch(/id.*name.*email|alice|bob|csv/i);
  }, 120_000);

  run("E74: multi-step approval workflow", async () => {
    reset();
    const r = await agent(
      "User 4 (Diana, moderator) requests Enterprise Plan (A003, $99.99). " +
      "Step 1: Check if her role allows (moderators need admin approval — create a ticket instead). " +
      "Step 2: Create a 'high' priority ticket 'Enterprise upgrade request: Diana'. " +
      "Step 3: Assign it to an admin (pick one). " +
      "Step 4: Notify Diana via push: 'Your upgrade request is pending admin approval.'",
      ct
    );
    log("E74", r);
    expect(DB.tickets.length).toBeGreaterThanOrEqual(1);
    expect(DB.notifications.length).toBeGreaterThanOrEqual(1);
  }, 120_000);

  run("E75: end-of-month reconciliation", async () => {
    reset();
    DB.orders.push({ id: "O-M1", userId: 1, sku: "A002", qty: 1, total: 29.99, status: "confirmed", createdAt: "2024-03-05" });
    DB.orders.push({ id: "O-M2", userId: 6, sku: "C001", qty: 5, total: 50.00, status: "confirmed", createdAt: "2024-03-12" });
    DB.orders.push({ id: "O-M3", userId: 9, sku: "A001", qty: 1, total: 9.99, status: "confirmed", createdAt: "2024-03-20" });
    DB.orders.push({ id: "O-M4", userId: 4, sku: "D001", qty: 2, total: 30.00, status: "confirmed", createdAt: "2024-03-28" });
    const r = await agent(
      "Run end-of-month reconciliation: total orders, total revenue, average order value (use mathCompute), " +
      "and send an email to all admins with the summary. Report everything.",
      ct
    );
    log("E75", r); expect(r.text).toMatch(/order|revenue|average|admin/i);
  }, 120_000);
});
