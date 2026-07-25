//! WebSocket gameplay gateway (port of `ws_gateway.gd`).
//!
//! The wire protocol is byte-for-byte the one `autoload/net_client.gd` speaks — client sends
//! `auth/create_match/join_match/list_matches/delete_match/action/leave/save`; the server replies
//! `hello/match_start/matches/action/game_over/room_closed/error`. Field names (`yourSeat`, `seats`,
//! `snapshot`, `seed`, `state_hash`, …) are preserved exactly.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::auth;
use crate::AppState;

/// Per-connection session. `code`/`seat` are visible to other connections (seat claiming +
/// broadcast); the rest is owned by the connection.
struct Session {
    tx: mpsc::UnboundedSender<Message>,
    authed: bool,
    key_id: String,
    scopes: Vec<String>,
    code: String,
    seat: i32,
}

pub struct Hub {
    inner: Mutex<HashMap<u64, Session>>,
    next_id: AtomicU64,
}

impl Hub {
    pub fn new() -> Hub {
        Hub {
            inner: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn register(&self, tx: mpsc::UnboundedSender<Message>) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.inner.lock().unwrap().insert(
            id,
            Session {
                tx,
                authed: false,
                key_id: String::new(),
                scopes: Vec::new(),
                code: String::new(),
                seat: -1,
            },
        );
        id
    }

    fn unregister(&self, id: u64) {
        self.inner.lock().unwrap().remove(&id);
    }

    fn send(&self, id: u64, msg: &Value) {
        let inner = self.inner.lock().unwrap();
        if let Some(s) = inner.get(&id) {
            let _ = s.tx.send(Message::Text(msg.to_string()));
        }
    }

    fn broadcast(&self, code: &str, msg: &Value) {
        let inner = self.inner.lock().unwrap();
        let text = msg.to_string();
        for s in inner.values() {
            if s.code == code {
                let _ = s.tx.send(Message::Text(text.clone()));
            }
        }
    }

    /// Lowest human seat not already held by another live session in this match.
    fn claim_seat(&self, code: &str, self_id: u64, human_seats: &[usize]) -> i32 {
        let inner = self.inner.lock().unwrap();
        let mut claimed: HashSet<usize> = HashSet::new();
        for (id, s) in inner.iter() {
            if *id != self_id && s.code == code && s.seat >= 0 {
                claimed.insert(s.seat as usize);
            }
        }
        for &seat in human_seats {
            if !claimed.contains(&seat) {
                return seat as i32;
            }
        }
        -1
    }

    fn claimed_count(&self, code: &str) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.values().filter(|s| s.code == code && s.seat >= 0).count()
    }

    fn detach_code(&self, code: &str) {
        let mut inner = self.inner.lock().unwrap();
        for s in inner.values_mut() {
            if s.code == code {
                s.code = String::new();
                s.seat = -1;
            }
        }
    }
}

pub async fn handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> Response {
    ws.on_upgrade(move |socket| serve_socket(socket, state))
}

async fn serve_socket(socket: WebSocket, state: Arc<AppState>) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let id = state.hub.register(tx);

    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            incoming = receiver.next() => match incoming {
                Some(Ok(Message::Text(text))) => handle_text(&state, id, &text),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
            _ = &mut send_task => break,
        }
    }

    state.hub.unregister(id);
    send_task.abort();
}

fn handle_text(state: &Arc<AppState>, id: u64, text: &str) {
    let msg: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return,
    };
    let t = msg.get("t").and_then(Value::as_str).unwrap_or("");

    // Everything but `auth` is gated behind a successful handshake.
    let (authed, scopes, code, seat, key_id) = {
        let inner = state.hub.inner.lock().unwrap();
        let s = match inner.get(&id) {
            Some(s) => s,
            None => return,
        };
        (s.authed, s.scopes.clone(), s.code.clone(), s.seat, s.key_id.clone())
    };

    if !authed {
        if t == "auth" {
            authenticate(state, id, &msg);
        }
        return;
    }

    match t {
        "create_match" => create_match(state, id, &scopes, &key_id, &msg),
        "join_match" => {
            let code = msg.get("code").and_then(Value::as_str).unwrap_or("").to_uppercase();
            join_match(state, id, &code);
        }
        "list_matches" => {
            let list = joinable_list(state);
            state.hub.send(id, &json!({ "t": "matches", "matches": list }));
        }
        "delete_match" => {
            let code = msg.get("code").and_then(Value::as_str).unwrap_or("").to_uppercase();
            delete_match(state, id, &key_id, &code);
        }
        "action" => action(state, id, &code, seat, msg.get("action").cloned().unwrap_or(Value::Null)),
        "leave" => detach(state, id),
        "save" => state.hub.send(id, &json!({ "t": "saved", "code": code })),
        // Client keepalive. Godot's WebSocketPeer cannot send protocol-level ping frames, so idle
        // connections are held open with an application-level round trip instead.
        "ping" => state.hub.send(id, &json!({ "t": "pong" })),
        _ => state.hub.send(id, &json!({ "t": "error", "message": "unknown_message" })),
    }
}

