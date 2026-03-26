.PHONY: build build-guest build-host test bench clean fmt clippy

# Build everything
build: build-guest build-host

# Build the guest WASM module
build-guest:
	cd guest && ./build.sh

# Build host crates
build-host:
	cargo build --release

# Run tests
test:
	cargo test

# Run benchmarks
bench:
	cargo bench

# Format code
fmt:
	cargo fmt --all
	cd guest && cargo fmt

# Lint
clippy:
	cargo clippy --all -- -D warnings
	cd guest && cargo clippy --target wasm32-wasip1 -- -D warnings

# Clean all build artifacts
clean:
	cargo clean
	cd guest && cargo clean

# Run an example script
run-example: build
	cargo run --release --bin sandcastle -- run examples/hello.js
