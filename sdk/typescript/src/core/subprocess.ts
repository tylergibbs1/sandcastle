import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import { unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { SandCastleOptions } from "../types/config.js";
import type {
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  ExecutionStatus,
  ExecutionTranscript,
  OutputValue,
} from "../types/execution.js";
import { BinaryNotFoundError } from "./errors.js";

const LIMIT_DEFAULTS: Required<ExecutionLimits> = {
  memoryMb: 32,
  timeoutMs: 10_000,
  fuel: 1_000_000_000,
  maxOutputBytes: 1_048_576,
};

/**
 * Execute code by spawning the `sandcastle` CLI as a child process.
 *
 * This is an internal function — the public API is `SandCastle.execute()`.
 */
export async function executeViaSubprocess(
  opts: SandCastleOptions,
  req: ExecuteOptions,
): Promise<ExecutionResult> {
  const binary = opts.binaryPath ?? "sandcastle";
  const limits: Required<ExecutionLimits> = {
    ...LIMIT_DEFAULTS,
    ...opts.defaults,
    ...req.limits,
  };

  const args = buildArgs(opts, limits, req);

  // Write artifacts to temp files
  const tmpFiles: string[] = [];
  if (req.artifacts?.length) {
    for (const art of req.artifacts) {
      const tmp = join(tmpdir(), `sandcastle-${randomUUID()}`);
      await writeFile(tmp, art.data);
      args.push("--artifact", `${art.name}=${tmp}`);
      tmpFiles.push(tmp);
    }
  }

  try {
    const proc = spawnCli(binary, args, req.code, req.signal);
    const { stdout, stderr, exitCode } = await proc;
    return parseOutput(stdout, stderr, exitCode);
  } finally {
    await Promise.all(tmpFiles.map((f) => unlink(f).catch(() => {})));
  }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

function buildArgs(
  opts: SandCastleOptions,
  limits: Required<ExecutionLimits>,
  req: ExecuteOptions,
): string[] {
  const args = [
    "run",
    "/dev/stdin",
    "--transcript",
    "--memory-mb",
    String(limits.memoryMb),
    "--timeout",
    String(Math.ceil(limits.timeoutMs / 1000)),
    "--fuel",
    String(limits.fuel),
  ];

  if (opts.guestModule) {
    args.push("--guest-module", opts.guestModule);
  }

  if (req.input !== undefined) {
    args.push("--input", JSON.stringify(req.input));
  }

  return args;
}

function spawnCli(
  binary: string,
  args: string[],
  stdin: string,
  signal?: AbortSignal,
): Promise<{ stdout: string; stderr: string; exitCode: number }> {
  return new Promise((resolve, reject) => {
    const proc = spawn(binary, args, { stdio: ["pipe", "pipe", "pipe"] });

    // AbortSignal support
    if (signal) {
      const onAbort = () => {
        proc.kill("SIGTERM");
      };
      signal.addEventListener("abort", onAbort, { once: true });
      proc.on("close", () => signal.removeEventListener("abort", onAbort));
    }

    let stdout = "";
    let stderr = "";

    proc.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });
    proc.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });

    proc.on("error", (err) => {
      if ((err as NodeJS.ErrnoException).code === "ENOENT") {
        reject(new BinaryNotFoundError(binary));
      } else {
        reject(err);
      }
    });

    proc.on("close", (code) => {
      resolve({ stdout, stderr, exitCode: code ?? 1 });
    });

    proc.stdin.write(stdin);
    proc.stdin.end();
  });
}

function parseOutput(stdout: string, stderr: string, exitCode: number): ExecutionResult {
  // The CLI may emit tracing logs before the JSON transcript.
  // Find the first '{' that starts a valid JSON object.
  const jsonStart = stdout.indexOf("{");
  if (jsonStart >= 0) {
    const jsonStr = stdout.slice(jsonStart);
    try {
      const raw = JSON.parse(jsonStr) as RawTranscript;
      return fromRawTranscript(raw);
    } catch {
      /* fall through to stderr check */
    }
  }

  // Some CLI modes output to stderr with transcript on stdout.
  const combined = stderr + stdout;
  const stderrJsonStart = combined.indexOf("{");
  if (stderrJsonStart >= 0) {
    try {
      const raw = JSON.parse(combined.slice(stderrJsonStart)) as RawTranscript;
      return fromRawTranscript(raw);
    } catch {
      /* fall through */
    }
  }

  return fallbackResult(stdout, stderr, exitCode);
}

