# syntax=docker/dockerfile:1

# Multi-stage build for the Jen dedicated server (pure Rust).
#
# Multi-arch: `docker buildx build --platform linux/amd64,linux/arm64`.
# rusqlite's `bundled` feature compiles SQLite from source per target arch, so no host SQLite is
# needed at build or run time.

# ---------- build ----------
FROM rust:1-slim AS build

# SQLite is bundled (compiled from C), so the builder needs a C toolchain.
RUN apt-get update && apt-get install -y --no-install-recommends \
        gcc libc6-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /src

# The workspace expects the shared jen_core crate at ./core (the submodule). Ensure it is present:
#   git submodule update --init --recursive
COPY . .

RUN cargo build --release -p jen-server \
    && strip target/release/jen-server

# ---------- runtime ----------
FROM debian:stable-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /src/target/release/jen-server /app/jen-server

ENV DB_PATH=/data/jen.db \
    ADMIN_PORT=8080 \
    WS_PORT=8081

VOLUME ["/data"]
EXPOSE 8080 8081

# Provide ADMIN_API_SECRET at runtime to enable key management + seed the admin key.
ENTRYPOINT ["/app/jen-server"]
