# Multi-stage build for minimal production image
# Usage:
#   docker build -t sandcastle .
#   docker run -p 8080:8080 sandcastle serve --http 0.0.0.0:8080

# --- Build stage ---
FROM rust:1.93-bookworm AS builder

RUN rustup target add wasm32-wasip1

WORKDIR /build
COPY . .

# Build guest WASM module
RUN cd guest && cargo build --target wasm32-wasip1 --release

# Build host binary
RUN cargo build --release --bin sandcastle

# --- Runtime stage ---
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/sandcastle /usr/local/bin/sandcastle
COPY --from=builder /build/guest/target/wasm32-wasip1/release/sandcastle_guest_js.wasm /usr/local/share/sandcastle/guest-js.wasm

ENV SANDCASTLE_GUEST_MODULE=/usr/local/share/sandcastle/guest-js.wasm

EXPOSE 8080

ENTRYPOINT ["sandcastle"]
CMD ["serve", "--http", "0.0.0.0:8080"]
