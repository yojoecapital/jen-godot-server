# jen-godot-server

Authoritative dedicated server for [Jen](https://github.com/yojoecapital/jen-godot).

Clients send player input over WebSockets. The server validates and applies it with the shared rules engine, then broadcasts the authoritative action stream back to every client. Both sides run the same RNG, so combat replays identically and the client never recomputes an outcome.

Matches can seat CPU players. Pass `ai` in `seat_controllers` and the server plays that seat with `core/jen_ai`, the same crate the client links. A match with no human seat is refused.

## Requirements

- Rust 1.85 or newer
- A C toolchain, for the bundled SQLite
- Docker, only for the container build

## Installation

```bash
git clone --recurse-submodules git@github.com:yojoecapital/jen-godot-server.git
cd jen-godot-server
```

## Submodules

```bash
git submodule update --init --recursive
```

## Configuration

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `ADMIN_API_SECRET` | No | none | Seeds the `admin` key on boot. Without it, key management is disabled and existing keys still work. |
| `DB_PATH` | No | `/data/jen.db` | SQLite file. The parent directory is created. |
| `ADMIN_PORT` | No | `8080` | REST API and management UI |
| `WS_PORT` | No | `8081` | Gameplay WebSocket |

The admin is an API key holding the `admin` scope, not a separate credential. Every `/api/*` call and every WebSocket `auth` presents a key secret, of which only the SHA-256 is stored.

| Scope | Grants |
| --- | --- |
| `admin` | Mint and revoke client keys, view and delete every match |
| `host_match` | Create matches, join, delete matches the key created |
| `join_match` | Join existing matches only |

## Build

```bash
cargo build --release -p jen-server
```

## Usage

```bash
ADMIN_API_SECRET=dev DB_PATH=./jen.db cargo run -p jen-server
```

Open `http://localhost:8080` and sign in with a key id and secret. Admin keys also get a Clients panel for minting and revoking keys.

Every endpoint takes `Authorization: Bearer <key-secret>`. Mint a client key, whose secret is returned once:

```bash
curl -H "Authorization: Bearer dev" \
  -d '{"id":"alice","scopes":["host_match","join_match"]}' \
  http://localhost:8080/api/keys
```

List matches, which returns every match for an admin key and only your own otherwise:

```bash
curl -H "Authorization: Bearer dev" http://localhost:8080/api/matches
```

Give players their key id and secret plus the WebSocket URL, `ws://host:8081`. They enter these under Settings, Online in the game client.

Client and server must report the same version at the handshake, or the connection is refused. Read the server's version without a key:

```bash
curl http://localhost:8080/api/version
```

## Testing

The workspace run covers the server, the shared rules engine, and the AI. Use `--release`, as the AI's search tests are far slower unoptimised.

```bash
cargo test --release
```

To check that client and server consume the shared RNG identically, generate a determinism vector:

```bash
cargo run -p jen-server --example genvector
```

It prints `snapshot0`, `actions`, `final` and `combat`. Load `snapshot0` in the client, replay `actions`, and compare against `final`. A vector whose `combat` count is zero proves nothing.

## Deployment

```bash
docker build -t jen-server .
```

```bash
docker run -d --name jen-server \
  -e ADMIN_API_SECRET="change-me" \
  -v jen-data:/data \
  -p 8080:8080 -p 8081:8081 \
  jen-server
```

Multi-arch images:

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  -t ghcr.io/yojoecapital/jen-server:$VERSION \
  --push .
```

## Project structure

```text
core/      submodule: shared rules engine and AI
server/    the binary: DB, auth, match registry, REST, WebSocket, embedded UI
Dockerfile
```
