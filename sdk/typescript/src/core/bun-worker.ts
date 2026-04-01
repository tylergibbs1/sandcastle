/**
 * Bun Worker sandbox executor.
 *
 * Runs user code in a Bun Worker thread for true isolation:
 * - Separate JavaScriptCore context (no shared globals)
 * - Timeout via worker.terminate()
 * - Zero native dependencies (no isolated-vm needed)
 * - Works with Bun's native Worker API
 *
 * This is the Bun-first executor — faster startup than isolated-vm
 * and zero C++ compilation on install.
 */

// @ts-nocheck — Bun-specific APIs

declare const Bun: {
  isMainThread: boolean;
};

declare const self: Worker;

const IS_WORKER = typeof self !== "undefined" && typeof self.postMessage === "function"
  && typeof Bun !== "undefined" && !Bun.isMainThread;

if (IS_WORKER) {
  // --- Worker thread: receive code, execute, send result ---
  self.onmessage = (event: MessageEvent) => {
    const { code, input, globals, id } = event.data;
    const consoleMsgs: Array<{ level: string; message: string; ts: number }> = [];
    const startTime = performance.now();

    // Set up console capture
    const console = {
      log: (...a: unknown[]) => consoleMsgs.push({ level: "log", message: a.map(String).join(" "), ts: Math.round(performance.now() - startTime) }),
      warn: (...a: unknown[]) => consoleMsgs.push({ level: "warn", message: a.map(String).join(" "), ts: Math.round(performance.now() - startTime) }),
      error: (...a: unknown[]) => consoleMsgs.push({ level: "error", message: a.map(String).join(" "), ts: Math.round(performance.now() - startTime) }),
      debug: (...a: unknown[]) => consoleMsgs.push({ level: "debug", message: a.map(String).join(" "), ts: Math.round(performance.now() - startTime) }),
      time: (label = "default") => { (console as any).__timers = (console as any).__timers ?? {}; (console as any).__timers[label] = performance.now(); },
      timeEnd: (label = "default") => {
        const t = (console as any).__timers?.[label];
        if (t !== undefined) { delete (console as any).__timers[label]; consoleMsgs.push({ level: "log", message: `${label}: ${(performance.now() - t).toFixed(2)}ms`, ts: Math.round(performance.now() - startTime) }); }
      },
      timeLog: (label = "default") => {
        const t = (console as any).__timers?.[label];
        if (t !== undefined) { consoleMsgs.push({ level: "log", message: `${label}: ${(performance.now() - t).toFixed(2)}ms`, ts: Math.round(performance.now() - startTime) }); }
      },
    };

    // Build scope with globals
    const scope: Record<string, unknown> = { console, input };
    if (globals) Object.assign(scope, globals);

    // Build function args
    const argNames = Object.keys(scope);
    const argValues = Object.values(scope);

    try {
      // Use Function constructor to run code in clean scope
      const fn = new Function(...argNames, code);
      const result = fn(...argValues);

      // Handle promises
      if (result && typeof result === "object" && typeof result.then === "function") {
        result.then(
          (value: unknown) => self.postMessage({ id, ok: true, value, console: consoleMsgs }),
          (err: Error) => self.postMessage({ id, ok: false, error: err?.message ?? String(err), stack: err?.stack, console: consoleMsgs }),
        );
      } else {
        self.postMessage({ id, ok: true, value: result, console: consoleMsgs });
      }
    } catch (e: any) {
      self.postMessage({ id, ok: false, error: e?.message ?? String(e), stack: e?.stack, console: consoleMsgs });
    }
  };
}

// --- Main thread exports ---

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
import type { HostFunction, OnConsoleCallback } from "../types/config.js";

const LIMIT_DEFAULTS: Required<ExecutionLimits> = {
  memoryMb: 128,
  timeoutMs: 10_000,
  fuel: 0,
  maxOutputBytes: 1_048_576,
};

// ---------------------------------------------------------------------------
// Worker Pool
// ---------------------------------------------------------------------------

interface PooledWorker {
  worker: Worker;
  busy: boolean;
}

export class BunWorkerPool {
  private workers: PooledWorker[] = [];
  private readonly maxSize: number;
  private readonly workerUrl: string;

  constructor(maxSize = 4) {
    this.maxSize = maxSize;
    // Use import.meta.url to resolve the worker file
    this.workerUrl = import.meta.url;
  }

  acquire(): PooledWorker {
    // Find a free worker
    const free = this.workers.find(w => !w.busy);
    if (free) {
      free.busy = true;
      return free;
    }

    // Create new if under limit
    if (this.workers.length < this.maxSize) {
      const entry: PooledWorker = {
        worker: new Worker(this.workerUrl),
        busy: true,
      };
      this.workers.push(entry);
      return entry;
    }

    // All busy, create a temporary one (not pooled)
    return {
      worker: new Worker(this.workerUrl),
      busy: true,
    };
  }

