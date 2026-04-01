# Troubleshooting

## Bun: "Worker not found" or similar

Make sure you're on Bun 1.0+. SandCastle uses Bun's built-in Worker API.

```bash
bun --version  # should be 1.0+
```

## Node.js: "isolated-vm is required"

The Node.js backend requires `isolated-vm`:

```bash
npm install isolated-vm
```

If installation fails (C++ compilation error), make sure you have build tools:

```bash
# macOS
xcode-select --install

# Ubuntu/Debian
sudo apt-get install python3 make g++
```

If you can't get it to compile, use Bun instead (zero dependencies).

## Timeout errors on simple code

Default timeout is 10 seconds. Increase if needed:

```ts
const sc = new SandCastle({
  defaults: { timeoutMs: 30_000 },
});
```

## Memory errors

Default memory limit is 128MB. Increase if needed:

```ts
const sc = new SandCastle({
  defaults: { memoryMb: 512 },
});
```

## Subprocess mode: binary not found

Subprocess mode (`mode: "subprocess"`) requires the `sandcastle` CLI binary. This is **not needed for the default mode** — only if you explicitly opt in.

```bash
# Build from source
cargo install --path crates/sandcastle-cli
```

## HTTP mode: connection refused

Make sure the server is running:

```bash
npx sandcastle serve --http 0.0.0.0:8080
```
