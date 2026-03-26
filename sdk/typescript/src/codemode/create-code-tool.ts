/**
 * createCodeTool — the main entry point for Code Mode.
 *
 * Takes your tool definitions and an executor, returns a single tool
 * that you give to your LLM. The LLM writes code that calls your tools
 * via a `codemode` proxy, and the executor runs it in a sandbox.
 */
import type { CodeTool, CodeToolOptions, Executor, ToolDefinition } from "./types.js";
import { generateTypes } from "./types-gen.js";

const DEFAULT_DESCRIPTION = `Execute JavaScript code that can call the following typed APIs via the \`codemode\` object.

Write an async arrow function. Call tools via \`codemode.toolName(input)\`. Return the final result.

Available APIs:
\`\`\`typescript
{{types}}
\`\`\`

Rules:
- Write an async arrow function: \`async () => { ... }\`
- Call tools as \`codemode.toolName({ key: value })\` — each call returns a Promise
- You can chain multiple tool calls, use variables, loops, conditionals
- Return the final result as the last expression
- console.log() output is captured but not returned
- No fetch(), no require(), no import — only the codemode API is available`;

/**
 * Create a single "code mode" tool from a set of tool definitions.
 *
 * Instead of exposing N tools to the LLM (which leads to N sequential
 * tool_use calls), this creates ONE tool where the LLM writes code
 * that calls all N tools in a single function. This cuts token usage
 * by up to 80% for multi-tool workflows.
 *
 * @example
 * ```ts
 * import { createCodeTool, SandCastleExecutor } from "sandcastle/codemode";
 *
 * const executor = new SandCastleExecutor();
 * const codemode = createCodeTool({
 *   tools: [
 *     { name: "getUser", description: "Get user by ID", inputSchema: { type: "object", properties: { id: { type: "number" } }, required: ["id"] }, execute: async ({ id }) => db.getUser(id) },
 *     { name: "sendEmail", description: "Send email", inputSchema: { type: "object", properties: { to: { type: "string" }, body: { type: "string" } }, required: ["to", "body"] }, execute: async (input) => mailer.send(input) },
 *   ],
 *   executor,
 * });
 *
 * // Give `codemode` to your LLM as a tool. It will write code like:
 * // async () => {
 * //   const user = await codemode.getUser({ id: 42 });
 * //   await codemode.sendEmail({ to: user.email, body: "Hello!" });
 * //   return { sent: true };
 * // }
 * ```
 */
export function createCodeTool(options: CodeToolOptions): CodeTool {
  const { executor } = options;

  const toolList = normalizeTools(options.tools);
  const types = generateTypes(toolList);
  const description = (options.description ?? DEFAULT_DESCRIPTION).replace("{{types}}", types);

  // Build the function map for the executor
  const fnMap: Record<string, (input: unknown) => Promise<unknown>> = {};
  for (const tool of toolList) {
    fnMap[tool.name] = tool.execute;
  }

  return {
    name: "codemode",
    description,
    inputSchema: {
      type: "object",
      properties: {
        code: {
          type: "string",
          description:
            "An async arrow function that calls tools via `codemode.toolName(input)` and returns a result.",
        },
      },
      required: ["code"],
    },
    execute: async (input: { code: string }) => {
      return executor.execute(input.code, fnMap);
    },
  };
}

function normalizeTools(
  tools: ToolDefinition[] | Record<string, ToolDefinition>,
): ToolDefinition[] {
  if (Array.isArray(tools)) return tools;
  return Object.entries(tools).map(([name, def]) => ({
    ...def,
    name: def.name ?? name,
  }));
}
