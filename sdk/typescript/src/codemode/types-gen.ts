/**
 * Generate TypeScript type declarations from tool definitions.
 *
 * Given a set of tools, produces a `.d.ts`-style string that tells the LLM
 * exactly what methods are available on the `codemode` proxy object.
 */
import type { JsonSchema, ToolDefinition } from "./types.js";

/** Generate TypeScript declarations for a set of tools. */
export function generateTypes(tools: ToolDefinition[] | Record<string, ToolDefinition>): string {
  const toolList = Array.isArray(tools) ? tools : Object.values(tools);

  const parts: string[] = [];
  const methodLines: string[] = [];

  for (const tool of toolList) {
    const inputTypeName = toPascalCase(tool.name) + "Input";
    const tsType = schemaToTypeScript(tool.inputSchema);

    parts.push(`type ${inputTypeName} = ${tsType};`);

    const desc = tool.description ? `  /** ${tool.description} */\n` : "";
    methodLines.push(`${desc}  ${tool.name}(input: ${inputTypeName}): Promise<unknown>;`);
  }

  parts.push("");
  parts.push("declare const codemode: {");
  parts.push(...methodLines);
  parts.push("};");

  return parts.join("\n");
}

/** Convert a JSON Schema to a TypeScript type string. */
function schemaToTypeScript(schema: JsonSchema): string {
  if (!schema || !schema.type) return "unknown";

  switch (schema.type) {
    case "string":
      if (schema.enum) {
        return schema.enum.map((v) => `"${v}"`).join(" | ");
      }
      return "string";

    case "number":
    case "integer":
      return "number";

    case "boolean":
      return "boolean";

    case "null":
      return "null";

    case "array": {
      const items = schema.items ? schemaToTypeScript(schema.items) : "unknown";
      return `${items}[]`;
    }

    case "object": {
      if (!schema.properties) return "Record<string, unknown>";
      const required = new Set(schema.required ?? []);
      const fields = Object.entries(schema.properties).map(([key, prop]) => {
        const opt = required.has(key) ? "" : "?";
        const type = schemaToTypeScript(prop);
        const desc = prop.description ? `  /** ${prop.description} */ ` : "";
        return `${desc}${key}${opt}: ${type}`;
      });
      return `{ ${fields.join("; ")} }`;
    }

    default:
      return "unknown";
  }
}

function toPascalCase(s: string): string {
  return s
    .split(/[_-]/)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join("");
}
