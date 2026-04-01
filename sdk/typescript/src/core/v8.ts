import { randomUUID } from "node:crypto";
import type {
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  ExecutionStatus,
  ExecutionTranscript,
  ConsoleEntry,
  OutputValue,
} from "../types/execution.js";

// isolated-vm is loaded lazily to avoid crashing if not installed
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

/**
 * Execute code in-process using a V8 isolate (via isolated-vm).
 *
 * Each call creates a fresh Isolate + Context for full sandbox isolation,
 * then disposes both after execution. This provides:
 * - ~0.5ms per execution (vs ~90ms subprocess, ~2.5ms HTTP)
 * - Full V8 JIT compilation
 * - console.log/warn/error/debug capture
 * - Timeout enforcement
 * - Memory limit enforcement
 * - Structured execution transcripts
 */
export async function executeViaV8(
  req: ExecuteOptions,
  defaults?: ExecutionLimits,
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

  // Create isolate with memory limit
  const isolate = new iv.Isolate({ memoryLimit: limits.memoryMb });

  try {
    // Check AbortSignal before starting
    if (req.signal?.aborted) {
      return buildResult(
        { type: "cancelled" },
        { type: "null" },
        executionId,
        startedAt,
        startTime,
        consoleMessages,
        limits,
        0,
      );
    }

    const context = await isolate.createContext();
    const jail = context.global;

    // --- Inject console ---
    await jail.set("__console_messages", new iv.ExternalCopy([]));

    await context.eval(`
      const __msgs = [];
      const console = {
        log:   (...a) => __msgs.push({ level: "log",   message: a.map(String).join(" "), ts: Date.now() }),
        warn:  (...a) => __msgs.push({ level: "warn",  message: a.map(String).join(" "), ts: Date.now() }),
        error: (...a) => __msgs.push({ level: "error", message: a.map(String).join(" "), ts: Date.now() }),
        debug: (...a) => __msgs.push({ level: "debug", message: a.map(String).join(" "), ts: Date.now() }),
      };
    `);

    // --- Inject input ---
    if (req.input !== undefined) {
      const inputCopy = new iv.ExternalCopy(req.input);
      await jail.set("__input_copy", inputCopy);
      await context.eval("const input = __input_copy.copy(); globalThis.__sandcastle_input = input;");
      inputCopy.release();
    } else {
      await context.eval("const input = undefined; globalThis.__sandcastle_input = undefined;");
    }

    // --- Build user code wrapper ---
    // Wrap user code in a function that captures the return value
    const wrappedCode = `
      (function() {
        const __startTs = Date.now();
        try {
          const __result = (function() { ${req.code} })();
          const __consoleMsgs = __msgs.map(m => ({
            ...m,
            ts: m.ts - __startTs
          }));
          return JSON.stringify({
            ok: true,
            value: __result,
            console: __consoleMsgs
          });
        } catch (e) {
          const __consoleMsgs = __msgs.map(m => ({
            ...m,
            ts: m.ts - __startTs
          }));
          return JSON.stringify({
            ok: false,
            error: e instanceof Error ? e.message : String(e),
            stack: e instanceof Error ? e.stack : undefined,
            console: __consoleMsgs
          });
        }
      })()
    `;

    // --- Execute with timeout ---
    let rawResult: string;
    try {
      const result = await context.eval(wrappedCode, {
        timeout: limits.timeoutMs,
      });
      rawResult = String(result);
    } catch (e: unknown) {
      // Timeout or memory errors from isolated-vm
      const message = e instanceof Error ? e.message : String(e);

      // Try to extract console messages even on crash
      try {
        const msgs = await context.eval("JSON.stringify(__msgs)");
        const parsed = JSON.parse(String(msgs));
        for (const m of parsed) {
          consoleMessages.push({ level: m.level, message: m.message, ts: m.ts });
        }
      } catch { /* ignore */ }

      const status = classifyError(message);
      context.release();
      return buildResult(
        status,
        { type: "null" },
        executionId,
        startedAt,
        startTime,
        consoleMessages,
        limits,
        getHeapUsed(isolate),
      );
    }

    // --- Parse result ---
    let parsed: {
      ok: boolean;
      value?: unknown;
      error?: string;
      stack?: string;
      console?: Array<{ level: string; message: string; ts: number }>;
    };
    try {
      parsed = JSON.parse(rawResult);
    } catch {
      parsed = { ok: true, value: rawResult };
    }

    // Collect console messages
    if (parsed.console) {
      for (const m of parsed.console) {
        consoleMessages.push({
          level: m.level as ConsoleEntry["level"],
          message: m.message,
          ts: m.ts,
        });
      }
    }

    const heapUsed = getHeapUsed(isolate);
    context.release();

    if (parsed.ok) {
      const output = valueToOutput(parsed.value, limits.maxOutputBytes);
      return buildResult(
        { type: "success" },
        output,
        executionId,
        startedAt,
        startTime,
        consoleMessages,
        limits,
        heapUsed,
      );
    }

    // Guest error
    return buildResult(
      { type: "guest_error", message: parsed.error ?? "unknown error" },
      { type: "null" },
      executionId,
      startedAt,
      startTime,
      consoleMessages,
      limits,
      heapUsed,
    );
  } finally {
    if (!isolate.isDisposed) {
      isolate.dispose();
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    if (value.length > maxBytes) {
      return { type: "string", value: value.slice(0, maxBytes) };
    }
    return { type: "string", value };
  }

  // Everything else → JSON
  const output: OutputValue = { type: "json", value };

  // Check size limit
  try {
    const serialized = JSON.stringify(value);
    if (serialized.length > maxBytes) {
      return { type: "string", value: `[output truncated: ${serialized.length} bytes exceeds ${maxBytes} limit]` };
    }
  } catch {
    return { type: "string", value: String(value) };
  }

  return output;
}

function getHeapUsed(isolate: InstanceType<typeof import("isolated-vm").Isolate>): number {
  try {
    const stats = isolate.getHeapStatisticsSync();
    return stats.used_heap_size;
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
): ExecutionResult {
  const finishedAt = new Date().toISOString();
  const transcript: ExecutionTranscript = {
    executionId,
    startedAt,
    finishedAt,
    status,
    fuelConsumed: 0,
    fuelLimit: limits.fuel,
    peakMemoryBytes,
    memoryLimitBytes: limits.memoryMb * 1024 * 1024,
    output,
    console: consoleMessages,
    capabilityCalls: [],
  };

  return {
    ok: status.type === "success",
    status,
    output,
    transcript,
    outputArtifacts: [],
  };
}
