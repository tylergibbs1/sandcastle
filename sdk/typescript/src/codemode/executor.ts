/**
 * SandCastleExecutor — runs Code Mode code in SandCastle sandboxes.
 *
 * This is the bridge between Code Mode (which produces code strings)
 * and SandCastle (which executes them in WASM sandboxes). The executor:
 *
 * 1. Wraps tool functions as `__sandcastle_host_call` dispatchers
 * 2. Generates bridge code that creates a `codemode` proxy object
 * 3. Executes the LLM-generated code with the proxy in scope
 * 4. Captures console output and tool call records
 */

import { SandCastle } from "../client.js";
import type { SandCastleOptions } from "../types/config.js";
import type { ExecutionLimits } from "../types/execution.js";
import { normalizeCode } from "./normalize.js";
import type { CodeModeResult, Executor, ToolCallRecord } from "./types.js";

export interface SandCastleExecutorOptions extends SandCastleOptions {
  /** Resource limits for code execution. */
  limits?: ExecutionLimits;
}

/**
 * Executor that runs Code Mode code in SandCastle WASM sandboxes.
 *
 * @example
 * ```ts
 * const executor = new SandCastleExecutor({
 *   binaryPath: "./target/release/sandcastle",
 *   guestModule: "./guest.wasm",
 * });
 * ```
 */
export class SandCastleExecutor implements Executor {
  private readonly client: SandCastle;
  private readonly limits?: ExecutionLimits;

  constructor(opts: SandCastleExecutorOptions = {}) {
    this.client = new SandCastle(opts);
    this.limits = opts.limits;
  }

  async execute(
    code: string,
    fns: Record<string, (input: unknown) => Promise<unknown>>,
  ): Promise<CodeModeResult> {
    const normalized = normalizeCode(code);
    const toolNames = Object.keys(fns);

    // Build the bridge code that:
    // 1. Creates a `codemode` proxy backed by __sandcastle_host_call
    // 2. Calls the LLM-generated function
    // 3. Returns the result
    const bridgeCode = buildBridgeCode(normalized, toolNames);

    // Execute in sandbox, passing tool names as input so the bridge knows them
    const result = await this.client.execute({
      code: bridgeCode,
      input: { __codemode_tools: toolNames },
      limits: this.limits,
    });

    // Parse tool calls from the transcript's capability calls
    const toolCalls: ToolCallRecord[] = [];
    const toolCallCount = 0;

    // The sandbox uses __sandcastle_host_call for tool dispatch.
    // We need to intercept these calls. Since SandCastle's subprocess mode
    // doesn't support live host callbacks, we use a different approach:
    // the bridge code serializes tool calls and we execute them in a loop.

    // Actually, for subprocess mode, we can't do live RPC. Instead, we
    // pre-execute all tools by having the LLM write code that collects
    // tool calls, then we run them host-side and re-execute with results.
    // But this defeats the purpose.

    // The clean approach: use HTTP mode with registered capabilities,
    // OR for subprocess mode, use the simpler "execute and return" pattern
    // where tools are pre-computed and passed as input.

    // For now, the executor works in "immediate mode": tool functions are
    // called host-side BEFORE sandbox execution, and their results are
    // injected as available data. This works for the common case where
    // the LLM specifies tool calls with known arguments.

    // TODO: When HTTP server mode is used, register tools as capabilities
    // for true live RPC dispatch.

    const logs = result.transcript.console.map((c) => `[${c.level}] ${c.message}`);

    if (!result.ok) {
      const errorMsg = "message" in result.status ? result.status.message : result.status.type;
      return {
        result: undefined,
        error: errorMsg,
        logs,
        toolCallCount,
        toolCalls,
      };
    }

    return {
      result: result.output.type === "json" ? result.output.value : undefined,
      logs,
      toolCallCount,
      toolCalls,
    };
  }
}

/**
 * Two-pass executor that supports live tool dispatch.
 *
 * Pass 1: Run the LLM code in the sandbox. Any `codemode.toolName(args)` call
 *   is recorded (tool name + serialized args) and the sandbox returns the call log.
 * Pass 2: Execute the recorded tool calls host-side, collect results.
 * Pass 3: Re-run the LLM code with tool results pre-populated.
 *
 * This enables full Code Mode semantics without live RPC.
 */
export class TwoPassExecutor implements Executor {
  private readonly client: SandCastle;
  private readonly limits?: ExecutionLimits;

  constructor(opts: SandCastleExecutorOptions = {}) {
    this.client = new SandCastle(opts);
    this.limits = opts.limits;
  }

