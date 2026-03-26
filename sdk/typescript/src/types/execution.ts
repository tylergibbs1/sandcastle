/**
 * Public request/response types for SandCastle executions.
 *
 * These types are part of the SDK's public API surface.
 * Treat changes as semver-visible.
 */

// ---------------------------------------------------------------------------
// Execution request
// ---------------------------------------------------------------------------

/** Options for a single sandbox execution. */
export interface ExecuteOptions {
  /** JavaScript source code to run inside the sandbox. */
  code: string;

  /**
   * JSON-serializable input made available to the guest as
   * `globalThis.__sandcastle_input` (or simply `input` in the wrapper).
   */
  input?: unknown;

  /** Resource limits for this execution. */
  limits?: ExecutionLimits;

  /** Files to mount read-only inside the sandbox. */
  artifacts?: InputArtifact[];

  /**
   * An `AbortSignal` that cancels the execution when triggered.
   * The sandbox is destroyed and the returned promise rejects with
   * `ExecutionAbortedError`.
   */
  signal?: AbortSignal;
}

/** Resource constraints for a single execution. */
export interface ExecutionLimits {
  /** Maximum memory in megabytes. @default 32 */
  memoryMb?: number;

  /** Wall-clock timeout in milliseconds. @default 10_000 */
  timeoutMs?: number;

  /**
   * Fuel units (instruction-count cap).
   * `0` means unlimited.
   * @default 1_000_000_000
   */
  fuel?: number;

  /** Maximum output payload size in bytes. @default 1_048_576 */
  maxOutputBytes?: number;
}

/** A file mounted read-only into the sandbox. */
export interface InputArtifact {
  /** Virtual path inside the sandbox (e.g. `"data.csv"`). */
  name: string;

  /** Raw file contents. */
  data: Uint8Array;
}

// ---------------------------------------------------------------------------
// Execution result
// ---------------------------------------------------------------------------

/** Full result of a sandbox execution. */
export interface ExecutionResult {
  /** Whether the execution completed successfully. */
  readonly ok: boolean;

  /** Discriminated status tag. */
  readonly status: ExecutionStatus;

  /** The value returned by the guest code. */
  readonly output: OutputValue;

  /** Structured execution transcript for debugging / replay. */
  readonly transcript: ExecutionTranscript;

  /** Files written by the guest to `/output/`. */
  readonly outputArtifacts: OutputArtifact[];
}

// ---------------------------------------------------------------------------
// Status (discriminated union)
// ---------------------------------------------------------------------------

export type ExecutionStatus =
  | { readonly type: "success" }
  | { readonly type: "timeout" }
  | { readonly type: "fuel_exhausted" }
  | { readonly type: "memory_exceeded" }
  | { readonly type: "cancelled" }
  | { readonly type: "guest_error"; readonly message: string }
  | { readonly type: "capability_error"; readonly message: string };

// ---------------------------------------------------------------------------
// Output (discriminated union)
// ---------------------------------------------------------------------------

export type OutputValue =
  | { readonly type: "json"; readonly value: unknown }
  | { readonly type: "string"; readonly value: string }
  | { readonly type: "bytes"; readonly value: Uint8Array }
  | { readonly type: "null" };

// ---------------------------------------------------------------------------
// Transcript
// ---------------------------------------------------------------------------

export interface ExecutionTranscript {
  readonly executionId: string;
  readonly startedAt: string;
  readonly finishedAt: string | null;
  readonly status: ExecutionStatus;
  readonly fuelConsumed: number;
  readonly fuelLimit: number;
  readonly peakMemoryBytes: number;
  readonly memoryLimitBytes: number;
  readonly output: OutputValue;
  readonly console: readonly ConsoleEntry[];
  readonly capabilityCalls: readonly CapabilityCallEntry[];
}

export interface ConsoleEntry {
  readonly level: "log" | "warn" | "error" | "debug";
  readonly message: string;
  /** Milliseconds since execution start. */
  readonly ts: number;
}

export interface CapabilityCallEntry {
  readonly capability: string;
  readonly method: string;
  readonly input: unknown;
  readonly output: unknown | undefined;
  readonly error: string | undefined;
  readonly durationMs: number;
  /** Milliseconds since execution start. */
  readonly ts: number;
}

// ---------------------------------------------------------------------------
// Artifacts
// ---------------------------------------------------------------------------

export interface OutputArtifact {
  readonly name: string;
  readonly data: Uint8Array;
}
