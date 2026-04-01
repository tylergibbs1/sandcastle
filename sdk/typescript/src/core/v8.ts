import { randomUUID } from "node:crypto";
import type {
  ConsoleEntry,
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  ExecutionStatus,
  ExecutionTranscript,
  OutputValue,
} from "../types/execution.js";
import type { HostFunction, OnConsoleCallback, V8PoolOptions } from "../types/config.js";

// ---------------------------------------------------------------------------
// Lazy isolated-vm loader
// ---------------------------------------------------------------------------

let ivm: typeof import("isolated-vm") | null = null;

async function getIvm() {
  if (!ivm) {
    try {
      ivm = await import("isolated-vm");
    } catch {
      throw new Error(
        "isolated-vm is required for V8 mode. Install it: npm install isolated-vm",
      );
    }
  }
  return ivm;
}

const LIMIT_DEFAULTS: Required<ExecutionLimits> = {
  memoryMb: 128,
  timeoutMs: 10_000,
  fuel: 0,
  maxOutputBytes: 1_048_576,
};

// ---------------------------------------------------------------------------
// Isolate Pool — reuse warm V8 isolates across executions
// ---------------------------------------------------------------------------

interface PooledIsolate {
  isolate: InstanceType<typeof import("isolated-vm").Isolate>;
  context: InstanceType<typeof import("isolated-vm").Context>;
  createdAt: number;
  uses: number;
}

export class IsolatePool {
  private pool: PooledIsolate[] = [];
  private readonly maxSize: number;
  private readonly maxAge: number;
  private readonly maxUses: number;
  private readonly memoryMb: number;
  private readonly snapshot: InstanceType<typeof import("isolated-vm").ExternalCopy<ArrayBuffer>> | null;

  constructor(opts: V8PoolOptions & { memoryMb: number; snapshot?: InstanceType<typeof import("isolated-vm").ExternalCopy<ArrayBuffer>> | null }) {
    this.maxSize = opts.maxIsolates ?? 8;
    this.maxAge = opts.maxAgeMs ?? 30_000;
    this.maxUses = opts.maxUsesPerIsolate ?? 1000;
    this.memoryMb = opts.memoryMb;
    this.snapshot = opts.snapshot ?? null;
  }

  acquire(): PooledIsolate | null {
    const now = Date.now();
    while (this.pool.length > 0) {
      const entry = this.pool.pop()!;
      if (entry.isolate.isDisposed) continue;
      if (now - entry.createdAt > this.maxAge) {
        entry.isolate.dispose();
        continue;
      }
      if (entry.uses >= this.maxUses) {
        entry.isolate.dispose();
        continue;
      }
      return entry;
    }
    return null;
  }

  release(entry: PooledIsolate) {
    entry.uses++;
    if (this.pool.length < this.maxSize && !entry.isolate.isDisposed) {
      this.pool.push(entry);
    } else if (!entry.isolate.isDisposed) {
      entry.isolate.dispose();
    }
  }

  async createNew(): Promise<PooledIsolate> {
    const iv = await getIvm();
    const opts: Record<string, unknown> = { memoryLimit: this.memoryMb };
    if (this.snapshot) {
      opts.snapshot = this.snapshot;
    }
    const isolate = new iv.Isolate(opts as ConstructorParameters<typeof iv.Isolate>[0]);
    const context = await isolate.createContext();
    return {
      isolate,
      context,
      createdAt: Date.now(),
      uses: 0,
    };
  }

  dispose() {
    for (const entry of this.pool) {
      if (!entry.isolate.isDisposed) entry.isolate.dispose();
    }
    this.pool = [];
  }
}

// ---------------------------------------------------------------------------
// Snapshot support — pre-warm V8 with libraries
// ---------------------------------------------------------------------------

export async function createSnapshot(scripts: string[]): Promise<InstanceType<typeof import("isolated-vm").ExternalCopy<ArrayBuffer>>> {
  const iv = await getIvm();
  return iv.Isolate.createSnapshot(
    scripts.map((code) => ({ code })),
  );
}

// ---------------------------------------------------------------------------
// Main execution
// ---------------------------------------------------------------------------

