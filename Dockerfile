# syntax=docker/dockerfile:1

# Multi-stage build for the Jen dedicated server.
#
# Supports:
#   linux/amd64  -> Linux Server preset
#   linux/arm64  -> Linux Server ARM64 preset
#
# Stage 1:
#   Downloads the matching Godot editor + export templates and builds the server.
#
# Stage 2:
#   Runs only the exported dedicated-server binary.

ARG GODOT_VERSION=4.6.2


# ---------- build ----------
FROM debian:bookworm-slim AS build

ARG GODOT_VERSION
ARG TARGETARCH

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        unzip \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /godot


RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) \
            GODOT_ARCH="x86_64"; \
            ;; \
        arm64) \
            GODOT_ARCH="arm64"; \
            ;; \
        *) \
            echo "Unsupported architecture: ${TARGETARCH}"; \
            exit 1; \
            ;; \
    esac; \
    BASE="https://github.com/godotengine/godot-builds/releases/download/${GODOT_VERSION}-stable"; \
    curl -fsSL -o godot.zip \
        "${BASE}/Godot_v${GODOT_VERSION}-stable_linux.${GODOT_ARCH}.zip"; \
    curl -fsSL -o templates.tpz \
        "${BASE}/Godot_v${GODOT_VERSION}-stable_export_templates.tpz"; \
    unzip -q godot.zip; \
    mv Godot_v${GODOT_VERSION}-stable_linux.${GODOT_ARCH} /usr/local/bin/godot; \
    chmod +x /usr/local/bin/godot; \
    mkdir -p "/root/.local/share/godot/export_templates/${GODOT_VERSION}.stable"; \
    unzip -q templates.tpz -d /tmp/templates; \
    mv /tmp/templates/templates/* \
        "/root/.local/share/godot/export_templates/${GODOT_VERSION}.stable/"


WORKDIR /src

COPY . .


RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) \
            EXPORT_PRESET="Linux Server"; \
            OUTPUT="build/jen-server.x86_64"; \
            ;; \
        arm64) \
            EXPORT_PRESET="Linux Server ARM64"; \
            OUTPUT="build/jen-server.arm64"; \
            ;; \
    esac; \
    godot --headless --path . --import >/dev/null 2>&1 || true; \
    mkdir -p build; \
    godot --headless \
        --path . \
        --export-release "${EXPORT_PRESET}" "${OUTPUT}"; \
    test -f "${OUTPUT}"


# ---------- runtime ----------
FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates \
        libgl1 \
    && rm -rf /var/lib/apt/lists/*


WORKDIR /app

COPY --from=build /src/build/ /app/


ENV DATA_PATH=/data \
    ADMIN_PORT=8080 \
    WS_PORT=8081


VOLUME ["/data"]

EXPOSE 8080 8081


# The correct binary is selected by the build stage.
ARG TARGETARCH

RUN set -eux; \
    case "${TARGETARCH}" in \
        amd64) mv /app/jen-server.x86_64 /app/jen-server ;; \
        arm64) mv /app/jen-server.arm64 /app/jen-server ;; \
    esac; \
    chmod +x /app/jen-server


# ADMIN_API_SECRET must be provided at runtime.
ENTRYPOINT ["/app/jen-server", "--headless"]