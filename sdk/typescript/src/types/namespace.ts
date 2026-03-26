import type { ExecutionLimits, ExecutionResult } from "./execution.js";

/** Configuration for creating a dispatch namespace. */
export interface NamespaceConfig {
  /** Maximum scripts in this namespace. @default 1000 */
  maxScripts?: number;
  /** Maximum concurrent executions. @default 100 */
  maxConcurrent?: number;
}

/** Options for registering a script. */
export interface ScriptConfig {
  /** Resource limits for this script. */
  limits?: ExecutionLimits;
}

/** A dispatch namespace client for multi-tenant script management. */
export interface DispatchNamespace {
  /** Register a named script in this namespace. */
  register(name: string, code: string, config?: ScriptConfig): Promise<void>;

  /** Remove a script from this namespace. */
  remove(name: string): Promise<boolean>;

  /** Execute a named script with the given input. */
  dispatch(name: string, input?: unknown, limits?: ExecutionLimits): Promise<ExecutionResult>;

  /** Execute a named script and return the output value directly. Throws on failure. */
  run<T = unknown>(name: string, input?: unknown, limits?: ExecutionLimits): Promise<T>;

  /** List all scripts in this namespace. */
  listScripts(): Promise<string[]>;
}
