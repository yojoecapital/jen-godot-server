//! REST API + embedded management UI (replaces `admin_api.gd`, unified onto API-key auth).
//!
//! Every `/api/*` call authenticates with `Authorization: Bearer <key-secret>` (the same secrets the
//! WS gateway accepts). Scope gates: `admin` mints/revokes keys and sees every match; any key sees
//! and deletes its own matches. The old `/admin/*` + `ADMIN_API_SECRET`-bearer routes are gone — the
//! admin is now just a key with the `admin` scope (seeded from `ADMIN_API_SECRET` on boot).

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::auth;
use crate::db::ApiKey;
use crate::AppState;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/api/me", get(me))
        .route("/api/keys", post(create_key).get(list_keys))
        .route("/api/keys/:id", delete(delete_key))
        .route("/api/matches", get(list_matches))
        .route("/api/matches/:code", delete(delete_match))
        .with_state(state)
}

// ---- auth helper ----

/// Resolve the calling key from the bearer secret, or `None` if absent/unknown.
fn caller(state: &AppState, headers: &HeaderMap) -> Option<ApiKey> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let secret = raw.strip_prefix("Bearer ")?;
    state.db.get_key_by_secret_hash(&auth::hash_secret(secret))
}

fn err(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({ "error": code }))).into_response()
}

// ---- /api/me ----

async fn me(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    match caller(&state, &headers) {
        Some(k) => Json(json!({ "id": k.id, "scopes": k.scopes })).into_response(),
        None => err(StatusCode::UNAUTHORIZED, "unauthorized"),
    }
}

// ---- /api/keys (admin) ----

async fn create_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let Some(k) = caller(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if !auth::has_scope(&k.scopes, auth::ADMIN) {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }
    let parsed: Value = match serde_json::from_str(&body) {
        Ok(v @ Value::Object(_)) => v,
        _ => return err(StatusCode::BAD_REQUEST, "bad_json"),
    };
    let id = parsed.get("id").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if id.is_empty() {
        return err(StatusCode::BAD_REQUEST, "missing_id");
    }
    let raw_scopes: Vec<String> = parsed
        .get("scopes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let scopes = auth::normalize_scopes(&raw_scopes);
    if scopes.is_empty() {
        return err(StatusCode::BAD_REQUEST, "no_valid_scopes");
    }
    if state.db.key_exists(&id) {
        return err(StatusCode::CONFLICT, "id_exists");
    }
    let secret = auth::generate_secret();
    if !state.db.insert_key(&id, &auth::hash_secret(&secret), &scopes) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, "db_error");
    }
    (
        StatusCode::CREATED,
        Json(json!({ "id": id, "secret": secret, "scopes": scopes })),
    )
        .into_response()
}

async fn list_keys(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(k) = caller(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if !auth::has_scope(&k.scopes, auth::ADMIN) {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }
    // Secrets are never returned — only their metadata.
    let keys: Vec<Value> = state
        .db
        .list_keys()
        .iter()
        .map(|key| json!({ "id": key.id, "scopes": key.scopes, "created_at": key.created_at }))
        .collect();
    Json(json!({ "keys": keys })).into_response()
}

async fn delete_key(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(k) = caller(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    if !auth::has_scope(&k.scopes, auth::ADMIN) {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }
    Json(json!({ "ok": state.db.delete_key(&id) })).into_response()
}

// ---- /api/matches ----

async fn list_matches(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let Some(k) = caller(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let admin = auth::has_scope(&k.scopes, auth::ADMIN);
    let owner = if admin { None } else { Some(k.id.as_str()) };
    let matches: Vec<Value> = state
        .db
        .list_matches(owner)
        .iter()
        .map(|m| {
            json!({
                "code": m.code,
                "owner_key_id": m.owner_key_id,
                "seats": m.seats,
                "status": m.status,
                "created_at": m.created_at,
                "updated_at": m.updated_at,
            })
        })
        .collect();
    Json(json!({ "matches": matches })).into_response()
}

async fn delete_match(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Response {
    let Some(k) = caller(&state, &headers) else {
        return err(StatusCode::UNAUTHORIZED, "unauthorized");
    };
    let admin = auth::has_scope(&k.scopes, auth::ADMIN);
    let owns = state
        .db
        .get_match(&code)
        .map(|m| m.owner_key_id == k.id)
        .unwrap_or(false);
    if !admin && !owns {
        return err(StatusCode::FORBIDDEN, "forbidden");
    }
    state.registry.drop_match(&code);
    Json(json!({ "ok": state.db.delete_match(&code) })).into_response()
}

// ---- embedded UI ----

async fn index() -> Html<&'static str> {
    Html(include_str!("ui/index.html"))
}

async fn app_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("ui/app.js"),
    )
        .into_response()
}

async fn style_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("ui/style.css"),
    )
        .into_response()
}
