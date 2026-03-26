import { describe, expect, it } from "bun:test";
import { BinaryNotFoundError, ExecutionAbortedError, SandCastle } from "../src/index.js";

describe("SandCastle constructor", () => {
  it("accepts empty options", () => {
    expect(() => new SandCastle()).not.toThrow();
  });

  it("accepts full options", () => {
    const sc = new SandCastle({
      binaryPath: "/usr/local/bin/sandcastle",
      guestModule: "/path/to/guest.wasm",
      defaults: {
        memoryMb: 128,
        timeoutMs: 60_000,
        fuel: 0,
        maxOutputBytes: 10_000_000,
      },
    });
    expect(sc).toBeInstanceOf(SandCastle);
  });
});

describe("execute() pre-flight checks", () => {
  it("throws ExecutionAbortedError for already-aborted signal", async () => {
    const sc = new SandCastle({ binaryPath: "/nonexistent" });
    const controller = new AbortController();
    controller.abort();

    try {
      await sc.execute({ code: "return 1;", signal: controller.signal });
      expect(true).toBe(false);
    } catch (err) {
      expect(err).toBeInstanceOf(ExecutionAbortedError);
    }
  });

  it("throws BinaryNotFoundError for missing binary", async () => {
    const sc = new SandCastle({ binaryPath: "/no/such/binary" });
    try {
      await sc.execute({ code: "return 1;" });
      expect(true).toBe(false);
    } catch (err) {
      expect(err).toBeInstanceOf(BinaryNotFoundError);
    }
  });
});

describe("run() pre-flight checks", () => {
  it("throws BinaryNotFoundError for missing binary", async () => {
    const sc = new SandCastle({ binaryPath: "/no/such/binary" });
    try {
      await sc.run("return 1;");
      expect(true).toBe(false);
    } catch (err) {
      expect(err).toBeInstanceOf(BinaryNotFoundError);
    }
  });
});

describe("diagnoseInstallation()", () => {
  it("returns actionable diagnostics for a missing binary", async () => {
    const sc = new SandCastle({ binaryPath: "/no/such/binary" });
    const result = await sc.diagnoseInstallation();
    expect(result.ok).toBe(false);
    expect(result.binaryPath).toBe("/no/such/binary");
    expect(result.nextSteps.length).toBeGreaterThan(0);
  });
});
