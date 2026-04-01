import type { ExecuteOptions, ExecutionResult } from "./types/execution.js";

/** Context passed to middleware hooks. */
export interface ExecutionContext {
  /** The options passed to execute(). */
  readonly options: ExecuteOptions;
  /** High-resolution timestamp when execution started. */
  readonly startTime: number;
  /** Arbitrary metadata — stash data here to share between before/after hooks. */
  metadata: Record<string, unknown>;
}

/**
 * Middleware hooks for the execution lifecycle.
 *
 * @example Logging
 * ```ts
 * sc.use({
 *   afterExecute(ctx, result) {
 *     console.log(`Executed in ${Date.now() - ctx.startTime}ms, status: ${result.status.type}`);
 *   }
 * });
 * ```
 *
 * @example Rate limiting
 * ```ts
 * let count = 0;
 * sc.use({
 *   beforeExecute() {
 *     if (++count > 100) throw new Error("rate limit exceeded");
 *   }
 * });
 * ```
 */
export interface ExecutionMiddleware {
  beforeExecute?(ctx: ExecutionContext): void | Promise<void>;
  afterExecute?(ctx: ExecutionContext, result: ExecutionResult): void | Promise<void>;
  onError?(ctx: ExecutionContext, error: Error): void | Promise<void>;
}
