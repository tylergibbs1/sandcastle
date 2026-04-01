// Client
export { jsonArtifact, SandCastle, textArtifact } from "./client.js";
export { diagnoseInstallation } from "./core/diagnostics.js";

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

// Types — config
export type { SandCastleOptions, HostFunction, OnConsoleCallback, V8PoolOptions } from "./types/config.js";
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
} from "./types/execution.js";

// Types — namespaces
export type {
  DispatchNamespace,
  NamespaceConfig,
  ScriptConfig,
} from "./types/namespace.js";
