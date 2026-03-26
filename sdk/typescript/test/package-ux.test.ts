import { describe, expect, it } from "bun:test";

const BIN_WRAPPER = new URL("../bin/sandcastle.cjs", import.meta.url).pathname;
const POSTINSTALL = new URL("../scripts/postinstall.cjs", import.meta.url).pathname;
const PACKAGE_JSON = new URL("../package.json", import.meta.url).pathname;

describe("package UX", () => {
  it("points package.json to CJS wrapper scripts", async () => {
    const pkg = await Bun.file(PACKAGE_JSON).json();
    expect(pkg.bin.sandcastle).toBe("./bin/sandcastle.cjs");
    expect(pkg.scripts.postinstall).toBe("node scripts/postinstall.cjs || true");
  });

  it("bin wrapper runs under Node and prints actionable guidance", () => {
    const proc = Bun.spawnSync({
      cmd: ["node", BIN_WRAPPER, "--help"],
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(proc.exitCode).toBe(1);
    const output = new TextDecoder().decode(proc.stderr);
    expect(output).toContain("binary not found");
    expect(output).toContain("install from source");
  });

  it("postinstall script runs under Node without ESM/CJS loader failure", () => {
    const proc = Bun.spawnSync({
      cmd: ["node", POSTINSTALL],
      stdout: "pipe",
      stderr: "pipe",
    });

    expect(proc.exitCode).toBe(0);
    const combined =
      new TextDecoder().decode(proc.stdout) + new TextDecoder().decode(proc.stderr);
    expect(combined).not.toContain("require is not defined");
    expect(combined.length).toBeGreaterThan(0);
  });
});
