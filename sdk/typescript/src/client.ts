import { ExecutionAbortedError, errorFromResult } from "./core/errors.js";
import { diagnoseInstallation } from "./core/diagnostics.js";
import {
  createNamespaceViaHttp,
  deleteNamespaceViaHttp,
  dispatchViaHttp,
  executeViaHttp,
  listScriptsViaHttp,
  registerViaHttp,
} from "./core/http.js";
import { executeViaSubprocess } from "./core/subprocess.js";
import { executeViaV8, IsolatePool, createSnapshot } from "./core/v8.js";
import type { SandCastleOptions } from "./types/config.js";
import type {
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  InputArtifact,
  RunOptions,
} from "./types/execution.js";
import type { DispatchNamespace, NamespaceConfig, ScriptConfig } from "./types/namespace.js";
import type { ExecutionContext, ExecutionMiddleware } from "./middleware.js";

/**
 * SandCastle — the easiest way to run untrusted JavaScript safely.
 *
 * @example Zero-config
 * ```ts
 * const sc = new SandCastle();
 * const result = await sc.run<number>("return 1 + 1");
 * ```
 *
 * @example With globals
 * ```ts
 * const answer = await sc.eval<number>("x + y", { x: 40, y: 2 });
 * ```
 *
 * @example With host functions
 * ```ts
 * const sc = new SandCastle({
 *   hostFunctions: { getPrice: (t) => prices[t] },
 * });
 * ```
 *
 * @example Presets
 * ```ts
 * const sc = SandCastle.strict();     // tight limits
 * const sc = SandCastle.permissive(); // generous limits + pooling
 * ```
 */
export class SandCastle {
  private readonly opts: SandCastleOptions;
  private pool: IsolatePool | null = null;
  private snapshot: unknown = null;
  private initialized = false;
  private middlewares: ExecutionMiddleware[] = [];

  constructor(opts: SandCastleOptions = {}) {
    this.opts = opts;
    if (opts.middleware) {
      this.middlewares = [...opts.middleware];
    }
  }

  // -------------------------------------------------------------------------
  // Presets
  // -------------------------------------------------------------------------

  /** Tight limits: 32MB, 1s timeout. Good for untrusted user code. */
  static strict(overrides?: Partial<SandCastleOptions>): SandCastle {
    return new SandCastle({
      defaults: { memoryMb: 32, timeoutMs: 1_000, maxOutputBytes: 65_536 },
      pool: { maxIsolates: 4 },
      ...overrides,
    });
  }

  /** Generous limits: 512MB, 60s timeout, large pool. Good for internal tools. */
  static permissive(overrides?: Partial<SandCastleOptions>): SandCastle {
    return new SandCastle({
      defaults: { memoryMb: 512, timeoutMs: 60_000, maxOutputBytes: 10_485_760 },
      pool: { maxIsolates: 16 },
      ...overrides,
    });
  }

  // -------------------------------------------------------------------------
  // Middleware
  // -------------------------------------------------------------------------

  /**
   * Add execution middleware. Returns `this` for chaining.
   *
   * @example
   * ```ts
   * sc.use({
   *   afterExecute(ctx, result) {
   *     console.log(`Took ${Date.now() - ctx.startTime}ms`);
   *   }
   * });
   * ```
   */
  use(middleware: ExecutionMiddleware): this {
    this.middlewares.push(middleware);
    return this;
  }

  // -------------------------------------------------------------------------
  // Core execution
  // -------------------------------------------------------------------------

  private async ensureInitialized() {
    if (this.initialized) return;
    this.initialized = true;
    if (this.isHttp || this.isSubprocess) return;
    if (this.opts.snapshotScripts?.length) {
      this.snapshot = await createSnapshot(this.opts.snapshotScripts);
    }
    if (this.opts.pool) {
      this.pool = new IsolatePool({
        ...this.opts.pool,
        memoryMb: this.opts.defaults?.memoryMb ?? 128,
        snapshot: this.snapshot as never,
      });
    }
  }

  private get isHttp(): boolean {
    return !!this.opts.httpEndpoint;
  }

  private get isSubprocess(): boolean {
    return this.opts.mode === "subprocess";
  }

  /** Execute JavaScript in a fresh sandbox and return the full result. */
  async execute(options: ExecuteOptions): Promise<ExecutionResult> {
    if (options.signal?.aborted) {
      throw new ExecutionAbortedError();
    }

    const ctx: ExecutionContext = {
      options,
      startTime: Date.now(),
      metadata: {},
    };

    // Run beforeExecute middleware
    for (const mw of this.middlewares) {
      if (mw.beforeExecute) await mw.beforeExecute(ctx);
    }

    let result: ExecutionResult;
    try {
      if (this.isHttp) {
        result = await executeViaHttp(this.opts.httpEndpoint!, options, this.opts.defaults);
      } else if (this.isSubprocess) {
        result = await executeViaSubprocess(this.opts, options);
      } else {
        await this.ensureInitialized();
        result = await executeViaV8(
          options,
          this.opts.defaults,
          this.opts.hostFunctions,
          this.opts.onConsole,
          this.pool,
        );
      }
    } catch (err) {
      for (const mw of [...this.middlewares].reverse()) {
        if (mw.onError) await mw.onError(ctx, err as Error);
      }
      throw err;
    }

    // Run afterExecute middleware (reverse order)
    for (const mw of [...this.middlewares].reverse()) {
      if (mw.afterExecute) await mw.afterExecute(ctx, result);
    }

    return result;
  }

