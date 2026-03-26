import type { ExecutionLimits } from "./execution.js";

/** Configuration for the SandCastle client. */
export interface SandCastleOptions {
  /**
   * Path to the `sandcastle` CLI binary.
   * Resolved via `PATH` when omitted. Ignored when `httpEndpoint` is set.
   * @default "sandcastle"
   */
  binaryPath?: string;

  /**
   * Path to the guest WASM module.
   * The CLI auto-detects when omitted. Ignored when `httpEndpoint` is set.
   */
  guestModule?: string;

  /**
   * HTTP endpoint of a running SandCastle server (e.g. `"http://localhost:8080"`).
   * When set, the client uses HTTP instead of spawning subprocesses.
   */
  httpEndpoint?: string;

  /**
   * Default resource limits applied to every execution.
   * Per-call limits in `ExecuteOptions` override these.
   */
  defaults?: ExecutionLimits;
}
