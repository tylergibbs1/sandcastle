# TypeScript Quick Start

This is the fastest path for most developers.

## Install

```bash
npm install @grayhaven/sandcastle
```

The package will try to download the `sandcastle` CLI automatically on macOS and Linux.

## Run your first sandbox

```ts
import { SandCastle } from "@grayhaven/sandcastle";

const sc = new SandCastle();
const result = await sc.run<number>("return 1 + 1;");

console.log(result); // 2
```

## Pass input

```ts
const greeting = await sc.run<string>(
  'return "Hello " + input.name;',
  { name: "World" },
);

console.log(greeting); // Hello World
```

## Verify the CLI wrapper

```bash
npx sandcastle --help
```

If that fails, see [Troubleshooting](troubleshooting.md).

## Mental model

- The TypeScript SDK is the primary interface.
- Under the hood, it talks to the `sandcastle` CLI by default.
- You can switch to HTTP mode later if you want a long-running server.
