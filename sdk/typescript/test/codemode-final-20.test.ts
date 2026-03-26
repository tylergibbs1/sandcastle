/**
 * Final 20 Code Mode experiments — testing research-inspired patterns.
 *
 * Focus: token efficiency, long-context handling, multi-step reasoning,
 * error recovery loops, batch processing, and complex real-world agent workflows.
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
// Simulated services — richer than before
// ---------------------------------------------------------------------------

const DB = {
  employees: [
    { id: 1, name: "Alice Chen", dept: "engineering", salary: 145000, level: "senior", hireDate: "2021-03-15", reviews: [4.5, 4.8, 4.7] },
    { id: 2, name: "Bob Martinez", dept: "engineering", salary: 95000, level: "mid", hireDate: "2023-01-10", reviews: [3.8, 4.0] },
    { id: 3, name: "Carol Davis", dept: "sales", salary: 120000, level: "senior", hireDate: "2020-06-01", reviews: [4.2, 4.5, 4.3, 4.6] },
    { id: 4, name: "Dan Lee", dept: "sales", salary: 75000, level: "junior", hireDate: "2024-09-01", reviews: [3.5] },
    { id: 5, name: "Eve Wilson", dept: "marketing", salary: 110000, level: "senior", hireDate: "2022-01-20", reviews: [4.0, 4.2, 4.1] },
    { id: 6, name: "Frank Brown", dept: "engineering", salary: 165000, level: "staff", hireDate: "2019-07-01", reviews: [4.9, 4.8, 4.9, 4.7, 4.8] },
    { id: 7, name: "Grace Kim", dept: "marketing", salary: 85000, level: "mid", hireDate: "2023-06-15", reviews: [3.9, 4.1] },
    { id: 8, name: "Hank Patel", dept: "engineering", salary: 130000, level: "senior", hireDate: "2021-11-01", reviews: [4.3, 4.5, 4.4] },
    { id: 9, name: "Iris Zhang", dept: "product", salary: 140000, level: "senior", hireDate: "2020-09-15", reviews: [4.6, 4.7, 4.5, 4.8] },
    { id: 10, name: "Jack Thompson", dept: "product", salary: 90000, level: "mid", hireDate: "2024-02-01", reviews: [3.7, 4.0] },
    { id: 11, name: "Kate Murphy", dept: "engineering", salary: 155000, level: "staff", hireDate: "2018-03-01", reviews: [4.8, 4.9, 4.7, 4.8, 4.9, 4.8] },
    { id: 12, name: "Leo Garcia", dept: "sales", salary: 100000, level: "mid", hireDate: "2022-08-15", reviews: [4.0, 4.2, 4.1] },
  ],
  budgets: {
    engineering: 800000,
    sales: 400000,
    marketing: 250000,
    product: 300000,
  } as Record<string, number>,
  projects: [
    { id: "P1", name: "Platform Rewrite", dept: "engineering", status: "active", budget: 200000, members: [1, 6, 8] },
    { id: "P2", name: "Mobile App", dept: "engineering", status: "active", budget: 150000, members: [2, 11] },
    { id: "P3", name: "Q1 Campaign", dept: "marketing", status: "completed", budget: 50000, members: [5, 7] },
    { id: "P4", name: "Enterprise Sales", dept: "sales", status: "active", budget: 100000, members: [3, 12] },
    { id: "P5", name: "Product Roadmap", dept: "product", status: "active", budget: 75000, members: [9, 10] },
    { id: "P6", name: "AI Features", dept: "engineering", status: "planning", budget: 300000, members: [1, 6, 8, 11] },
  ],
  actions: [] as Array<{ type: string; details: unknown }>,
};

const tools: ToolDefinition[] = [
  {
    name: "listEmployees",
    description: "List employees. Filter by dept, level, or minSalary.",
    inputSchema: { type: "object", properties: { dept: { type: "string" }, level: { type: "string" }, minSalary: { type: "number" } } },
    execute: async (i) => {
      let e = [...DB.employees]; const f = (i ?? {}) as any;
      if (f.dept) e = e.filter(x => x.dept === f.dept);
      if (f.level) e = e.filter(x => x.level === f.level);
      if (f.minSalary) e = e.filter(x => x.salary >= f.minSalary);
      return e;
    },
  },
  {
    name: "getEmployee",
    description: "Get employee by ID.",
    inputSchema: { type: "object", properties: { id: { type: "number" } }, required: ["id"] },
    execute: async (i) => DB.employees.find(e => e.id === (i as any).id) ?? { error: "Not found" },
  },
  {
    name: "updateEmployee",
    description: "Update employee fields (salary, level, dept).",
    inputSchema: { type: "object", properties: { id: { type: "number" }, salary: { type: "number" }, level: { type: "string" }, dept: { type: "string" } }, required: ["id"] },
    execute: async (i) => { const { id, ...u } = i as any; const e = DB.employees.find(x => x.id === id); if (!e) return { error: "Not found" }; Object.assign(e, u); return e; },
  },
  {
    name: "listProjects",
    description: "List projects. Filter by dept, status.",
    inputSchema: { type: "object", properties: { dept: { type: "string" }, status: { type: "string" } } },
    execute: async (i) => { let p = [...DB.projects]; const f = (i ?? {}) as any; if (f.dept) p = p.filter(x => x.dept === f.dept); if (f.status) p = p.filter(x => x.status === f.status); return p; },
  },
  {
    name: "getBudget",
    description: "Get department budget.",
    inputSchema: { type: "object", properties: { dept: { type: "string" } }, required: ["dept"] },
    execute: async (i) => ({ dept: (i as any).dept, budget: DB.budgets[(i as any).dept] ?? 0 }),
  },
  {
    name: "recordAction",
    description: "Record an action/decision for audit trail.",
    inputSchema: { type: "object", properties: { type: { type: "string" }, details: {} }, required: ["type"] },
    execute: async (i) => { const a = i as any; DB.actions.push({ type: a.type, details: a.details ?? null }); return { recorded: true, count: DB.actions.length }; },
  },
  {
    name: "compute",
    description: "Math operations on arrays: sum, average, median, min, max, stddev, percentile.",
    inputSchema: { type: "object", properties: { op: { type: "string" }, values: { type: "array", items: { type: "number" } }, p: { type: "number" } }, required: ["op", "values"] },
    execute: async (i) => {
      const { op, values, p } = i as any; const s = [...values].sort((a: number, b: number) => a - b);
      const ops: Record<string, () => number> = {
        sum: () => values.reduce((a: number, b: number) => a + b, 0),
        average: () => values.reduce((a: number, b: number) => a + b, 0) / values.length,
        median: () => { const m = Math.floor(s.length / 2); return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2; },
        min: () => s[0], max: () => s[s.length - 1],
        stddev: () => { const avg = values.reduce((a: number, b: number) => a + b, 0) / values.length; return Math.sqrt(values.reduce((s: number, v: number) => s + (v - avg) ** 2, 0) / values.length); },
        percentile: () => s[Math.max(0, Math.ceil((p ?? 50) / 100 * s.length) - 1)],
      };
      return ops[op] ? { result: +ops[op]().toFixed(2) } : { error: "Unknown op" };
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
    const r = await client.messages.create({ model: "claude-haiku-4-5-20251001", max_tokens: 4096, messages: msgs, tools: [tool] });
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
  DB.actions.length = 0;
  // Restore employee data to original values
  const originals = [
    { id: 1, salary: 145000, level: "senior", dept: "engineering" },
    { id: 2, salary: 95000, level: "mid", dept: "engineering" },
    { id: 3, salary: 120000, level: "senior", dept: "sales" },
    { id: 4, salary: 75000, level: "junior", dept: "sales" },
    { id: 5, salary: 110000, level: "senior", dept: "marketing" },
    { id: 6, salary: 165000, level: "staff", dept: "engineering" },
    { id: 7, salary: 85000, level: "mid", dept: "marketing" },
    { id: 8, salary: 130000, level: "senior", dept: "engineering" },
    { id: 9, salary: 140000, level: "senior", dept: "product" },
    { id: 10, salary: 90000, level: "mid", dept: "product" },
    { id: 11, salary: 155000, level: "staff", dept: "engineering" },
    { id: 12, salary: 100000, level: "mid", dept: "sales" },
  ];
  for (const o of originals) {
    const e = DB.employees.find(x => x.id === o.id);
    if (e) Object.assign(e, o);
  }
  // Restore budgets
  DB.budgets.engineering = 800000;
  DB.budgets.sales = 400000;
  DB.budgets.marketing = 250000;
  DB.budgets.product = 300000;
  // Restore project budgets/members
  DB.projects[0].budget = 200000; DB.projects[0].members = [1, 6, 8];
  DB.projects[1].budget = 150000; DB.projects[1].members = [2, 11];
  DB.projects[2].budget = 50000; DB.projects[2].status = "completed";
  DB.projects[3].budget = 100000;
  DB.projects[4].budget = 75000;
  DB.projects[5].budget = 300000; DB.projects[5].status = "planning";
}
function log(n: string, r: R) { console.log(`  [${r.rt}rt/${r.tc}tc/${r.ms}ms] ${n}: ${r.text.slice(0, 120).replace(/\n/g, " ")}...`); }

// ---------------------------------------------------------------------------
// 20 Experiments
// ---------------------------------------------------------------------------

describe("Final 20 experiments", () => {
  const executor = new TwoPassExecutor({ binaryPath: BINARY_PATH, guestModule: GUEST_MODULE });
  const ct = createCodeTool({ tools, executor });

  // --- Token efficiency patterns (research-inspired) ---

  run("F01: complex analytics in single sandbox call", async () => {
    reset();
    const r = await agent(
      "Analyze the engineering department: average salary, salary range (min to max), average review score across all engineers, " +
      "and headcount per level. Do all of this efficiently.",
      ct
    );
    log("F01", r); expect(r.text).toMatch(/engineer|salary|average|senior|staff/i);
  }, 60_000);

  run("F02: cross-department budget utilization", async () => {
    reset();
    const r = await agent(
      "For each department, calculate: total employee salary cost, department budget, utilization % (salary/budget), " +
      "and whether they're over or under budget. Return a summary table.",
      ct
    );
    log("F02", r); expect(r.text).toMatch(/engineering|sales|marketing|product|budget|%/i);
  }, 60_000);

  run("F03: employee performance ranking with weighted score", async () => {
    reset();
    const r = await agent(
      "Rank all employees by a weighted performance score: average review * 40 + (years of tenure * 10) + (salary percentile within dept * 50). " +
      "Show top 5 with their scores and breakdown.",
      ct
    );
    log("F03", r); expect(r.text).toMatch(/\d+.*score|rank|top/i);
  }, 60_000);

  run("F04: project staffing analysis", async () => {
    reset();
    const r = await agent(
      "For each active project, list the members by name, their average review scores, and the project's total salary cost (sum of member salaries). " +
      "Which project has the highest-rated team?",
      ct
    );
    log("F04", r); expect(r.text).toMatch(/project|member|review|salary/i);
  }, 60_000);

  // --- Multi-step decision making ---

  run("F05: salary adjustment recommendations", async () => {
    reset();
    const r = await agent(
      "Review all employees. Recommend a salary adjustment for anyone whose average review score is above 4.5 but whose salary " +
      "is below the department median. Suggest a 10% raise for each. List who qualifies and the new proposed salary.",
      ct
    );
    log("F05", r); expect(r.text).toMatch(/raise|salary|recommend|adjust/i);
  }, 60_000);

  run("F06: promotion candidates with validation", async () => {
    reset();
    const r = await agent(
      "Find promotion candidates: employees at 'mid' level with average review >= 4.0 and at least 2 reviews. " +
      "For each candidate, verify their department has budget headroom (total salaries < 90% of budget). " +
      "List qualified candidates with reasoning.",
      ct
    );
    log("F06", r); expect(r.text).toMatch(/promot|candidate|mid|review/i);
  }, 60_000);

  run("F07: headcount planning", async () => {
    reset();
    const r = await agent(
      "Management wants to hire 3 more engineers. Calculate: current engineering salary total, remaining budget, " +
      "average engineering salary (to estimate cost of 3 new hires), and whether the budget can support it. " +
      "If not, suggest how many hires the budget can actually support.",
      ct
    );
    log("F07", r); expect(r.text).toMatch(/engineer|budget|hire|salary/i);
  }, 60_000);

  // --- Error handling and edge cases ---

  run("F08: handle missing data gracefully", async () => {
    reset();
    const r = await agent("Get employee #99 (doesn't exist) and employee #1 (Alice). Handle the missing one gracefully and report both results.", ct);
    log("F08", r); expect(r.text).toMatch(/not found|alice|error|missing/i);
  }, 60_000);

  run("F09: department with no projects", async () => {
    reset();
    const r = await agent(
      "List all departments and their active project count. Some departments might have zero active projects — handle that correctly.",
      ct
    );
    log("F09", r); expect(r.text).toMatch(/engineering|marketing|0|project/i);
  }, 60_000);

  run("F10: conflicting constraints", async () => {
    reset();
    const r = await agent(
      "Find employees who are 'senior' level but have an average review below 4.0. If none exist, say so clearly.",
      ct
    );
    log("F10", r); expect(r.text).toMatch(/no|none|senior|review/i);
  }, 60_000);

  // --- Complex workflows with audit trail ---

  run("F11: annual review process", async () => {
    reset();
    const r = await agent(
      "Run the annual review process: for each employee, compute their average review score, " +
      "determine if they're eligible for promotion (mid level + avg >= 4.0 + 2+ reviews), " +
      "and record an action for each decision. Report the summary.",
      ct
    );
    log("F11", r); expect(DB.actions.length).toBeGreaterThanOrEqual(1);
  }, 90_000);

  run("F12: budget reallocation", async () => {
    reset();
    const r = await agent(
      "Marketing completed project P3 ($50K budget). Reallocate that $50K to engineering's AI Features project (P6). " +
      "Record an audit action for the reallocation with the details.",
      ct
    );
    log("F12", r); expect(r.text).toMatch(/reallocat|budget|50|market|engineer/i);
  }, 60_000);

  // --- Data transformation patterns ---

  run("F13: generate org chart data", async () => {
    reset();
    const r = await agent(
      "Generate an org chart summary: for each department, list employees grouped by level (staff → senior → mid → junior), " +
      "with their names and salary. Format as a hierarchical structure.",
      ct
    );
    log("F13", r); expect(r.text).toMatch(/engineering|staff|senior|mid/i);
  }, 60_000);

  run("F14: compensation equity analysis", async () => {
    reset();
    const r = await agent(
      "Run a compensation equity analysis: for each level (junior, mid, senior, staff), compute the salary mean, " +
      "median, and standard deviation using the compute tool. Flag any employee whose salary is more than 1 stddev " +
      "below their level's mean. Report findings.",
      ct
    );
    log("F14", r); expect(r.text).toMatch(/mean|median|stddev|salary|level/i);
  }, 90_000);

  run("F15: tenure vs performance correlation", async () => {
    reset();
    const r = await agent(
      "For all employees, compute years of tenure (from hireDate to now). Then group them into buckets: " +
      "<1 year, 1-2 years, 2-4 years, 4+ years. For each bucket, compute average review score. " +
      "Is there a correlation between tenure and performance?",
      ct
    );
    log("F15", r); expect(r.text).toMatch(/tenure|year|review|correlat|bucket/i);
  }, 60_000);

  // --- Stress patterns ---

  run("F16: all employees all projects all stats", async () => {
    reset();
    const r = await agent(
      "Get ALL employees AND all projects. For each employee, list which projects they're on. " +
      "For each project, compute the team's average salary and average review score. " +
      "Return everything.",
      ct
    );
    log("F16", r); expect(r.tc).toBeGreaterThanOrEqual(1);
  }, 90_000);

  run("F17: what-if scenario modeling", async () => {
    reset();
    const r = await agent(
      "Model this scenario: if we give a 15% raise to all employees with avg review > 4.5, " +
      "what would be the new total salary cost per department? Compare with current cost and budget. " +
      "Which departments would go over budget?",
      ct
    );
    log("F17", r); expect(r.text).toMatch(/raise|15%|budget|department|cost/i);
  }, 60_000);

  run("F18: multi-criteria ranking with tiebreakers", async () => {
    reset();
    const r = await agent(
      "Rank all senior employees by: (1) average review score, (2) tenure as tiebreaker, (3) salary as second tiebreaker. " +
      "Show the ranked list with all three criteria values.",
      ct
    );
    log("F18", r); expect(r.text).toMatch(/rank|senior|review|tenure/i);
  }, 60_000);

  run("F19: batch update with validation", async () => {
    reset();
    const r = await agent(
      "Promote all 'mid' level employees with avg review >= 4.0 to 'senior'. " +
      "For each promotion, increase salary by 20%. " +
      "Record an audit action for each promotion. " +
      "Report who was promoted and their new salary.",
      ct
    );
    log("F19", r); expect(DB.actions.length).toBeGreaterThanOrEqual(1);
  }, 90_000);

  run("F20: executive summary report", async () => {
    reset();
    const r = await agent(
      "Generate an executive summary: total headcount, total salary spend, average salary, " +
      "headcount and spend per department, active project count and total project budget, " +
      "top 3 highest-rated employees, and any departments over 80% budget utilization. " +
      "Make it concise and actionable.",
      ct
    );
    log("F20", r); expect(r.text).toMatch(/headcount|salary|budget|department|project/i);
  }, 90_000);
});
