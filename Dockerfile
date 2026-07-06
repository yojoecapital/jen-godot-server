# syntax=docker/dockerfile:1
# Multi-stage build for the Jen dedicated server.
#   Stage 1 downloads Godot + export templates and exports the dedicated-server binary.
#   Stage 2 is a slim runtime carrying only the binary, its embedded pck, and the
#   godot-sqlite native library.
ARG GODOT_VERSION=4.6.2

# ---------- build ----------
FROM debian:bookworm-slim AS build
ARG GODOT_VERSION
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl unzip && rm -rf /var/lib/apt/lists/*
WORKDIR /godot
RUN base="https://github.com/godotengine/godot-builds/releases/download/${GODOT_VERSION}-stable" \
 && curl -fsSL -o godot.zip "${base}/Godot_v${GODOT_VERSION}-stable_linux.x86_64.zip" \
 && curl -fsSL -o templates.tpz "${base}/Godot_v${GODOT_VERSION}-stable_export_templates.tpz" \
 && unzip -q godot.zip && mv Godot_v${GODOT_VERSION}-stable_linux.x86_64 /usr/local/bin/godot \
 && chmod +x /usr/local/bin/godot \
 && mkdir -p "/root/.local/share/godot/export_templates/${GODOT_VERSION}.stable" \
 && unzip -q templates.tpz -d /tmp/tpl \
 && mv /tmp/tpl/templates/* "/root/.local/share/godot/export_templates/${GODOT_VERSION}.stable/"
WORKDIR /src
COPY . .
RUN godot --headless --path . --import >/dev/null 2>&1 || true \
 && mkdir -p build \
 && godot --headless --path . --export-release "Linux Server" build/jen-server.x86_64
RUN test -f build/jen-server.x86_64

# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libgl1 && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/build/ /app/
ENV DB_PATH=/data/jen.db \
    ADMIN_PORT=8080 \
    WS_PORT=8081
VOLUME ["/data"]
EXPOSE 8080 8081
# ADMIN_API_SECRET must be provided at run time (docker run -e ADMIN_API_SECRET=...).
ENTRYPOINT ["/app/jen-server.x86_64", "--headless"]
