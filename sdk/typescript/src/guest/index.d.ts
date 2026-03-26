/**
 * SandCastle Guest Type Declarations
 *
 * These types describe the APIs available inside a SandCastle sandbox.
 * Feed this file to your LLM alongside the task prompt so the agent knows
 * what APIs are available and gets proper completions.
 *
 * Import path: `sandcastle/guest`
 */

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/**
 * The JSON input passed to this execution.
 * Also available as `globalThis.__sandcastle_input`.
 */
declare const input: unknown;

// ---------------------------------------------------------------------------
// Console (captured in transcript)
// ---------------------------------------------------------------------------

declare const console: {
  log(...args: unknown[]): void;
  warn(...args: unknown[]): void;
  error(...args: unknown[]): void;
  debug(...args: unknown[]): void;
};

// ---------------------------------------------------------------------------
// Built-in globals
// ---------------------------------------------------------------------------

declare const JSON: typeof globalThis.JSON;
declare const Math: typeof globalThis.Math;
declare const Date: typeof globalThis.Date;
declare const URL: typeof globalThis.URL;
declare const URLSearchParams: typeof globalThis.URLSearchParams;
declare const TextEncoder: typeof globalThis.TextEncoder;
declare const TextDecoder: typeof globalThis.TextDecoder;
declare function atob(data: string): string;
declare function btoa(data: string): string;
declare const crypto: {
  randomUUID(): string;
  getRandomValues<T extends ArrayBufferView>(array: T): T;
};

// ---------------------------------------------------------------------------
// Virtual filesystem (artifacts)
// ---------------------------------------------------------------------------

/** Read an input artifact. Returns `null` if not found. */
declare function __sandcastle_read_artifact(name: string): string | null;

/** Write an output artifact. Returns `true` on success. */
declare function __sandcastle_write_artifact(name: string, data: string): boolean;

// ---------------------------------------------------------------------------
// Host capability calls
// ---------------------------------------------------------------------------

/**
 * Call a host capability method.
 *
 * @param capability - Capability name (e.g. `"http"`, `"user_service"`)
 * @param method - Method name on the capability
 * @param payload - JSON-encoded arguments
 * @returns JSON-encoded response
 */
declare function __sandcastle_host_call(
  capability: string,
  method: string,
  payload: string,
): string;
