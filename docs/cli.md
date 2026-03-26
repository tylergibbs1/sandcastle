# CLI Guide

## Install

If you already installed `@grayhaven/sandcastle`, the wrapper is available through `npx`:

```bash
npx sandcastle --help
```

To install the CLI directly from source:

```bash
cargo install --path crates/sandcastle-cli
```

## Common commands

Run a script:

```bash
npx sandcastle run script.js --input '{"name": "Alice"}'
```

Initialize a project:

```bash
npx sandcastle init
```

Start the HTTP server:

```bash
npx sandcastle serve --watch ./scripts
```

Diagnose the native runtime:

```bash
npx sandcastle doctor
```

Open the REPL:

```bash
npx sandcastle repl
```

Generate guest declarations:

```bash
npx sandcastle codegen capabilities.json -o types.d.ts
```
