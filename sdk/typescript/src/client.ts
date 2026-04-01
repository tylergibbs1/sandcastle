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
} from "./types/execution.js";
import type { DispatchNamespace, NamespaceConfig, ScriptConfig } from "./types/namespace.js";

/**
 * SandCastle client.
 *
 * Executes JavaScript inside isolated V8 sandboxes.
 * Supports three modes:
 * - **v8** (default): in-process V8 isolate via `isolated-vm` (~0.5ms/call)
 * - **subprocess**: spawns the `sandcastle` CLI binary per execution
 * - **HTTP**: talks to a running `sandcastle serve` instance
 *
 * @example Default (V8 in-process)
 * ```ts
 * const sc = new SandCastle();
 * const result = await sc.run<number>("return 1 + 1");
 * ```
 *
 * @example With host functions
 * ```ts
 * const sc = new SandCastle({
 *   hostFunctions: {
 *     getPrice: (ticker) => prices[ticker],
 *   },
 *   onConsole: (level, msg) => console.log(`[${level}] ${msg}`),
 * });
 * ```
 *
 * @example With isolate pooling (high throughput)
 * ```ts
 * const sc = new SandCastle({
 *   pool: { maxIsolates: 16 },
 * });
 * ```
 *
 * @example HTTP mode with namespaces
 * ```ts
 * const sc = new SandCastle({ httpEndpoint: "http://localhost:8080" });
 * const ns = sc.namespace("tenant-abc");
 * await ns.register("worker", 'return input.x * 2;');
 * const result = await ns.run<number>("worker", { x: 21 });
 * ```
 */
export class SandCastle {
  private readonly opts: SandCastleOptions;
  private pool: IsolatePool | null = null;
  private snapshot: unknown = null;
  private initialized = false;

  constructor(opts: SandCastleOptions = {}) {
    this.opts = opts;
  }

  /**
   * Lazy initialization for pool and snapshot (V8 mode only).
   * Called automatically on first execute().
   */
  private async ensureInitialized() {
    if (this.initialized) return;
    this.initialized = true;

    if (this.isHttp || this.isSubprocess) return;

    // Create snapshot if scripts provided
    if (this.opts.snapshotScripts?.length) {
      this.snapshot = await createSnapshot(this.opts.snapshotScripts);
    }

    // Create pool if configured
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

  /**
   * Execute JavaScript in a fresh sandbox and return the full result.
   */
  async execute(options: ExecuteOptions): Promise<ExecutionResult> {
    if (options.signal?.aborted) {
      throw new ExecutionAbortedError();
    }

    if (this.isHttp) {
      return executeViaHttp(this.opts.httpEndpoint!, options, this.opts.defaults);
    }
    if (this.isSubprocess) {
      return executeViaSubprocess(this.opts, options);
    }

    await this.ensureInitialized();
    return executeViaV8(
      options,
      this.opts.defaults,
      this.opts.hostFunctions,
      this.opts.onConsole,
      this.pool,
    );
  }

  /**
   * Execute JavaScript and return the output value directly.
   * Throws `ExecutionFailedError` (or a subclass) on any non-success status.
   */
  async run<T = unknown>(code: string, input?: unknown, limits?: ExecutionLimits): Promise<T> {
    const result = await this.execute({ code, input, limits });

    const err = errorFromResult(result);
    if (err) throw err;

    if (result.output.type === "json") return result.output.value as T;
    if (result.output.type === "string") return result.output.value as T;
    return undefined as T;
  }

  /**
   * Dispose the client and release all pooled resources.
   * Call this when you're done using the client to free V8 isolates.
   */
  dispose() {
    if (this.pool) {
      this.pool.dispose();
      this.pool = null;
    }
  }

  /**
   * Diagnose whether the local SDK install can resolve a working SandCastle binary.
   */
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

      async dispatch(
        scriptName: string,
        input?: unknown,
        limits?: ExecutionLimits,
      ): Promise<ExecutionResult> {
        return dispatchViaHttp(endpoint, scriptName, input, limits, name);
      },

      async run<T = unknown>(
        scriptName: string,
        input?: unknown,
        limits?: ExecutionLimits,
      ): Promise<T> {
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
// Convenience helpers
// ---------------------------------------------------------------------------

/** Create an `InputArtifact` from a UTF-8 string. */
export function textArtifact(name: string, content: string): InputArtifact {
  return { name, data: new TextEncoder().encode(content) };
}

/** Create an `InputArtifact` from a JSON-serializable value. */
export function jsonArtifact(name: string, value: unknown): InputArtifact {
  return textArtifact(name, JSON.stringify(value));
}