// ---------------------------------------------------------------------------
// Raw transcript → public types
// ---------------------------------------------------------------------------

function fromRawTranscript(raw: RawTranscript): ExecutionResult {
  const status = parseStatus(raw.status);
  const output = parseOutput2(raw.output);

  const transcript: ExecutionTranscript = {
    executionId: raw.execution_id,
    startedAt: raw.started_at,
    finishedAt: raw.finished_at ?? null,
    status,
    fuelConsumed: raw.fuel_consumed,
    fuelLimit: raw.fuel_limit,
    peakMemoryBytes: raw.peak_memory_bytes,
    memoryLimitBytes: raw.memory_limit_bytes,
    output,
    console: (raw.console ?? []).map((c) => ({
      level: c.level,
      message: c.message,
      ts: c.ts,
    })),
    capabilityCalls: (raw.capability_calls ?? []).map((c) => ({
      capability: c.capability,
      method: c.method,
      input: c.input,
      output: c.output,
      error: c.error,
      durationMs: c.duration_ms,
      ts: c.ts,
    })),
  };

  return {
    ok: status.type === "success",
    status,
    output,
    transcript,
    outputArtifacts: [],
  };
}

function parseStatus(raw: unknown): ExecutionStatus {
  if (typeof raw === "object" && raw !== null && "type" in raw) {
    const obj = raw as Record<string, unknown>;
    const type = obj.type as string;
    if (type === "guest_error" || type === "capability_error") {
      return { type, message: String(obj.message ?? "unknown") } as ExecutionStatus;
    }
    if (
      type === "success" ||
      type === "timeout" ||
      type === "fuel_exhausted" ||
      type === "memory_exceeded" ||
      type === "cancelled"
    ) {
      return { type } as ExecutionStatus;
    }
  }
  if (typeof raw === "string") {
    return raw === "success" ? { type: "success" } : { type: "guest_error", message: raw };
  }
  return { type: "guest_error", message: "unknown status" };
}

function parseOutput2(raw: unknown): OutputValue {
  if (raw === null || raw === undefined) return { type: "null" };
  if (typeof raw === "object" && raw !== null && "type" in raw) {
    const obj = raw as Record<string, unknown>;
    if (obj.type === "json") return { type: "json", value: obj.value };
    if (obj.type === "string") return { type: "string", value: String(obj.value) };
    if (obj.type === "null") return { type: "null" };
  }
  return { type: "json", value: raw };
}

function fallbackResult(stdout: string, stderr: string, exitCode: number): ExecutionResult {
  const status: ExecutionStatus =
    exitCode === 0
      ? { type: "success" }
      : { type: "guest_error", message: stderr || `exit code ${exitCode}` };

  let output: OutputValue;
  try {
    output = { type: "json", value: JSON.parse(stdout) };
  } catch {
    output = stdout.trim() ? { type: "string", value: stdout.trim() } : { type: "null" };
  }

  const now = new Date().toISOString();
  return {
    ok: status.type === "success",
    status,
    output,
    transcript: {
      executionId: randomUUID(),
      startedAt: now,
      finishedAt: now,
      status,
      fuelConsumed: 0,
      fuelLimit: 0,
      peakMemoryBytes: 0,
      memoryLimitBytes: 0,
      output,
      console: [],
      capabilityCalls: [],
    },
    outputArtifacts: [],
  };
}

// The raw JSON shape emitted by the CLI (snake_case).
interface RawTranscript {
  execution_id: string;
  started_at: string;
  finished_at?: string;
  status: unknown;
  fuel_consumed: number;
  fuel_limit: number;
  peak_memory_bytes: number;
  memory_limit_bytes: number;
  output: unknown;
  console?: {
    level: "log" | "warn" | "error" | "debug";
    message: string;
    ts: number;
  }[];
  capability_calls?: {
    capability: string;
    method: string;
    input: unknown;
    output?: unknown;
    error?: string;
    duration_ms: number;
    ts: number;
  }[];
}
