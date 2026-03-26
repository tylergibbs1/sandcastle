import { describe, expect, it } from "bun:test";
import type { ExecutionResult, ExecutionStatus, OutputValue } from "../src/index.js";
import { jsonArtifact, SandCastle, textArtifact } from "../src/index.js";

// -----------------------------------------------------------------------
// Public API surface
// -----------------------------------------------------------------------

describe("public exports", () => {
  it("SandCastle is constructable with no args", () => {
    const sc = new SandCastle();
    expect(sc).toBeInstanceOf(SandCastle);
  });

  it("SandCastle is constructable with options", () => {
    const sc = new SandCastle({
      binaryPath: "/usr/bin/sandcastle",
      guestModule: "/path/to/guest.wasm",
      defaults: { memoryMb: 64, timeoutMs: 30_000 },
    });
    expect(sc).toBeInstanceOf(SandCastle);
  });
});

// -----------------------------------------------------------------------
// Discriminated union narrowing (compile-time + runtime)
// -----------------------------------------------------------------------

describe("ExecutionStatus discriminated union", () => {
  it("narrows success (no message field)", () => {
    const s: ExecutionStatus = { type: "success" };
    if (s.type === "success") {
      // Compile-time: s has no 'message' property
      expect(s.type).toBe("success");
    }
  });

  it("narrows guest_error (has message field)", () => {
    const s: ExecutionStatus = { type: "guest_error", message: "oops" };
    if (s.type === "guest_error") {
      expect(s.message).toBe("oops");
    }
  });

  it("narrows capability_error (has message field)", () => {
    const s: ExecutionStatus = {
      type: "capability_error",
      message: "rate limit",
    };
    if (s.type === "capability_error") {
      expect(s.message).toBe("rate limit");
    }
  });

  it("exhaustive switch compiles", () => {
    // Use a function to avoid TS narrowing the literal type
    function getLabel(status: ExecutionStatus): string {
      switch (status.type) {
        case "success":
          return "ok";
        case "timeout":
          return "timed out";
        case "fuel_exhausted":
          return "fuel";
        case "memory_exceeded":
          return "oom";
        case "cancelled":
          return "cancelled";
        case "guest_error":
          return status.message;
        case "capability_error":
          return status.message;
      }
    }
    expect(getLabel({ type: "timeout" })).toBe("timed out");
    expect(getLabel({ type: "guest_error", message: "oops" })).toBe("oops");
  });
});

describe("OutputValue discriminated union", () => {
  it("narrows json", () => {
    const o: OutputValue = { type: "json", value: [1, 2, 3] };
    if (o.type === "json") {
      expect(o.value).toEqual([1, 2, 3]);
    }
  });

  it("narrows string", () => {
    const o: OutputValue = { type: "string", value: "hello" };
    if (o.type === "string") {
      expect(o.value).toBe("hello");
    }
  });

  it("narrows bytes", () => {
    const o: OutputValue = {
      type: "bytes",
      value: new Uint8Array([1, 2, 3]),
    };
    if (o.type === "bytes") {
      expect(o.value).toBeInstanceOf(Uint8Array);
      expect(o.value.length).toBe(3);
    }
  });

  it("narrows null", () => {
    const o: OutputValue = { type: "null" };
    expect(o.type).toBe("null");
  });
});

// -----------------------------------------------------------------------
// Artifact helpers
// -----------------------------------------------------------------------

describe("textArtifact", () => {
  it("creates an artifact with UTF-8 encoded data", () => {
    const art = textArtifact("readme.txt", "Hello, World!");
    expect(art.name).toBe("readme.txt");
    expect(art.data).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(art.data)).toBe("Hello, World!");
  });

  it("handles empty string", () => {
    const art = textArtifact("empty.txt", "");
    expect(art.data.length).toBe(0);
  });

  it("handles unicode content", () => {
    const art = textArtifact("unicode.txt", "Hello \u{1F680} World");
    const decoded = new TextDecoder().decode(art.data);
    expect(decoded).toBe("Hello \u{1F680} World");
  });

  it("handles multi-line content", () => {
    const csv = "name,age\nAlice,30\nBob,25";
    const art = textArtifact("data.csv", csv);
    expect(new TextDecoder().decode(art.data)).toBe(csv);
  });
});

describe("jsonArtifact", () => {
  it("serializes objects", () => {
    const art = jsonArtifact("config.json", { key: "value", n: 42 });
    const parsed = JSON.parse(new TextDecoder().decode(art.data));
    expect(parsed).toEqual({ key: "value", n: 42 });
  });

  it("serializes arrays", () => {
    const art = jsonArtifact("list.json", [1, 2, 3]);
    const parsed = JSON.parse(new TextDecoder().decode(art.data));
    expect(parsed).toEqual([1, 2, 3]);
  });

  it("serializes null", () => {
    const art = jsonArtifact("null.json", null);
    const parsed = JSON.parse(new TextDecoder().decode(art.data));
    expect(parsed).toBeNull();
  });

  it("serializes nested structures", () => {
    const data = {
      users: [
        { id: 1, tags: ["admin"] },
        { id: 2, tags: [] },
      ],
    };
    const art = jsonArtifact("nested.json", data);
    const parsed = JSON.parse(new TextDecoder().decode(art.data));
    expect(parsed).toEqual(data);
  });
});

// -----------------------------------------------------------------------
// ExecutionResult shape
// -----------------------------------------------------------------------

describe("ExecutionResult", () => {
  it("ok is true when status is success", () => {
    const r: ExecutionResult = {
      ok: true,
      status: { type: "success" },
      output: { type: "json", value: 42 },
      transcript: {
        executionId: "abc",
        startedAt: "2026-01-01T00:00:00Z",
        finishedAt: "2026-01-01T00:00:01Z",
        status: { type: "success" },
        fuelConsumed: 100,
        fuelLimit: 1000,
        peakMemoryBytes: 1024,
        memoryLimitBytes: 33554432,
        output: { type: "json", value: 42 },
        console: [],
        capabilityCalls: [],
      },
      outputArtifacts: [],
    };
    expect(r.ok).toBe(true);
    expect(r.status.type).toBe("success");
  });

  it("ok is false when status is not success", () => {
    const r: ExecutionResult = {
      ok: false,
      status: { type: "guest_error", message: "ReferenceError: x is not defined" },
      output: { type: "null" },
      transcript: {
        executionId: "def",
        startedAt: "2026-01-01T00:00:00Z",
        finishedAt: "2026-01-01T00:00:00Z",
        status: { type: "guest_error", message: "ReferenceError: x is not defined" },
        fuelConsumed: 50,
        fuelLimit: 1000,
        peakMemoryBytes: 512,
        memoryLimitBytes: 33554432,
        output: { type: "null" },
        console: [{ level: "error", message: "x is not defined", ts: 5 }],
        capabilityCalls: [],
      },
      outputArtifacts: [],
    };
    expect(r.ok).toBe(false);
    expect(r.transcript.console).toHaveLength(1);
  });
});
