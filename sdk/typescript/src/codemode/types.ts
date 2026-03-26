/**
 * Code Mode types.
 *
 * These mirror the concepts from Cloudflare's @cloudflare/codemode but are
 * runtime-agnostic — they work with SandCastle's subprocess or HTTP transport.
 */

/** A tool definition that Code Mode can expose to the sandbox. */
export interface ToolDefinition {
  /** Tool name — becomes a method on the `codemode` proxy in the sandbox. */
  name: string;

  /** Human-readable description included in the type declarations. */
  description: string;

  /**
   * JSON Schema for the tool's input parameter.
   * Used to generate TypeScript type declarations.
   */
  inputSchema: JsonSchema;

  /** The host-side function that executes when the sandbox calls this tool. */
  execute: (input: unknown) => Promise<unknown>;
}

/** Subset of JSON Schema we use for TypeScript generation. */
export interface JsonSchema {
  type?: string;
  properties?: Record<string, JsonSchema>;
  items?: JsonSchema;
  required?: string[];
  description?: string;
  enum?: string[];
  default?: unknown;
}

/** Result of executing code in Code Mode. */
export interface CodeModeResult {
  /** The value returned by the generated code. */
  result: unknown;

  /** Error message if execution failed. */
  error?: string;

  /** Captured console output from the sandbox. */
  logs: string[];

  /** Number of tool calls made during execution. */
  toolCallCount: number;

  /** Individual tool call records. */
  toolCalls: ToolCallRecord[];
}

/** A record of a single tool call made during code execution. */
export interface ToolCallRecord {
  tool: string;
  input: unknown;
  output: unknown;
  error?: string;
  durationMs: number;
}

/**
 * Executor interface — abstracts the sandbox runtime.
 *
 * SandCastle provides `SandCastleExecutor` which runs code in WASM sandboxes.
 * You can implement this interface for other runtimes (Node VM, iframe, etc.).
 */
export interface Executor {
  execute(
    code: string,
    fns: Record<string, (input: unknown) => Promise<unknown>>,
  ): Promise<CodeModeResult>;
}

/**
 * Options for `createCodeTool`.
 */
export interface CodeToolOptions {
  /** Tool definitions to expose inside the sandbox. */
  tools: ToolDefinition[] | Record<string, ToolDefinition>;

  /** The executor that runs generated code. */
  executor: Executor;

  /**
   * Custom description for the code tool.
   * Use `{{types}}` as a placeholder for the generated type declarations.
   * @default A sensible default describing the code execution capability.
   */
  description?: string;
}

/**
 * The code tool returned by `createCodeTool`.
 * This is a single tool definition you pass to your LLM.
 */
export interface CodeTool {
  name: "codemode";
  description: string;
  inputSchema: {
    type: "object";
    properties: {
      code: { type: "string"; description: string };
    };
    required: ["code"];
  };
  execute: (input: { code: string }) => Promise<CodeModeResult>;
}