  /**
   * Execute JavaScript and return the output value directly.
   * Throws on non-success status.
   *
   * The second argument can be:
   * - A plain value → used as `input` (legacy)
   * - A `RunOptions` object → `{ globals, input, limits, signal }`
   *
   * @example
   * ```ts
   * await sc.run("return 1 + 1");
   * await sc.run("return input.x * 2", { x: 21 });
   * await sc.run("return name", { globals: { name: "Alice" } });
   * ```
   */
  async run<T = unknown>(
    code: string,
    inputOrOptions?: unknown,
    limits?: ExecutionLimits,
  ): Promise<T> {
    let execOpts: ExecuteOptions;

    if (isRunOptions(inputOrOptions)) {
      execOpts = {
        code,
        input: inputOrOptions.input,
        globals: inputOrOptions.globals,
        limits: inputOrOptions.limits ?? limits,
        signal: inputOrOptions.signal,
      };
    } else {
      execOpts = { code, input: inputOrOptions, limits };
    }

    const result = await this.execute(execOpts);
    const err = errorFromResult(result);
    if (err) throw err;

    if (result.output.type === "json") return result.output.value as T;
    if (result.output.type === "string") return result.output.value as T;
    return undefined as T;
  }

  /**
   * Evaluate an expression and return the result. No `return` needed.
   *
   * @example
   * ```ts
   * await sc.eval("1 + 1");                      // 2
   * await sc.eval("x + y", { x: 40, y: 2 });     // 42
   * await sc.eval("items.filter(x => x > 2)", { items: [1,2,3,4] }); // [3,4]
   * ```
   */
  async eval<T = unknown>(
    expression: string,
    globals?: Record<string, unknown>,
  ): Promise<T> {
    return this.run<T>(`return (${expression})`, { globals });
  }

  /** Dispose the client and release all pooled resources. */
  dispose() {
    if (this.pool) {
      this.pool.dispose();
      this.pool = null;
    }
  }

  /** Diagnose whether the local SDK install can resolve a working SandCastle binary. */
  async diagnoseInstallation() {
    return diagnoseInstallation(this.opts.binaryPath ?? "sandcastle");
  }

  // -------------------------------------------------------------------------
  // Script registry (requires HTTP mode)
  // -------------------------------------------------------------------------

  async register(name: string, code: string, config?: ScriptConfig): Promise<void> {
    this.requireHttp("register");
    return registerViaHttp(this.opts.httpEndpoint!, name, code, config?.limits);
  }

  async dispatch(
    name: string,
    input?: unknown,
    limits?: ExecutionLimits,
  ): Promise<ExecutionResult> {
    this.requireHttp("dispatch");
    return dispatchViaHttp(this.opts.httpEndpoint!, name, input, limits);
  }

  // -------------------------------------------------------------------------
  // Namespaces (requires HTTP mode)
  // -------------------------------------------------------------------------

  async createNamespace(name: string, config?: NamespaceConfig): Promise<DispatchNamespace> {
    this.requireHttp("createNamespace");
    await createNamespaceViaHttp(this.opts.httpEndpoint!, name, config);
    return this.namespace(name);
  }

  async deleteNamespace(name: string): Promise<boolean> {
    this.requireHttp("deleteNamespace");
    return deleteNamespaceViaHttp(this.opts.httpEndpoint!, name);
  }

  namespace(name: string): DispatchNamespace {
    this.requireHttp("namespace");
    const endpoint = this.opts.httpEndpoint!;

    return {
      async register(scriptName: string, code: string, config?: ScriptConfig): Promise<void> {
        return registerViaHttp(endpoint, scriptName, code, config?.limits, name);
      },
      async remove(scriptName: string): Promise<boolean> {
        const res = await fetch(`${endpoint}/namespaces/${name}/scripts/${scriptName}`, {
          method: "DELETE",
        });
        return res.ok;
      },
      async dispatch(scriptName: string, input?: unknown, limits?: ExecutionLimits): Promise<ExecutionResult> {
        return dispatchViaHttp(endpoint, scriptName, input, limits, name);
      },
      async run<T = unknown>(scriptName: string, input?: unknown, limits?: ExecutionLimits): Promise<T> {
        const result = await dispatchViaHttp(endpoint, scriptName, input, limits, name);
        const err = errorFromResult(result);
        if (err) throw err;
        if (result.output.type === "json") return result.output.value as T;
        if (result.output.type === "string") return result.output.value as T;
        return undefined as T;
      },
      async listScripts(): Promise<string[]> {
        return listScriptsViaHttp(endpoint, name);
      },
    };
  }

  private requireHttp(method: string): void {
    if (!this.isHttp) {
      throw new Error(`${method}() requires HTTP mode. Set httpEndpoint in SandCastleOptions.`);
    }
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isRunOptions(val: unknown): val is RunOptions {
  if (typeof val !== "object" || val === null) return false;
  return "globals" in val || "limits" in val || "signal" in val;
}

/** Create an `InputArtifact` from a UTF-8 string. */
export function textArtifact(name: string, content: string): InputArtifact {
  return { name, data: new TextEncoder().encode(content) };
}

/** Create an `InputArtifact` from a JSON-serializable value. */
export function jsonArtifact(name: string, value: unknown): InputArtifact {
  return textArtifact(name, JSON.stringify(value));
}
