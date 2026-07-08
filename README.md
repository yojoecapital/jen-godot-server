# jen-godot-server

Authoritative dedicated server for [Jen](https://github.com/yojoecapital/jen-godot).

This is a headless Godot 4.6 application that serves as the single source of truth for online matches. Clients send player input over WebSockets, the server validates and applies that input using the shared simulation rules ([jen-godot-simulation](https://github.com/yojoecapital/jen-godot-simulation), included as a submodule), drives CPU-controlled players, and streams the resulting authoritative action log back to connected clients. Match state and server data are persisted as JSON files.

## Project layout

```text
project.godot              # Headless project (main scene: server/server_root.tscn)
simulation/                # Submodule: shared simulation, AI, serialization

server/
  server_root.gd/.tscn     # Boot: initialize storage, start admin API and WebSocket server
  db.gd                    # JSON file persistence (API keys, matches)
  auth.gd                  # Secret generation, hashing, scope validation
  admin_api.gd             # Admin API handlers
  admin_http_server.gd     # Minimal HTTP/1.1 server over TCPServer
  selftest.gd              # Headless integration tests

export_presets.cfg         # Dedicated Linux Server export preset
Dockerfile
```

## Data storage

The server stores persistent data as JSON files under `/data/`.

```text
/data/
  keys.json                # API keys and permissions
  matches/
    AB12.json              # Individual match state
    CD34.json
```

Each match is stored independently, allowing match state to be loaded, updated, and removed without affecting other matches.

## Configuration

| Variable           | Required | Default | Description                                     |
| ------------------ | -------- | ------- | ----------------------------------------------- |
| `ADMIN_API_SECRET` | **Yes**  | —       | Bearer token used to authenticate the admin API |
| `DATA_PATH`        | No       | `/data` | Directory for JSON persistence files            |
| `ADMIN_PORT`       | No       | `8080`  | Admin API port                                  |
| `WS_PORT`          | No       | `8081`  | Gameplay WebSocket port                         |

## Running with Docker

```bash
git clone --recurse-submodules git@github.com:yojoecapital/jen-godot-server.git
cd jen-godot-server

docker build -t jen-server .

docker run -d \
  --name jen-server \
  -e ADMIN_API_SECRET="keep_your_head_up" \
  -v jen-data:/data \
  -p 8080:8080 \
  -p 8081:8081 \
  jen-server
```

The `jen-data` volume persists `/data/keys.json` and `/data/matches/` across container restarts.

The Docker image pins the Godot version via:

```text
--build-arg GODOT_VERSION=4.6.2
```

## Admin API

All endpoints require:

```text
Authorization: Bearer $ADMIN_API_SECRET
```

Example usage:

```bash
S=http://localhost:8080
H="Authorization: Bearer $ADMIN_API_SECRET"

# Create a host key (can also join matches).
# The secret is returned only once.
curl -H "$H" \
  -d '{"id":"alice","scopes":["host_match","join_match"]}' \
  $S/admin/keys

# Create a join-only key.
curl -H "$H" \
  -d '{"id":"bob","scopes":["join_match"]}' \
  $S/admin/keys

# List keys (secrets are never returned).
curl -H "$H" $S/admin/keys

# Revoke a key.
curl -H "$H" -X DELETE $S/admin/keys/alice

# List matches.
curl -H "$H" $S/admin/matches

# Delete a match.
curl -H "$H" -X DELETE $S/admin/matches/AB12
```

Provide players with the returned `id` and `secret`, along with the server's WebSocket URL (for example, `ws://host:8081`). They can enter these under **Settings → Online** in the game client.

Permission scopes:

* `host_match` — create matches, join matches, and delete matches that the key created.
* `join_match` — join existing matches only.

## Local development

```bash
git submodule update --init --recursive

# Run the headless integration tests (JSON storage, routing, auth, HTTP parsing).
godot --headless --path . -s res://server/selftest.gd

# Run the server locally.
ADMIN_API_SECRET=dev \
godot --headless --path . res://server/server_root.gd
```

## Deployment

```bash
# If needed
docker buildx create --name multiarch --use
docker buildx inspect --bootstrap
docker buildx ls

# Push
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t ghcr.io/yojoecapital/$VERSION \
  --push .
```

