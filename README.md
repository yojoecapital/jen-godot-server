# jen-godot-server

Authoritative dedicated server for [Jen](https://github.com/yojoecapital/jen-godot).

A **pure-Rust** binary that is the single source of truth for online matches. Clients send player
input over WebSockets; the server validates and applies it with the shared Rust rules engine
[`jen_core`](https://github.com/yojoecapital/jen-godot-simulation) (the `rust-core` branch, included
as the `core/` submodule), then broadcasts the authoritative action stream back to every client.
Both sides run the same `jen_core::Pcg32` RNG, so combat replays deterministically from a shared
`rng_state` — the client never recomputes an outcome. Persistence is **SQLite**.

> **Matches are human-only in this build.** The Rust core carries no AI, so the server rejects
> `cpu`/`ai` seats (`{"t":"error","message":"cpu_seats_unsupported"}`). CPU opponents are a follow-up.

## Project layout

```text
Cargo.toml                 # workspace: members = ["core", "server"]
core/                      # submodule -> jen-godot-simulation @ rust-core (the jen_core crate)
server/
  Cargo.toml
  src/
    main.rs                # env, open DB, seed admin key, spawn REST+UI (8080) and WS (8081)
    db.rs                  # SQLite (rusqlite, bundled): api_keys + matches
    auth.rs                # secret gen / SHA-256 hash / scopes
    registry.rs            # authoritative match logic (human-only) over jen_core
    rest.rs                # /api/* (bearer = API key) + embedded UI
    ws.rs                  # WebSocket gameplay gateway (net_client.gd protocol)
    ui/                    # index.html / app.js / style.css (embedded via include_str!)
  examples/
    genvector.rs           # cross-runtime determinism vector (see below)
Dockerfile
```

## Unified auth

There is no separate admin secret anymore. **The admin is just an API key with the `admin` scope**,
seeded from `ADMIN_API_SECRET` on boot. Every `/api/*` call and every WS `auth` presents a key
secret; only its SHA-256 is stored.

| Scope        | Grants                                                        |
| ------------ | ------------------------------------------------------------- |
| `admin`      | Mint/revoke client keys, and view/delete **every** match      |
| `host_match` | Create matches, join, and delete matches the key created      |
| `join_match` | Join existing matches only                                    |

## Configuration

| Variable           | Required | Default        | Description                                              |
| ------------------ | -------- | -------------- | -------------------------------------------------------- |
| `ADMIN_API_SECRET` | No\*     | —              | Seeds/refreshes the `admin` key on boot                  |
| `DB_PATH`          | No       | `/data/jen.db` | SQLite database file (parent dir is created)             |
| `ADMIN_PORT`       | No       | `8080`         | REST API + management UI                                 |
| `WS_PORT`          | No       | `8081`         | Gameplay WebSocket                                       |

\* Without it, key management is disabled, but any previously issued key still authenticates.

## REST API + web UI

Open `http://host:8080` and sign in with a key `id` + secret. Admin keys get a **Clients** panel
(list/create/revoke keys) plus **Matches** (all matches); other keys see only their own matches.

All endpoints take `Authorization: Bearer <key-secret>`:

```bash
S=http://localhost:8080
A="Authorization: Bearer $ADMIN_API_SECRET"

curl -H "$A" $S/api/me                                   # { id, scopes }

# Mint a client key (admin only). The secret is returned once.
curl -H "$A" -d '{"id":"alice","scopes":["host_match","join_match"]}' $S/api/keys
curl -H "$A" $S/api/keys                                 # list (no secrets)
curl -H "$A" -X DELETE $S/api/keys/alice                 # revoke

curl -H "$A" $S/api/matches                              # admin: all; else own
curl -H "$A" -X DELETE $S/api/matches/AB12               # admin or owner
```

Give players their `id` + `secret` and the WebSocket URL (`ws://host:8081`); they enter these under
**Settings → Online** in the game client.

## Local development

```bash
git submodule update --init --recursive

# Tests (SQLite CRUD, auth scopes, human-only rejection, turn/actor validation,
# deterministic replay, persistence + rehydrate).
cargo test

# Run.
ADMIN_API_SECRET=dev DB_PATH=./jen.db cargo run -p jen-server
```

### Proving client/server determinism

`cargo run -p jen-server --example genvector` plays a recorded self-play game and prints
`{snapshot0, actions, final, combat}`. Loading `snapshot0` in the Godot client, replaying `actions`
through its own `jen_core`, and comparing against `final` shows both runtimes consume the shared
`Pcg32` identically — `combat` counts the RNG-consuming events, so a vector that exercises no combat
proves nothing.

## Running with Docker

```bash
git clone --recurse-submodules git@github.com:yojoecapital/jen-godot-server.git
cd jen-godot-server

docker build -t jen-server .
docker run -d --name jen-server \
  -e ADMIN_API_SECRET="keep_your_head_up" \
  -v jen-data:/data \
  -p 8080:8080 -p 8081:8081 \
  jen-server
```

## Deployment

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t ghcr.io/yojoecapital/jen-server:$VERSION \
  --push .
```
