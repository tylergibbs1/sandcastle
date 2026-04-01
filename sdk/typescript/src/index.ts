// Client
export { jsonArtifact, SandCastle, SandboxSession, textArtifact } from "./client.js";
export { diagnoseInstallation } from "./core/diagnostics.js";

// Zero-config standalone functions
export { run, execute, evaluate } from "./singleton.js";

// Errors
export {
  BinaryNotFoundError,
  ExecutionAbortedError,
  ExecutionFailedError,
  FuelExhaustedError,
  GuestError,
  MemoryExceededError,
  SandCastleError,
  TimeoutError,
} from "./core/errors.js";
export type { InstallationDiagnostics } from "./core/diagnostics.js";

// Middleware
export type { ExecutionContext, ExecutionMiddleware } from "./middleware.js";

// Types — config
export type { SandCastleOptions, HostFunction, OnConsoleCallback, V8PoolOptions } from "./types/config.js";

// Types — execution
export type {
  CapabilityCallEntry,
  ConsoleEntry,
  ExecuteOptions,
  ExecutionLimits,
  ExecutionResult,
  ExecutionStatus,
  ExecutionTranscript,
  InputArtifact,
  OutputArtifact,
  OutputValue,
  RunOptions,
} from "./types/execution.js";

// Types — namespaces
export type {
  DispatchNamespace,
  NamespaceConfig,
  ScriptConfig,
} from "./types/namespace.js";
