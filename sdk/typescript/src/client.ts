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
 * Executes JavaScript code inside lightweight WASM sandboxes.
 * Supports two modes:
 * - **subprocess** (default): spawns the CLI binary per execution
 * - **HTTP**: talks to a running `sandcastle serve` instance
 *
 * @example Subprocess mode
 * ```ts
 * const sc = new SandCastle();
 * const result = await sc.run<number>("return 1 + 1");
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

  constructor(opts: SandCastleOptions = {}) {
    this.opts = opts;
  }

  private get isHttp(): boolean {
    return !!this.opts.httpEndpoint;
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
    return executeViaSubprocess(this.opts, options);
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
   * Diagnose whether the local SDK install can resolve a working SandCastle binary.
   */
  async diagnoseInstallation() {
    return diagnoseInstallation(this.opts.binaryPath ?? "sandcastle");
  }

  // -------------------------------------------------------------------------
  // Script registry (requires HTTP mode)
  // -------------------------------------------------------------------------

  /**
   * Register a named script for fast dispatch.
   * Requires HTTP mode (`httpEndpoint` set).
   */
  async register(name: string, code: string, config?: ScriptConfig): Promise<void> {
    this.requireHttp("register");
    return registerViaHttp(this.opts.httpEndpoint!, name, code, config?.limits);
  }

  /**
   * Dispatch to a pre-registered script by name.
   * Requires HTTP mode (`httpEndpoint` set).
   */
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

  /**
   * Create a dispatch namespace for multi-tenant isolation.
   * Requires HTTP mode (`httpEndpoint` set).
   */
  async createNamespace(name: string, config?: NamespaceConfig): Promise<DispatchNamespace> {
    this.requireHttp("createNamespace");
    await createNamespaceViaHttp(this.opts.httpEndpoint!, name, config);
    return this.namespace(name);
  }

  /**
   * Delete a dispatch namespace.
   * Requires HTTP mode (`httpEndpoint` set).
   */
  async deleteNamespace(name: string): Promise<boolean> {
    this.requireHttp("deleteNamespace");
    return deleteNamespaceViaHttp(this.opts.httpEndpoint!, name);
  }

  /**
   * Get a handle to a dispatch namespace.
   * Requires HTTP mode (`httpEndpoint` set).
   *
   * @example
   * ```ts
   * const ns = sc.namespace("tenant-abc");
   * await ns.register("worker", 'return input.x + 1;');
   * const result = await ns.run<number>("worker", { x: 41 });
   * ```
   */
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
