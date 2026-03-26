.PHONY: build build-guest build-host test test-cli bench clean fmt clippy publish-sdk

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

# Run CLI integration tests (requires release build)
test-cli: build
	bash tests/cli_integration.sh

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

# Publish TypeScript SDK to npm (bumps patch version automatically)
# Usage: make publish-sdk [V=minor|major|patch]
publish-sdk:
	@cd sdk/typescript && \
		bun install --frozen-lockfile && \
		bun run build && \
		npm version $(or $(V),patch) --no-git-tag-version && \
		echo "//registry.npmjs.org/:_authToken=$${NPM_TOKEN}" > .npmrc && \
		npm publish --access public --ignore-scripts && \
		rm -f .npmrc && \
		echo "Published $$(node -p "require('./package.json').name + '@' + require('./package.json').version")"
