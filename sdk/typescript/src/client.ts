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

  /**
   * Create a reusable sandboxed function. Call it like a normal function.
   *
   * @example
   * ```ts
   * const double = sc.wrap<number, [number]>("return args[0] * 2");
   * await double(21);  // 42
   * await double(5);   // 10
   *
   * // With named params
   * const greet = sc.wrap<string>("return `Hello, ${name}!`");
   * await greet({ name: "Alice" });  // "Hello, Alice!"
   * ```
   */
  wrap<TReturn = unknown, TArgs extends unknown[] = unknown[]>(
    code: string,
    opts?: { limits?: ExecutionLimits },
  ): (...argsOrGlobals: TArgs | [Record<string, unknown>]) => Promise<TReturn> {
    return async (...args) => {
      // If single object arg, treat as globals. Otherwise inject as `args` array.
      if (args.length === 1 && typeof args[0] === "object" && args[0] !== null && !Array.isArray(args[0])) {
        return this.run<TReturn>(code, { globals: args[0] as Record<string, unknown>, limits: opts?.limits });
      }
      return this.run<TReturn>(code, { globals: { args }, limits: opts?.limits });
    };
  }

  /**
   * Create a persistent sandbox session. State carries across calls.
   *
   * @example
   * ```ts
   * const session = await sc.session();
   * await session.run("var counter = 0");
   * await session.run("counter++");
   * await session.run<number>("return counter");  // 1
   * session.dispose();
   * ```
   */
  async session(limits?: ExecutionLimits): Promise<SandboxSession> {
    await this.ensureInitialized();
    return SandboxSession.create(limits ?? this.opts.defaults);
  }

  /**
   * Run multiple scripts in parallel. Returns results in order.
   *
   * @example
   * ```ts
   * const results = await sc.batch([
   *   { code: "return 1 + 1" },
   *   { code: "return 2 + 2" },
   *   { code: "return 3 + 3" },
   * ]);
   * // [2, 4, 6]
   * ```
   */
  async batch<T = unknown>(
    items: Array<string | ExecuteOptions>,
  ): Promise<T[]> {
    const promises = items.map((item) => {
      const opts = typeof item === "string" ? { code: item } : item;
      return this.run<T>(opts.code, {
        input: opts.input,
        globals: opts.globals,
        limits: opts.limits,
      });
    });
    return Promise.all(promises);
  }

  /**
   * Test if code runs without errors. Returns `true` on success, `false` on failure.
   * Never throws.
   *
   * @example
   * ```ts
   * await sc.test("return 1 + 1");           // true
   * await sc.test("throw new Error('no')");   // false
   * await sc.test("while(true){}");           // false (timeout)
   * ```
   */
  async test(code: string, limits?: ExecutionLimits): Promise<boolean> {
    try {
      const result = await this.execute({ code, limits: { timeoutMs: 5_000, ...limits } });
      return result.ok;
    } catch {
      return false;
    }
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

// ---------------------------------------------------------------------------
// SandboxSession — persistent state across calls
// ---------------------------------------------------------------------------

/**
 * A persistent sandbox session. State (variables, functions) carries across calls.
 * Created via `sc.session()`.
 */
export class SandboxSession {
  private isolate: unknown;
  private context: unknown;
  private disposed = false;
  private iv: typeof import("isolated-vm") | null = null;

  private constructor() {}

  /** @internal */
  static async create(limits?: ExecutionLimits): Promise<SandboxSession> {
    const session = new SandboxSession();
    try {
      session.iv = await import("isolated-vm");
    } catch {
      throw new Error("isolated-vm is required for sessions. Install it: npm install isolated-vm");
    }
    const iv = session.iv;
    const memoryMb = limits?.memoryMb ?? 128;
    session.isolate = new iv.Isolate({ memoryLimit: memoryMb });
    session.context = (session.isolate as InstanceType<typeof iv.Isolate>).createContextSync();

    // Set up console
    const ctx = session.context as InstanceType<typeof iv.Context>;
    ctx.evalSync(`
      var console = { log(){}, warn(){}, error(){}, debug(){} };
      var __sc_timers = {};
      console.time = (label) => { __sc_timers[label || 'default'] = Date.now(); };
      console.timeEnd = (label) => {
        const k = label || 'default';
        const start = __sc_timers[k];
        if (start) { delete __sc_timers[k]; }
      };
    `);
    return session;
  }

  /**
   * Run code in this session. Variables persist across calls.
   * Use `var` for declarations that should persist.
   *
   * @example
   * ```ts
   * await session.run("var counter = 0");
   * await session.run("counter++");
   * await session.run<number>("return counter"); // 1
   * ```
   */
  async run<T = unknown>(code: string, globals?: Record<string, unknown>): Promise<T> {
    if (this.disposed) throw new Error("Session has been disposed");
    const iv = this.iv!;
    const ctx = this.context as InstanceType<typeof iv.Context>;
    const jail = ctx.global;

    if (globals) {
      for (const [key, val] of Object.entries(globals)) {
        const copy = new iv.ExternalCopy(val);
        jail.setSync(`__sg_${key}`, copy);
        ctx.evalSync(`var ${key} = __sg_${key}.copy();`);
        copy.release();
      }
    }

    // If code starts with `return`, wrap in IIFE + serialize.
    // Otherwise execute at top-level scope so declarations persist.
    if (code.trimStart().startsWith("return ")) {
      const wrapped = `JSON.stringify((()=>{${code}})())`;
      const raw = ctx.evalSync(wrapped, { timeout: 10000 });
      const s = String(raw);
      return (s === "undefined" ? undefined : JSON.parse(s)) as T;
    }

    ctx.evalSync(code, { timeout: 10000 });
    return undefined as T;
  }

  /** Evaluate an expression in this session. No `return` needed. */
  async eval<T = unknown>(expression: string, globals?: Record<string, unknown>): Promise<T> {
    return this.run<T>(`return (${expression})`, globals);
  }

  /** Dispose the session and free V8 resources. */
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    const iv = this.iv!;
    (this.context as InstanceType<typeof iv.Context>).release();
    const iso = this.isolate as InstanceType<typeof iv.Isolate>;
    if (!iso.isDisposed) iso.dispose();
  }
}

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