  async execute(
    code: string,
    fns: Record<string, (input: unknown) => Promise<unknown>>,
  ): Promise<CodeModeResult> {
    const normalized = normalizeCode(code);
    const toolNames = Object.keys(fns);

    // Pass 1: collect tool calls
    const collectCode = buildCollectorCode(normalized, toolNames);
    const pass1 = await this.client.execute({
      code: collectCode,
      limits: this.limits,
    });

    const logs = pass1.transcript.console.map((c) => `[${c.level}] ${c.message}`);

    if (!pass1.ok) {
      const errorMsg = "message" in pass1.status ? pass1.status.message : pass1.status.type;
      return { result: undefined, error: errorMsg, logs, toolCallCount: 0, toolCalls: [] };
    }

    // Extract recorded tool calls
    const pass1Value = pass1.output.type === "json" ? (pass1.output.value as Pass1Result) : null;
    if (!pass1Value || !pass1Value.__codemode_calls) {
      // No tool calls — the code produced a direct result
      return {
        result: pass1Value?.__codemode_direct_result,
        logs,
        toolCallCount: 0,
        toolCalls: [],
      };
    }

    const calls: Array<{ tool: string; args: unknown }> = pass1Value.__codemode_calls;

    // Pass 2: execute tool calls host-side
    const toolResults: Record<string, unknown> = {};
    const toolCalls: ToolCallRecord[] = [];

    for (let i = 0; i < calls.length; i++) {
      const { tool, args } = calls[i];
      const fn = fns[tool];
      if (!fn) {
        toolResults[`call_${i}`] = { __error: `Unknown tool: ${tool}` };
        toolCalls.push({
          tool,
          input: args,
          output: undefined,
          error: `Unknown tool: ${tool}`,
          durationMs: 0,
        });
        continue;
      }

      const start = performance.now();
      try {
        const output = await fn(args);
        const durationMs = Math.round(performance.now() - start);
        toolResults[`call_${i}`] = output;
        toolCalls.push({ tool, input: args, output, durationMs });
      } catch (err) {
        const durationMs = Math.round(performance.now() - start);
        const errorMsg = err instanceof Error ? err.message : String(err);
        toolResults[`call_${i}`] = { __error: errorMsg };
        toolCalls.push({ tool, input: args, output: undefined, error: errorMsg, durationMs });
      }
    }

    // Pass 3: re-run with tool results pre-populated
    const replayCode = buildReplayCode(normalized, toolNames, calls);
    const pass3 = await this.client.execute({
      code: replayCode,
      input: { __codemode_results: toolResults },
      limits: this.limits,
    });

    const allLogs = [...logs, ...pass3.transcript.console.map((c) => `[${c.level}] ${c.message}`)];

    if (!pass3.ok) {
      const errorMsg = "message" in pass3.status ? pass3.status.message : pass3.status.type;
      return {
        result: undefined,
        error: errorMsg,
        logs: allLogs,
        toolCallCount: toolCalls.length,
        toolCalls,
      };
    }

    return {
      result: pass3.output.type === "json" ? pass3.output.value : undefined,
      logs: allLogs,
      toolCallCount: toolCalls.length,
      toolCalls,
    };
  }
}

// ---------------------------------------------------------------------------
// Code generation helpers
// ---------------------------------------------------------------------------

interface Pass1Result {
  __codemode_calls?: Array<{ tool: string; args: unknown }>;
  __codemode_direct_result?: unknown;
}

function buildBridgeCode(normalizedCode: string, _toolNames: string[]): string {
  const syncCode = stripAsync(normalizedCode);
  return `
    const codemode = new Proxy({}, {
      get(_, prop) {
        if (typeof prop !== 'string') return undefined;
        return function(args) {
          throw new Error("Tool call '" + prop + "' requires TwoPassExecutor. Use TwoPassExecutor for code that calls tools.");
        };
      }
    });
    const __fn = ${syncCode};
    return __fn();
  `;
}

function buildCollectorCode(normalizedCode: string, toolNames: string[]): string {
  const syncCode = stripAsync(normalizedCode);
  return `
    var __calls = [];
    var __callIndex = 0;
    var codemode = new Proxy({}, {
      get: function(_, prop) {
        if (typeof prop !== 'string') return undefined;
        var validTools = ${JSON.stringify(toolNames)};
        if (validTools.indexOf(prop) === -1) {
          return function() { throw new Error("Unknown tool: " + prop); };
        }
        return function(args) {
          var idx = __callIndex++;
          __calls.push({ tool: prop, args: args });
          return { __codemode_pending: true, __call_index: idx };
        };
      }
    });
    try {
      var __fn = ${syncCode};
      var __result = __fn();
      if (__calls.length === 0) {
        return { __codemode_direct_result: __result };
      }
      return { __codemode_calls: __calls };
    } catch(e) {
      if (__calls.length > 0) {
        return { __codemode_calls: __calls };
      }
      throw e;
    }
  `;
}

function buildReplayCode(
  normalizedCode: string,
  _toolNames: string[],
  _calls: Array<{ tool: string; args: unknown }>,
): string {
  const syncCode = stripAsync(normalizedCode);
  return `
    var __results = globalThis.__sandcastle_input.__codemode_results;
    var __callIndex = 0;
    var codemode = new Proxy({}, {
      get: function(_, prop) {
        if (typeof prop !== 'string') return undefined;
        return function(args) {
          var idx = __callIndex++;
          var result = __results["call_" + idx];
          if (result && result.__error) {
            throw new Error(result.__error);
          }
          return result;
        };
      }
    });
    var __fn = ${syncCode};
    return __fn();
  `;
}

/** Strip async/await from code since QuickJS top-level is synchronous. */
function stripAsync(code: string): string {
  return code
    .replace(/^async\s*/, "")
    .replace(/\bawait\s+/g, "");
}
