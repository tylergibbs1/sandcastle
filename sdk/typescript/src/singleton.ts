import { SandCastle } from "./client.js";
import type { ExecuteOptions, ExecutionLimits, ExecutionResult } from "./types/execution.js";

let instance: SandCastle | null = null;

function getDefault(): SandCastle {
  if (!instance) {
    instance = new SandCastle({ pool: { maxIsolates: 4 } });
    // Auto-cleanup on process exit
    if (typeof process !== "undefined" && process.on) {
      process.on("exit", () => instance?.dispose());
    }
  }
  return instance;
}

/**
 * Execute JavaScript in a sandbox. Zero setup required.
 *
 * @example
 * ```ts
 * import { run } from "@grayhaven/sandcastle";
 * const result = await run<number>("return 1 + 1");
 * ```
 */
export async function run<T = unknown>(
  code: string,
  inputOrOptions?: unknown,
  limits?: ExecutionLimits,
): Promise<T> {
  return getDefault().run<T>(code, inputOrOptions, limits);
}

/**
 * Execute JavaScript and return the full result. Zero setup required.
 *
 * @example
 * ```ts
 * import { execute } from "@grayhaven/sandcastle";
 * const { ok, output, transcript } = await execute({ code: "return 42" });
 * ```
 */
export async function execute(options: ExecuteOptions): Promise<ExecutionResult> {
  return getDefault().execute(options);
}

/**
 * Evaluate a JavaScript expression. No `return` needed. Zero setup required.
 *
 * @example
 * ```ts
 * import { evaluate } from "@grayhaven/sandcastle";
 * const sum = await evaluate<number>("1 + 1");           // 2
 * const greeting = await evaluate("name + '!'", { name: "Alice" }); // "Alice!"
 * ```
 */
export async function evaluate<T = unknown>(
  expression: string,
  globals?: Record<string, unknown>,
): Promise<T> {
  return getDefault().eval<T>(expression, globals);
}
