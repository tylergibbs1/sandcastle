import { describe, expect, it } from "bun:test";
import {
  BinaryNotFoundError,
  ExecutionAbortedError,
  ExecutionFailedError,
  errorFromResult,
  FuelExhaustedError,
  GuestError,
  MemoryExceededError,
  SandCastleError,
  TimeoutError,
} from "../src/core/errors.js";
import type { ExecutionResult } from "../src/types/execution.js";

// Shared fixture
function makeResult(overrides: Partial<ExecutionResult> = {}): ExecutionResult {
  return {
    ok: false,
    status: { type: "timeout" },
    output: { type: "null" },
    transcript: {
      executionId: "test-id",
      startedAt: "2026-01-01T00:00:00Z",
      finishedAt: "2026-01-01T00:00:01Z",
      status: { type: "timeout" },
      fuelConsumed: 500,
      fuelLimit: 1000,
      peakMemoryBytes: 4096,
      memoryLimitBytes: 33554432,
      output: { type: "null" },
      console: [],
      capabilityCalls: [],
    },
    outputArtifacts: [],
    ...overrides,
  };
}

// -----------------------------------------------------------------------
// Hierarchy
// -----------------------------------------------------------------------

describe("error hierarchy", () => {
  it("all errors extend SandCastleError", () => {
    const r = makeResult();
    for (const Cls of [
      TimeoutError,
      FuelExhaustedError,
      MemoryExceededError,
      GuestError,
      ExecutionFailedError,
    ]) {
      expect(new Cls(r)).toBeInstanceOf(SandCastleError);
      expect(new Cls(r)).toBeInstanceOf(Error);
    }
    expect(new ExecutionAbortedError()).toBeInstanceOf(SandCastleError);
    expect(new BinaryNotFoundError("/x")).toBeInstanceOf(SandCastleError);
  });

  it("TimeoutError extends ExecutionFailedError", () => {
    const err = new TimeoutError(makeResult());
    expect(err).toBeInstanceOf(ExecutionFailedError);
  });

  it("FuelExhaustedError extends ExecutionFailedError", () => {
    const err = new FuelExhaustedError(makeResult());
    expect(err).toBeInstanceOf(ExecutionFailedError);
  });

  it("MemoryExceededError extends ExecutionFailedError", () => {
    const err = new MemoryExceededError(makeResult());
    expect(err).toBeInstanceOf(ExecutionFailedError);
  });

  it("GuestError extends ExecutionFailedError", () => {
    const r = makeResult({
      status: { type: "guest_error", message: "boom" },
    });
    const err = new GuestError(r);
    expect(err).toBeInstanceOf(ExecutionFailedError);
  });

  it("ExecutionAbortedError does NOT extend ExecutionFailedError", () => {
    const err = new ExecutionAbortedError();
    expect(err).not.toBeInstanceOf(ExecutionFailedError);
    expect(err).toBeInstanceOf(SandCastleError);
  });

  it("BinaryNotFoundError does NOT extend ExecutionFailedError", () => {
    const err = new BinaryNotFoundError("/bin/sandcastle");
    expect(err).not.toBeInstanceOf(ExecutionFailedError);
  });
});

// -----------------------------------------------------------------------
// .name property
// -----------------------------------------------------------------------

describe("error names", () => {
  it.each([
    ["SandCastleError", SandCastleError, "test"],
    ["ExecutionAbortedError", ExecutionAbortedError, undefined],
    ["BinaryNotFoundError", BinaryNotFoundError, "/x"],
  ] as const)("%s has correct .name", (expected, Cls, arg) => {
    const err = arg !== undefined ? new (Cls as any)(arg) : new (Cls as any)();
    expect(err.name).toBe(expected);
  });

  it.each([
    ["TimeoutError", TimeoutError],
    ["FuelExhaustedError", FuelExhaustedError],
    ["MemoryExceededError", MemoryExceededError],
    ["GuestError", GuestError],
    ["ExecutionFailedError", ExecutionFailedError],
  ] as const)("%s has correct .name", (expected, Cls) => {
    expect(new Cls(makeResult()).name).toBe(expected);
  });
});

// -----------------------------------------------------------------------
// Properties
// -----------------------------------------------------------------------

describe("error properties", () => {
  it("ExecutionFailedError exposes .status and .result", () => {
    const r = makeResult({
      status: { type: "guest_error", message: "oops" },
    });
    const err = new ExecutionFailedError(r);
    expect(err.status).toEqual(r.status);
    expect(err.result).toBe(r);
    expect(err.message).toBe("oops");
  });

  it("ExecutionFailedError uses type as message when no message field", () => {
    const r = makeResult({ status: { type: "timeout" } });
    const err = new ExecutionFailedError(r);
    expect(err.message).toBe("execution timeout");
  });

  it("BinaryNotFoundError exposes .binaryPath", () => {
    const err = new BinaryNotFoundError("/opt/sandcastle");
    expect(err.binaryPath).toBe("/opt/sandcastle");
    expect(err.message).toContain("/opt/sandcastle");
  });

  it("ExecutionAbortedError has fixed message", () => {
    expect(new ExecutionAbortedError().message).toBe("execution aborted");
  });
});

// -----------------------------------------------------------------------
// errorFromResult
// -----------------------------------------------------------------------

describe("errorFromResult", () => {
  it("returns null for success", () => {
    const r = makeResult({ ok: true, status: { type: "success" } });
    expect(errorFromResult(r)).toBeNull();
  });

  it("returns TimeoutError for timeout", () => {
    const r = makeResult({ status: { type: "timeout" } });
    expect(errorFromResult(r)).toBeInstanceOf(TimeoutError);
  });

  it("returns FuelExhaustedError for fuel_exhausted", () => {
    const r = makeResult({ status: { type: "fuel_exhausted" } });
    expect(errorFromResult(r)).toBeInstanceOf(FuelExhaustedError);
  });

  it("returns MemoryExceededError for memory_exceeded", () => {
    const r = makeResult({ status: { type: "memory_exceeded" } });
    expect(errorFromResult(r)).toBeInstanceOf(MemoryExceededError);
  });

  it("returns GuestError for guest_error", () => {
    const r = makeResult({
      status: { type: "guest_error", message: "ReferenceError" },
    });
    const err = errorFromResult(r);
    expect(err).toBeInstanceOf(GuestError);
    expect(err!.message).toBe("ReferenceError");
  });

  it("returns ExecutionFailedError for capability_error", () => {
    const r = makeResult({
      status: { type: "capability_error", message: "quota" },
    });
    const err = errorFromResult(r);
    expect(err).toBeInstanceOf(ExecutionFailedError);
    expect(err!.message).toBe("quota");
  });

  it("returns ExecutionFailedError for cancelled", () => {
    const r = makeResult({ status: { type: "cancelled" } });
    expect(errorFromResult(r)).toBeInstanceOf(ExecutionFailedError);
  });

  it("every returned error carries the original result", () => {
    for (const type of ["timeout", "fuel_exhausted", "memory_exceeded", "cancelled"] as const) {
      const r = makeResult({ status: { type } });
      const err = errorFromResult(r)!;
      expect(err.result).toBe(r);
    }
  });
});
