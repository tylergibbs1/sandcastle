/**
 * Normalize LLM-generated code into a consistent async arrow function.
 *
 * Handles common LLM quirks:
 * - Strips markdown code fences (```js ... ```)
 * - Wraps bare expressions in `return`
 * - Wraps non-arrow code in `async () => { ... }`
 * - Handles `async function() { ... }` format
 */

/**
 * Normalize code from an LLM into an async arrow function string.
 *
 * @example
 * ```ts
 * normalizeCode("```js\nconst x = 1;\nx\n```");
 * // "async () => {\nconst x = 1;\nreturn (x)\n}"
 *
 * normalizeCode("async () => { return 42; }");
 * // "async () => { return 42; }"
 * ```
 */
export function normalizeCode(raw: string): string {
  let code = raw.trim();

  // Strip markdown code fences
  code = stripCodeFences(code);

  // Already an async arrow function — return as-is
  if (/^async\s*\(/.test(code) || /^async\s*\w+\s*=>/.test(code)) {
    return code;
  }

  // `async function ...` — convert to arrow
  if (/^async\s+function/.test(code)) {
    const bodyMatch = code.match(/\{([\s\S]*)\}\s*$/);
    if (bodyMatch) {
      return `async () => {\n${bodyMatch[1].trim()}\n}`;
    }
  }

  // Regular function or just statements — wrap in async arrow
  // If the last statement is an expression (not a return/throw/if/etc),
  // add an implicit return
  const lines = code.split("\n").filter((l) => l.trim());
  if (lines.length > 0) {
    const lastLine = lines[lines.length - 1].trim();
    if (!isStatement(lastLine)) {
      lines[lines.length - 1] = `return (${lastLine})`;
    }
  }

  return `async () => {\n${lines.join("\n")}\n}`;
}

function stripCodeFences(code: string): string {
  // Match ```lang ... ``` or ``` ... ```
  const fenceMatch = code.match(/^```\w*\s*\n?([\s\S]*?)\n?\s*```$/);
  if (fenceMatch) {
    return fenceMatch[1].trim();
  }
  return code;
}

function isStatement(line: string): boolean {
  return (
    line.startsWith("return ") ||
    line.startsWith("return;") ||
    line.startsWith("throw ") ||
    line.startsWith("if ") ||
    line.startsWith("if(") ||
    line.startsWith("for ") ||
    line.startsWith("for(") ||
    line.startsWith("while ") ||
    line.startsWith("while(") ||
    line.startsWith("switch ") ||
    line.startsWith("switch(") ||
    line.startsWith("try ") ||
    line.startsWith("try{") ||
    line.startsWith("const ") ||
    line.startsWith("let ") ||
    line.startsWith("var ") ||
    line.startsWith("//") ||
    line.startsWith("/*") ||
    line.startsWith("await ") ||
    line.endsWith(";") ||
    line.endsWith("}")
  );
}
