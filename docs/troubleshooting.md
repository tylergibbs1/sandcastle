# Troubleshooting

## `BinaryNotFoundError`

This means the TypeScript SDK could not find a working `sandcastle` CLI binary.

Try this first:

```bash
npx sandcastle --help
```

If that still fails:

```bash
cargo install --path crates/sandcastle-cli
```

Or download a release from:

`https://github.com/tylergibbs1/sandcastle/releases`

## Automatic binary download did not happen

Common reasons:

- Your platform does not have a prebuilt binary.
- The install ran offline.
- `postinstall` was disabled or skipped.

Current automatic download support:

- macOS
- Linux

## Unsupported platform

Today the automatic binary path is focused on macOS and Linux. On other platforms, install from source with Rust.

## Verify your setup

Check the wrapper:

```bash
npx sandcastle --help
```

Check the native runtime once it is installed:

```bash
sandcastle doctor
```

Check the SDK:

```ts
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
await sc.run("return 1 + 1;");
```

Get an explicit diagnosis:

```ts
import { diagnoseInstallation } from "@grayhaven/sandcastle";

console.log(await diagnoseInstallation());
```
