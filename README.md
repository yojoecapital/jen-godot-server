# jen-godot-server

Authoritative dedicated server for [Jen](https://github.com/yojoecapital/jen-godot) — a headless
Godot 4.6 build that is the single source of truth for online matches. Clients send input over
WebSockets; the server validates and applies it against the shared rules
([jen-godot-simulation](https://github.com/yojoecapital/jen-godot-simulation), a submodule),
drives CPU seats, and streams the authoritative action stream back. State is persisted in SQLite.

## Layout

```
project.godot              # headless project; main scene = server/server_root.tscn
simulation/                # submodule: shared rules + AI + serialization
addons/godot-sqlite/       # SQLite GDExtension (Linux x86_64)
server/
  server_root.gd/.tscn     # boot: open DB, start admin REST + (Phase 2) WS gateway
  db.gd                    # SQLite: api_keys + matches
  auth.gd                  # secret generation/hashing, scope checks
  admin_api.gd             # admin REST handlers
  admin_http_server.gd     # minimal polled HTTP/1.1 over TCPServer
  selftest.gd              # headless self-test of the admin layer
export_presets.cfg         # "Linux Server" dedicated-server preset
Dockerfile
```

## Configuration (environment variables)

| Var | Required | Default | Purpose |
|---|---|---|---|
| `ADMIN_API_SECRET` | **yes** | — | Bearer token for the admin REST API |
| `DB_PATH` | no | `user://jen.db` (`/data/jen.db` in Docker) | SQLite file |
| `ADMIN_PORT` | no | `8080` | Admin REST port |
| `WS_PORT` | no | `8081` | Gameplay WebSocket port |

## Run with Docker

```bash
git clone --recurse-submodules git@github.com:yojoecapital/jen-godot-server.git
cd jen-godot-server
docker build -t jen-server .
docker run -d --name jen-server \
  -e ADMIN_API_SECRET="$(openssl rand -hex 24)" \
  -v jen-data:/data \
  -p 8080:8080 -p 8081:8081 \
  jen-server
```

`-v jen-data:/data` persists `/data/jen.db` (API keys + match snapshots) across restarts. The
Docker image pins the Godot version via `--build-arg GODOT_VERSION=4.6.2`.

## Admin API

All requests require `Authorization: Bearer $ADMIN_API_SECRET`.

```bash
S=http://localhost:8080; H="Authorization: Bearer $ADMIN_API_SECRET"

# create a client key that may host (and join) matches — the secret is shown once
curl -H "$H" -d '{"id":"alice","scopes":["host_match","join_match"]}' $S/admin/keys
# create a join-only key
curl -H "$H" -d '{"id":"bob","scopes":["join_match"]}' $S/admin/keys

curl -H "$H" $S/admin/keys                 # list keys (no secrets)
curl -H "$H" -X DELETE $S/admin/keys/alice # revoke a key
curl -H "$H" $S/admin/matches              # list matches
curl -H "$H" -X DELETE $S/admin/matches/AB12  # delete any match
```

Give the returned `id` + `secret` and the server's WebSocket URL (`ws://host:8081`) to a player;
they enter them under **Settings → Online** in the game client. `host_match` keys can create
matches (and delete the ones they created); `join_match` keys can only join.

## Local development

```bash
git submodule update --init --recursive
# self-test the admin layer (SQLite + routing + auth + HTTP parsing), no networking:
godot --headless --path . -s res://server/selftest.gd
# run the server locally:
ADMIN_API_SECRET=dev godot --headless --path . res://server/server_root.tscn
```