  release(entry: PooledWorker) {
    entry.busy = false;
    // If it's not in our pool (overflow), terminate it
    if (!this.workers.includes(entry)) {
      entry.worker.terminate();
    }
  }

  dispose() {
    for (const entry of this.workers) {
      entry.worker.terminate();
    }
    this.workers = [];
  }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

export async function executeViaBunWorker(
  req: ExecuteOptions,
  defaults?: ExecutionLimits,
  hostFunctions?: Record<string, HostFunction>,
  onConsole?: OnConsoleCallback,
  pool?: BunWorkerPool | null,
): Promise<ExecutionResult> {
  const limits: Required<ExecutionLimits> = {
    ...LIMIT_DEFAULTS,
    ...defaults,
    ...req.limits,
  };

  const executionId = randomUUID();
  const startedAt = new Date().toISOString();
  const startTime = performance.now();

  // Cancelled
  if (req.signal?.aborted) {
    return buildResult(
      { type: "cancelled" }, { type: "null" },
      executionId, startedAt, startTime, [], limits,
    );
  }

  // Prepare globals (merge host functions as values — they'll be serialized)
  // Note: host functions can't cross Worker boundary as closures.
  // For Bun worker mode, hostFunctions must be pure data transforms.
  const globals = { ...(req.globals ?? {}) };

  // Create or acquire worker
  let poolEntry: PooledWorker | null = null;
  let worker: Worker;

  if (pool) {
    poolEntry = pool.acquire();
    worker = poolEntry.worker;
  } else {
    worker = new Worker(import.meta.url);
  }

  const id = executionId;

  try {
    const result = await new Promise<{
      ok: boolean;
      value?: unknown;
      error?: string;
      stack?: string;
      console?: Array<{ level: string; message: string; ts: number }>;
    }>((resolve, reject) => {
      const timeout = setTimeout(() => {
        worker.terminate();
        resolve({
          ok: false,
          error: "Script execution timed out",
          console: [],
        });
      }, limits.timeoutMs);

      worker.onmessage = (event: MessageEvent) => {
        if (event.data.id === id) {
          clearTimeout(timeout);
          resolve(event.data);
        }
      };

      worker.onerror = (err: ErrorEvent) => {
        clearTimeout(timeout);
        resolve({ ok: false, error: err.message ?? "Worker error", console: [] });
      };

      worker.postMessage({ code: req.code, input: req.input, globals, id });
    });

    // Collect console
    const consoleMessages: ConsoleEntry[] = (result.console ?? []).map(m => ({
      level: m.level as ConsoleEntry["level"],
      message: m.message,
      ts: m.ts,
    }));

    // Stream console to callback
    if (onConsole) {
      for (const msg of consoleMessages) {
        onConsole(msg.level, msg.message, msg.ts);
      }
    }

    if (result.ok) {
      const output = valueToOutput(result.value, limits.maxOutputBytes);
      return buildResult({ type: "success" }, output, executionId, startedAt, startTime, consoleMessages, limits);
    }

    const status = classifyError(result.error ?? "unknown error");
    return buildResult(status, { type: "null" }, executionId, startedAt, startTime, consoleMessages, limits);
  } finally {
    if (poolEntry && pool) {
      pool.release(poolEntry);
    } else {
      worker.terminate();
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function classifyError(message: string): ExecutionStatus {
  const lower = message.toLowerCase();
  if (lower.includes("timed out") || lower.includes("timeout")) return { type: "timeout" };
  if (lower.includes("memory") || lower.includes("allocation")) return { type: "memory_exceeded" };
  return { type: "guest_error", message };
}

function valueToOutput(value: unknown, maxBytes: number): OutputValue {
  if (value === null || value === undefined) return { type: "null" };
  if (typeof value === "string") {
    return value.length > maxBytes ? { type: "string", value: value.slice(0, maxBytes) } : { type: "string", value };
  }
  try {
    const s = JSON.stringify(value);
    if (s.length > maxBytes) return { type: "string", value: `[output truncated: ${s.length} bytes > ${maxBytes} limit]` };
  } catch { return { type: "string", value: String(value) }; }
  return { type: "json", value };
}

function buildResult(
  status: ExecutionStatus, output: OutputValue,
  executionId: string, startedAt: string, startTime: number,
  consoleMessages: ConsoleEntry[], limits: Required<ExecutionLimits>,
): ExecutionResult {
  const ms = Math.round(performance.now() - startTime);
  const finishedAt = new Date().toISOString();
  const value = output.type === "json" ? output.value : output.type === "string" ? output.value : undefined;
  const transcript: ExecutionTranscript = {
    executionId, startedAt, finishedAt, status,
    fuelConsumed: 0, fuelLimit: limits.fuel,
    peakMemoryBytes: 0, memoryLimitBytes: limits.memoryMb * 1024 * 1024,
    output, console: consoleMessages, capabilityCalls: [],
  };
  return { ok: status.type === "success", status, output, transcript, outputArtifacts: [], logs: consoleMessages, ms, value, memoryBytes: 0 };
}
