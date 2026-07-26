//! Jen dedicated server (pure Rust). Source of truth for authoritative online play.
//!
//! Two listeners: REST + management UI on `ADMIN_PORT` (8080) and the WebSocket gameplay gateway on
//! `WS_PORT` (8081) — the client's `server_url` is the `ws://host:8081` URL. Persistence is SQLite
//! at `DB_PATH`. On boot `ADMIN_API_SECRET` (when set) seeds an `admin`-scoped key.

mod auth;
mod db;
mod registry;
mod rest;
mod version;
mod ws;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;

use crate::db::Db;
use crate::registry::Registry;
use crate::ws::Hub;

/// Shared application state handed to both the REST router and the WS gateway.
pub struct AppState {
    pub db: Arc<Db>,
    pub registry: Arc<Registry>,
    pub hub: Arc<Hub>,
}

#[tokio::main]
async fn main() {
    let admin_secret = env("ADMIN_API_SECRET", "");
    let db_path = env("DB_PATH", "/data/jen.db");
    let admin_port: u16 = env("ADMIN_PORT", "8080").parse().unwrap_or(8080);
    let ws_port: u16 = env("WS_PORT", "8081").parse().unwrap_or(8081);

    ensure_parent_dir(&db_path);
    let db = Arc::new(Db::open(&db_path).unwrap_or_else(|e| panic!("open db at {db_path}: {e}")));
    println!("[jen-server] db ready at {db_path}");

    if admin_secret.is_empty() {
        eprintln!(
            "[jen-server] ADMIN_API_SECRET is not set — key management is disabled \
             (existing keys still authenticate)."
        );
    } else {
        db.upsert_key(
            "admin",
            &auth::hash_secret(&admin_secret),
            &[auth::ADMIN.into(), auth::HOST_MATCH.into(), auth::JOIN_MATCH.into()],
        );
        println!("[jen-server] admin key seeded from ADMIN_API_SECRET");
    }

    let state = Arc::new(AppState {
        db: db.clone(),
        registry: Arc::new(Registry::new(db)),
        hub: Arc::new(Hub::new()),
    });

    let rest_app = rest::router(state.clone());
    let ws_app = Router::new().route("/", get(ws::handler)).with_state(state.clone());

    let rest_listener = bind(admin_port).await;
    let ws_listener = bind(ws_port).await;
    println!("[jen-server] REST + UI on :{admin_port}   WS gameplay on :{ws_port}");

    tokio::select! {
        r = axum::serve(rest_listener, rest_app) => { r.expect("rest server"); }
        r = axum::serve(ws_listener, ws_app) => { r.expect("ws server"); }
    }
}

async fn bind(port: u16) -> TcpListener {
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| panic!("could not bind port {port}: {e}"))
}

fn ensure_parent_dir(path: &str) {
    if let Some(parent) = std::path::Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
}

fn env(key: &str, fallback: &str) -> String {
    std::env::var(key).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| fallback.to_string())
}
