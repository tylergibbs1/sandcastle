#!/usr/bin/env bash
set -euo pipefail

# Build the SandCastle guest JS runtime to WASM
# Requires: rustup target add wasm32-wasip1

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "Building SandCastle guest JS runtime..."

# Ensure the target is installed
if ! rustup target list --installed | grep -q wasm32-wasip1; then
    echo "Installing wasm32-wasip1 target..."
    rustup target add wasm32-wasip1
fi

# Build in release mode
cargo build --target wasm32-wasip1 --release

WASM_PATH="target/wasm32-wasip1/release/sandcastle_guest_js.wasm"

if [ -f "$WASM_PATH" ]; then
    SIZE=$(wc -c < "$WASM_PATH" | tr -d ' ')
    echo "Built: $WASM_PATH ($SIZE bytes)"
else
    echo "ERROR: Build completed but WASM file not found at $WASM_PATH"
    exit 1
fi

# Pre-initialize with wizer if available (bakes QuickJS Runtime + polyfills
# into the WASM module's linear memory, saving ~1-2ms per evaluate call).
if command -v wizer &> /dev/null; then
    echo "Running wizer pre-initialization..."
    WIZER_OUT="${WASM_PATH%.wasm}.wized.wasm"
    if wizer --allow-wasi --wasm-bulk-memory true --init-func wizer_initialize "$WASM_PATH" -o "$WIZER_OUT" 2>&1; then
        mv "$WIZER_OUT" "$WASM_PATH"
        WIZER_SIZE=$(wc -c < "$WASM_PATH" | tr -d ' ')
        echo "Wizer pre-initialized: $WASM_PATH ($WIZER_SIZE bytes)"
    else
        rm -f "$WIZER_OUT"
        echo "WARNING: wizer pre-initialization failed, using non-pre-initialized module"
    fi
else
    echo "Note: wizer not found, skipping pre-initialization (install with: cargo install wizer --all-features)"
fi
