// Client
export { jsonArtifact, SandCastle, SandboxSession, textArtifact } from "./client.js";

// Zero-config standalone functions
export { run, execute, evaluate } from "./singleton.js";

// Errors
export {
  ExecutionAbortedError,
  ExecutionFailedError,
  FuelExhaustedError,
  GuestError,
  MemoryExceededError,
  SandCastleError,
  TimeoutError,
} from "./core/errors.js";

// Middleware
export type { ExecutionContext, ExecutionMiddleware } from "./middleware.js";

// Types — config
export type { SandCastleOptions, HostFunction, OnConsoleCallback, V8PoolOptions } from "./types/config.js";

// Types — execution
export type {
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
