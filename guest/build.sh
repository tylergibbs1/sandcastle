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
