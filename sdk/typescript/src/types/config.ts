import type { ExecutionLimits, ConsoleEntry } from "./execution.js";

/** A host function exposed to sandboxed code. */
export type HostFunction = (...args: unknown[]) => unknown;

/** Callback invoked in real-time when guest code writes to console. */
export type OnConsoleCallback = (level: ConsoleEntry["level"], message: string, ts: number) => void;

/** Configuration for isolate pooling. */
export interface V8PoolOptions {
  /**
   * Maximum number of warm isolates to keep in the pool.
   * @default 8
   */
  maxIsolates?: number;

  /**
   * Maximum age (ms) before an isolate is discarded.
   * @default 30_000
   */
  maxAgeMs?: number;

  /**
   * Maximum times an isolate is reused before disposal.
   * @default 1000
   */
  maxUsesPerIsolate?: number;
}

/** Configuration for the SandCastle client. */
export interface SandCastleOptions {
  /**
   * Execution mode.
   * - `"v8"` (default): in-process V8 isolate via `isolated-vm` (~0.5ms/call).
   *   Requires `npm install isolated-vm`.
   * - `"subprocess"`: spawns the `sandcastle` CLI binary per call (~90ms/call).
   *   Requires the `sandcastle` binary installed.
   *
   * Setting `httpEndpoint` overrides this and uses HTTP mode.
   * @default "v8"
   */
  mode?: "v8" | "subprocess";

  /**
   * Path to the `sandcastle` CLI binary.
   * Only used in subprocess mode. Resolved via `PATH` when omitted.
   * @default "sandcastle"
   */
  binaryPath?: string;

  /**
   * Path to the guest WASM module.
   * Only used in subprocess mode. The CLI auto-detects when omitted.
   */
  guestModule?: string;

  /**
   * HTTP endpoint of a running SandCastle server (e.g. `"http://localhost:8080"`).
   * When set, the client uses HTTP instead of V8 or subprocess mode.
   */
  httpEndpoint?: string;

  /**
   * Default resource limits applied to every execution.
   * Per-call limits in `ExecuteOptions` override these.
   */
  defaults?: ExecutionLimits;

  /**
   * Host functions exposed to sandboxed code (V8 mode only).
   * These are callable from inside the sandbox as global functions.
   *
   * @example
   * ```ts
   * const sc = new SandCastle({
   *   hostFunctions: {
   *     fetchPrice: (ticker: string) => prices[ticker],
   *     log: (msg: string) => console.log("[sandbox]", msg),
   *   }
   * });
   * await sc.run("return fetchPrice('AAPL')");
   * ```
   */
  hostFunctions?: Record<string, HostFunction>;

  /**
   * Callback invoked in real-time when guest code writes to console (V8 mode only).
   * Called synchronously as `console.log/warn/error/debug` execute inside the sandbox.
   *
   * @example
   * ```ts
   * const sc = new SandCastle({
   *   onConsole: (level, message) => process.stderr.write(`[${level}] ${message}\n`),
   * });
   * ```
   */
  onConsole?: OnConsoleCallback;

  /**
   * Enable isolate pooling for higher throughput (V8 mode only).
   * When set, V8 isolates are reused across executions instead of
   * being created and destroyed per call.
   *
   * @example
   * ```ts
   * const sc = new SandCastle({
   *   pool: { maxIsolates: 16, maxAgeMs: 60_000 }
   * });
   * ```
   */
  pool?: V8PoolOptions;

  /**
   * Pre-warm V8 with these scripts (V8 mode only).
   * Creates a V8 snapshot with the given code pre-compiled, making
   * subsequent executions faster when they use the same libraries.
   *
   * @example
   * ```ts
   * const sc = new SandCastle({
   *   snapshotScripts: [
   *     "function helpers() { ... }",
   *     "const utils = { ... }",
   *   ]
   * });
   * ```
   */
  snapshotScripts?: string[];

  /**
   * Execution middleware for logging, metrics, rate-limiting, etc.
   * Can also be added later with `sc.use(middleware)`.
   */
  middleware?: import("../middleware.js").ExecutionMiddleware[];
}