export async function executeViaV8(
  req: ExecuteOptions,
  defaults?: ExecutionLimits,
  hostFunctions?: Record<string, HostFunction>,
  onConsole?: OnConsoleCallback,
  pool?: IsolatePool | null,
): Promise<ExecutionResult> {
  const iv = await getIvm();
  const limits: Required<ExecutionLimits> = {
    ...LIMIT_DEFAULTS,
    ...defaults,
    ...req.limits,
  };

  const executionId = randomUUID();
  const startedAt = new Date().toISOString();
  const startTime = performance.now();
  const consoleMessages: ConsoleEntry[] = [];

  // Cancelled before start
  if (req.signal?.aborted) {
    return buildResult(
      { type: "cancelled" },
      { type: "null" },
      executionId, startedAt, startTime, consoleMessages, limits, 0,
    );
  }

  // Acquire or create isolate (+ context for pooled mode)
  let poolEntry: PooledIsolate | null = null;
  let isolate: InstanceType<typeof import("isolated-vm").Isolate>;
  let context: InstanceType<typeof import("isolated-vm").Context>;
  let ownsIsolate: boolean;

  if (pool) {
    poolEntry = pool.acquire() ?? await pool.createNew();
    isolate = poolEntry.isolate;
    context = poolEntry.context;
    ownsIsolate = false;
  } else {
    isolate = new iv.Isolate({ memoryLimit: limits.memoryMb });
    context = await isolate.createContext();
    ownsIsolate = true;
  }

  try {
    const jail = context.global;

    // --- Console with streaming callback ---
    const consoleCallback = new iv.Callback(
      (levelStr: string, message: string) => {
        const level = levelStr as ConsoleEntry["level"];
        const ts = Math.round(performance.now() - startTime);
        consoleMessages.push({ level, message, ts });
        if (onConsole) {
          onConsole(level, message, ts);
        }
      },
      { async: false },
    );
    jail.setSync("__sc_console_cb", consoleCallback);

    context.evalSync(`
      var __sc_timers = {};
      var console = {
        log:   (...a) => __sc_console_cb("log",   a.map(String).join(" ")),
        warn:  (...a) => __sc_console_cb("warn",  a.map(String).join(" ")),
        error: (...a) => __sc_console_cb("error", a.map(String).join(" ")),
        debug: (...a) => __sc_console_cb("debug", a.map(String).join(" ")),
        time:  (label) => { __sc_timers[label || "default"] = Date.now(); },
        timeEnd: (label) => {
          var k = label || "default", s = __sc_timers[k];
          if (s !== undefined) { delete __sc_timers[k]; __sc_console_cb("log", k + ": " + (Date.now() - s) + "ms"); }
        },
        timeLog: (label) => {
          var k = label || "default", s = __sc_timers[k];
          if (s !== undefined) { __sc_console_cb("log", k + ": " + (Date.now() - s) + "ms"); }
        },
      };
    `);

    // --- Inject host functions ---
    if (hostFunctions && Object.keys(hostFunctions).length > 0) {
      for (const [name, fn] of Object.entries(hostFunctions)) {
        const cb = new iv.Callback(
          (...args: unknown[]) => fn(...args),
          { async: false },
        );
        jail.setSync(name, cb);
      }
    }

    // --- Inject input ---
    if (req.input !== undefined) {
      const inputCopy = new iv.ExternalCopy(req.input);
      jail.setSync("__sc_input", inputCopy);
      context.evalSync(
        "var input = __sc_input.copy(); globalThis.__sandcastle_input = input;",
      );
      inputCopy.release();
    } else {
      context.evalSync(
        "var input = undefined; globalThis.__sandcastle_input = undefined;",
      );
    }

    // --- Inject globals ---
    if (req.globals) {
      for (const [key, val] of Object.entries(req.globals)) {
        if (!/^[a-zA-Z_$][a-zA-Z0-9_$]*$/.test(key)) {
          throw new Error(`Invalid global name "${key}": must be a valid JavaScript identifier`);
        }
        const copy = new iv.ExternalCopy(val);
        jail.setSync(`__sc_g_${key}`, copy);
        context.evalSync(`var ${key} = __sc_g_${key}.copy();`);
        copy.release();
      }
    }

    // --- Build expression from user code ---
    const isAsync = /\bawait\b/.test(req.code) || /\basync\b/.test(req.code);
    const trimmed = req.code.trimStart();
    // Fast path: strip `return` and eval as expression (avoids IIFE overhead)
    const expr = trimmed.startsWith("return ") ? trimmed.slice(7) : `(()=>{${req.code}})()`;

    // --- Execute ---
    // Errors are caught externally (faster than try/catch inside sandbox).
    // Results use structured clone ({ copy: true }) instead of JSON roundtrip.
    try {
      let value: unknown;
      if (isAsync) {
        const asyncExpr = `(async()=>{${req.code}})()`;
        value = await context.eval(asyncExpr, { timeout: limits.timeoutMs, promise: true, copy: true });
      } else {
        value = context.evalSync(expr, { timeout: limits.timeoutMs, copy: true });
      }

      const heapUsed = getHeapUsed(isolate);
      const cpuTimeNs = getCpuTime(isolate);
      const output = valueToOutput(value, limits.maxOutputBytes);
      return buildResult(
        { type: "success" }, output,
        executionId, startedAt, startTime, consoleMessages, limits, heapUsed, cpuTimeNs,
      );
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : String(e);
      const stack = e instanceof Error ? e.stack : undefined;
      const heapUsed = getHeapUsed(isolate);
      const cpuTimeNs = getCpuTime(isolate);

      const status = classifyError(message);
      if (status.type === "guest_error" && stack) {
        const guestStack = cleanGuestStack(stack, req.code);
        return buildResult(
          { type: "guest_error", message, guestStack }, { type: "null" },
          executionId, startedAt, startTime, consoleMessages, limits, heapUsed, cpuTimeNs,
        );
      }
      return buildResult(
        status, { type: "null" },
        executionId, startedAt, startTime, consoleMessages, limits, heapUsed, cpuTimeNs,
      );
    }
  } finally {
    if (poolEntry && pool) {
      pool.release(poolEntry);
    } else {
      context.release();
      if (ownsIsolate && !isolate.isDisposed) {
        isolate.dispose();
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function cleanGuestStack(rawStack: string, userCode: string): string {
  const lines = userCode.split("\n");
  // Replace isolated-vm internal references with "sandbox:" prefix
  // and try to show the offending line from user code
  return rawStack
    .replace(/<isolated-vm>/g, "sandbox")
    .replace(/at\s+<isolated-vm\s+boundary>/g, "")
    .split("\n")
    .filter((line) => line.trim().length > 0)
    .slice(0, 10) // limit stack depth
    .join("\n");
}

function classifyError(message: string): ExecutionStatus {
  const lower = message.toLowerCase();
  if (lower.includes("script execution timed out") || lower.includes("timeout")) {
    return { type: "timeout" };
  }
  if (lower.includes("memory") || lower.includes("allocation")) {
    return { type: "memory_exceeded" };
  }
  return { type: "guest_error", message };
}

function valueToOutput(value: unknown, maxBytes: number): OutputValue {
  if (value === null || value === undefined) return { type: "null" };
  if (typeof value === "string") {
    return value.length > maxBytes
      ? { type: "string", value: value.slice(0, maxBytes) }
      : { type: "string", value };
  }
  try {
    const serialized = JSON.stringify(value);
    if (serialized.length > maxBytes) {
      return { type: "string", value: `[output truncated: ${serialized.length} bytes exceeds ${maxBytes} limit]` };
    }
  } catch {
    return { type: "string", value: String(value) };
  }
  return { type: "json", value };
}

function getHeapUsed(isolate: InstanceType<typeof import("isolated-vm").Isolate>): number {
  try {
    return isolate.getHeapStatisticsSync().used_heap_size;
  } catch {
    return 0;
  }
}

function getCpuTime(isolate: InstanceType<typeof import("isolated-vm").Isolate>): number {
  try {
    return Number(isolate.cpuTime) / 1_000_000; // nanoseconds → milliseconds
  } catch {
    return 0;
  }
}

function buildResult(
  status: ExecutionStatus,
  output: OutputValue,
  executionId: string,
  startedAt: string,
  startTime: number,
  consoleMessages: ConsoleEntry[],
  limits: Required<ExecutionLimits>,
  peakMemoryBytes: number,
  cpuTimeMs = 0,
): ExecutionResult {
  const finishedAt = new Date().toISOString();
  const transcript: ExecutionTranscript = {
    executionId, startedAt, finishedAt, status,
    fuelConsumed: 0,
    fuelLimit: limits.fuel,
    peakMemoryBytes,
    memoryLimitBytes: limits.memoryMb * 1024 * 1024,
    output,
    console: consoleMessages,
    capabilityCalls: [],
  };
  const ms = Math.round(performance.now() - startTime);
  const value = output.type === "json" ? output.value : output.type === "string" ? output.value : undefined;

  return {
    ok: status.type === "success",
    status,
    output,
    transcript,
    outputArtifacts: [],
    logs: consoleMessages,
    ms,
    value,
    memoryBytes: peakMemoryBytes,
    cpuTimeMs,
  };
}