fn authenticate(state: &Arc<AppState>, id: u64, msg: &Value) {
    let key = msg.get("key").and_then(Value::as_str).unwrap_or("");
    let want_id = msg.get("id").and_then(Value::as_str).unwrap_or("");
    let row = state.db.get_key_by_secret_hash(&auth::hash_secret(key));
    let ok = match &row {
        Some(k) if want_id.is_empty() || k.id == want_id => true,
        _ => false,
    };
    if !ok {
        state.hub.send(id, &json!({ "t": "hello", "ok": false }));
        return;
    }
    let k = row.unwrap();
    {
        let mut inner = state.hub.inner.lock().unwrap();
        if let Some(s) = inner.get_mut(&id) {
            s.authed = true;
            s.key_id = k.id.clone();
            s.scopes = k.scopes.clone();
        }
    }
    state
        .hub
        .send(id, &json!({ "t": "hello", "ok": true, "scopes": k.scopes, "id": k.id }));
}

fn create_match(state: &Arc<AppState>, id: u64, scopes: &[String], key_id: &str, msg: &Value) {
    if !auth::has_scope(scopes, auth::HOST_MATCH) {
        state.hub.send(id, &json!({ "t": "error", "message": "forbidden_host" }));
        return;
    }
    let config = crate::registry::config_from_json(msg.get("config").unwrap_or(&Value::Null));
    match state.registry.create(key_id, config) {
        Ok(view) => attach_and_start(state, id, &view),
        Err(reason) => state.hub.send(id, &json!({ "t": "error", "message": reason })),
    }
}

fn join_match(state: &Arc<AppState>, id: u64, code: &str) {
    match state.registry.view(code) {
        Some(view) => attach_and_start(state, id, &view),
        None => state.hub.send(id, &json!({ "t": "error", "message": "no_match" })),
    }
}

fn attach_and_start(state: &Arc<AppState>, id: u64, view: &crate::registry::MatchView) {
    let human_seats = state.registry.human_seats(&view.code);
    let seat = state.hub.claim_seat(&view.code, id, &human_seats);
    if seat == -1 {
        state.hub.send(id, &json!({ "t": "error", "message": "match_full" }));
        return;
    }
    {
        let mut inner = state.hub.inner.lock().unwrap();
        if let Some(s) = inner.get_mut(&id) {
            s.code = view.code.clone();
            s.seat = seat;
        }
    }
    state.hub.send(
        id,
        &json!({
            "t": "match_start",
            "code": view.code,
            "seed": view.seed,
            "yourSeat": seat,
            "seats": view.seats,
            "snapshot": view.snapshot,
        }),
    );
}

fn action(state: &Arc<AppState>, id: u64, code: &str, seat: i32, action_dict: Value) {
    if code.is_empty() {
        state.hub.send(id, &json!({ "t": "error", "message": "not_in_match" }));
        return;
    }
    let dict = if action_dict.is_object() { action_dict } else { json!({}) };
    match state.registry.apply(code, seat, &dict) {
        Ok(out) => {
            for (aseat, adict) in &out.actions {
                state.hub.broadcast(
                    code,
                    &json!({
                        "t": "action",
                        "seat": aseat,
                        "action": adict,
                        "state_hash": out.state_hash,
                    }),
                );
            }
            if out.winner != -1 {
                state.hub.broadcast(code, &json!({ "t": "game_over", "winner": out.winner }));
            }
        }
        Err(reason) => state.hub.send(id, &json!({ "t": "error", "message": reason })),
    }
}

fn delete_match(state: &Arc<AppState>, id: u64, key_id: &str, code: &str) {
    if state.registry.owner_of(code).as_deref() != Some(key_id) {
        state.hub.send(id, &json!({ "t": "error", "message": "forbidden_delete" }));
        return;
    }
    state.registry.drop_match(code);
    state.db.delete_match(code);
    state.hub.broadcast(code, &json!({ "t": "room_closed" }));
    state.hub.detach_code(code);
}

fn detach(state: &Arc<AppState>, id: u64) {
    let mut inner = state.hub.inner.lock().unwrap();
    if let Some(s) = inner.get_mut(&id) {
        s.code = String::new();
        s.seat = -1;
    }
}

fn joinable_list(state: &Arc<AppState>) -> Vec<Value> {
    let mut out = Vec::new();
    for entry in state.registry.list() {
        if entry.status != "open" {
            continue;
        }
        let humans = state.registry.human_seats(&entry.code).len();
        let claimed = state.hub.claimed_count(&entry.code);
        out.push(json!({
            "code": entry.code,
            "seats": entry.seats,
            "owner": entry.owner,
            "status": entry.status,
            "current_seat": entry.current_seat,
            "winner": entry.winner,
            "open_seats": humans.saturating_sub(claimed),
        }));
    }
    out
}
