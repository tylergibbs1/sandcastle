import type { ExecutionLimits } from "./execution.js";

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
}
