import { randomUUID } from "node:crypto";
import type {
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  ExecutionStatus,
  ExecutionTranscript,
  OutputValue,
} from "../types/execution.js";

/**
 * Execute code via the SandCastle HTTP server.
 */
export async function executeViaHttp(
  endpoint: string,
  req: ExecuteOptions,
  defaults?: ExecutionLimits,
): Promise<ExecutionResult> {
  const limits = { ...defaults, ...req.limits };
  const body = {
    code: req.code,
    input: req.input,
    limits:
      limits.timeoutMs || limits.memoryMb || limits.fuel
        ? {
            memory_mb: limits.memoryMb,
            timeout_ms: limits.timeoutMs,
            fuel: limits.fuel,
          }
        : undefined,
  };

  const res = await fetch(`${endpoint}/execute`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal: req.signal,
  });

  return parseHttpResult(res);
}

/**
 * Register a named script via the HTTP server.
 */
export async function registerViaHttp(
  endpoint: string,
  name: string,
  code: string,
  limits?: ExecutionLimits,
  namespacePath?: string,
): Promise<void> {
  const url = namespacePath
    ? `${endpoint}/namespaces/${namespacePath}/scripts`
    : `${endpoint}/scripts`;

  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      name,
      code,
      limits: limits
        ? {
            memory_mb: limits.memoryMb,
            timeout_ms: limits.timeoutMs,
            fuel: limits.fuel,
          }
        : undefined,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Failed to register script: ${body}`);
  }
}

/**
 * Dispatch to a named script via the HTTP server.
 */
export async function dispatchViaHttp(
  endpoint: string,
  name: string,
  input?: unknown,
  limits?: ExecutionLimits,
  namespacePath?: string,
): Promise<ExecutionResult> {
  const url = namespacePath
    ? `${endpoint}/namespaces/${namespacePath}/dispatch/${name}`
    : `${endpoint}/dispatch/${name}`;

  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      input,
      limits: limits
        ? {
            memory_mb: limits.memoryMb,
            timeout_ms: limits.timeoutMs,
            fuel: limits.fuel,
          }
        : undefined,
    }),
  });

  return parseHttpResult(res);
}

/**
 * Create a dispatch namespace via the HTTP server.
 */
export async function createNamespaceViaHttp(
  endpoint: string,
  name: string,
  config?: { maxScripts?: number; maxConcurrent?: number },
): Promise<void> {
  const res = await fetch(`${endpoint}/namespaces`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      name,
      max_scripts: config?.maxScripts,
      max_concurrent: config?.maxConcurrent,
    }),
  });

  if (!res.ok) {
    const body = await res.text();
    throw new Error(`Failed to create namespace: ${body}`);
  }
}

/**
 * Delete a dispatch namespace via the HTTP server.
 */
export async function deleteNamespaceViaHttp(endpoint: string, name: string): Promise<boolean> {
  const res = await fetch(`${endpoint}/namespaces/${name}`, {
    method: "DELETE",
  });
  return res.ok;
}

/**
 * List scripts (global or in a namespace) via the HTTP server.
 */
export async function listScriptsViaHttp(
  endpoint: string,
  namespacePath?: string,
): Promise<string[]> {
  const url = namespacePath
    ? `${endpoint}/namespaces/${namespacePath}/scripts`
    : `${endpoint}/scripts`;

  const res = await fetch(url);
  if (!res.ok) return [];
  const body = (await res.json()) as { names?: string[] };
  return body.names ?? [];
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

async function parseHttpResult(res: Response): Promise<ExecutionResult> {
  const raw = (await res.json()) as Record<string, unknown>;

  // The HTTP server returns the ExecutionResult directly
  if ("ok" in raw && typeof raw.ok === "boolean") {
    return raw as unknown as ExecutionResult;
  }

  // Or it returns the transcript format
  const status = parseStatus(raw.status);
  const output = parseOutput(raw.output);

  const transcript: ExecutionTranscript = {
    executionId: (raw.execution_id as string) ?? (raw.executionId as string) ?? randomUUID(),
    startedAt: (raw.started_at as string) ?? new Date().toISOString(),
    finishedAt: (raw.finished_at as string) ?? null,
    status,
    fuelConsumed: (raw.fuel_consumed as number) ?? 0,
    fuelLimit: (raw.fuel_limit as number) ?? 0,
    peakMemoryBytes: (raw.peak_memory_bytes as number) ?? 0,
    memoryLimitBytes: (raw.memory_limit_bytes as number) ?? 0,
    output,
    console: [],
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

function parseStatus(raw: unknown): ExecutionStatus {
  if (typeof raw === "object" && raw !== null && "type" in raw) {
    const obj = raw as Record<string, unknown>;
    const type = obj.type as string;
    if (type === "guest_error" || type === "capability_error") {
      return {
        type,
        message: String(obj.message ?? "unknown"),
      } as ExecutionStatus;
    }
    if (["success", "timeout", "fuel_exhausted", "memory_exceeded", "cancelled"].includes(type)) {
      return { type } as ExecutionStatus;
    }
  }
  return { type: "guest_error", message: "unknown status" };
}

function parseOutput(raw: unknown): OutputValue {
  if (raw === null || raw === undefined) return { type: "null" };
  if (typeof raw === "object" && raw !== null && "type" in raw) {
    const obj = raw as Record<string, unknown>;
    if (obj.type === "json") return { type: "json", value: obj.value };
    if (obj.type === "string") return { type: "string", value: String(obj.value) };
    if (obj.type === "null") return { type: "null" };
  }
  return { type: "json", value: raw };
}
