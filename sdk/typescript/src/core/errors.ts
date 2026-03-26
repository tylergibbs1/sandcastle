import type { ExecutionResult, ExecutionStatus } from "../types/execution.js";

/**
 * Base class for all SandCastle errors.
 * Consumers can catch this to handle any SDK-level failure.
 */
export class SandCastleError extends Error {
  constructor(message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "SandCastleError";
  }
}

/**
 * The sandbox executed code but it did not succeed.
 *
 * Carries the full `ExecutionResult` so callers can inspect the transcript,
 * console output, and status for diagnostics.
 */
export class ExecutionFailedError extends SandCastleError {
  /** The discriminated status that caused the failure. */
  readonly status: ExecutionStatus;

  /** The full execution result including transcript. */
  readonly result: ExecutionResult;

  constructor(result: ExecutionResult) {
    const msg =
      "message" in result.status ? result.status.message : `execution ${result.status.type}`;
    super(msg);
    this.name = "ExecutionFailedError";
    this.status = result.status;
    this.result = result;
  }
}

/** A guest timeout specifically — subclass for convenient narrowing. */
export class TimeoutError extends ExecutionFailedError {
  constructor(result: ExecutionResult) {
    super(result);
    this.name = "TimeoutError";
  }
}

/** Fuel / instruction budget exhausted. */
export class FuelExhaustedError extends ExecutionFailedError {
  constructor(result: ExecutionResult) {
    super(result);
    this.name = "FuelExhaustedError";
  }
}

/** Memory limit exceeded. */
export class MemoryExceededError extends ExecutionFailedError {
  constructor(result: ExecutionResult) {
    super(result);
    this.name = "MemoryExceededError";
  }
}

/** The guest code threw or returned an error. */
export class GuestError extends ExecutionFailedError {
  constructor(result: ExecutionResult) {
    super(result);
    this.name = "GuestError";
  }
}

/** Execution was cancelled via AbortSignal. */
export class ExecutionAbortedError extends SandCastleError {
  constructor() {
    super("execution aborted");
    this.name = "ExecutionAbortedError";
  }
}

/** The SandCastle binary was not found on the system. */
export class BinaryNotFoundError extends SandCastleError {
  readonly binaryPath: string;

  constructor(binaryPath: string) {
    super(
      `SandCastle binary not found at "${binaryPath}". ` +
        "Install it or pass binaryPath in SandCastleOptions.",
    );
    this.name = "BinaryNotFoundError";
    this.binaryPath = binaryPath;
  }
}

/**
 * Build the right error subclass from an `ExecutionResult`.
 * Returns `null` when the result is a success.
 */
export function errorFromResult(result: ExecutionResult): ExecutionFailedError | null {
  switch (result.status.type) {
    case "success":
      return null;
    case "timeout":
      return new TimeoutError(result);
    case "fuel_exhausted":
      return new FuelExhaustedError(result);
    case "memory_exceeded":
      return new MemoryExceededError(result);
    case "guest_error":
      return new GuestError(result);
    default:
      return new ExecutionFailedError(result);
  }
}
