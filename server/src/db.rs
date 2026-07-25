//! SQLite persistence (port of `db.gd`, JSON files → rusqlite).
//!
//! One process-wide `Connection` behind a `Mutex` (game traffic is low-throughput, so a global lock
//! is simpler and safe). `bundled` compiles SQLite from source per target arch, sidestepping the
//! missing-shared-lib problem that forced the earlier JSON fallback.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

pub struct ApiKey {
    pub id: String,
    pub secret_hash: String,
    pub scopes: Vec<String>,
    pub created_at: i64,
}

pub struct MatchRow {
    pub code: String,
    pub owner_key_id: String,
    pub seed: i64,
    pub seats: Vec<String>,
    pub snapshot: Value,
    pub status: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct Db {
    conn: Mutex<Connection>,
}

impl Db {
    pub fn open(path: &str) -> rusqlite::Result<Db> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS api_keys (
                id          TEXT PRIMARY KEY,
                secret_hash TEXT NOT NULL,
                scopes      TEXT NOT NULL,
                created_at  INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_api_keys_secret_hash ON api_keys(secret_hash);
            CREATE TABLE IF NOT EXISTS matches (
                code         TEXT PRIMARY KEY,
                owner_key_id TEXT NOT NULL,
                seed         INTEGER NOT NULL,
                seats        TEXT NOT NULL,
                snapshot     TEXT NOT NULL,
                status       TEXT NOT NULL,
                created_at   INTEGER NOT NULL,
                updated_at   INTEGER NOT NULL
            );",
        )?;
        Ok(Db {
            conn: Mutex::new(conn),
        })
    }

    // ---- api_keys ----

    /// Insert-or-replace — used to seed/refresh the `admin` key from `ADMIN_API_SECRET` on boot.
    pub fn upsert_key(&self, id: &str, secret_hash: &str, scopes: &[String]) {
        let conn = self.conn.lock().unwrap();
        let existing_created: Option<i64> = conn
            .query_row("SELECT created_at FROM api_keys WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        let created = existing_created.unwrap_or_else(now);
        conn.execute(
            "INSERT INTO api_keys (id, secret_hash, scopes, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET secret_hash = excluded.secret_hash, scopes = excluded.scopes",
            params![id, secret_hash, scopes_json(scopes), created],
        )
        .unwrap();
    }

    /// Mint a new key; fails (returns false) if the id is already taken.
    pub fn insert_key(&self, id: &str, secret_hash: &str, scopes: &[String]) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO api_keys (id, secret_hash, scopes, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![id, secret_hash, scopes_json(scopes), now()],
        )
        .is_ok()
    }

    pub fn key_exists(&self, id: &str) -> bool {
        self.get_key_by_id(id).is_some()
    }

    pub fn get_key_by_id(&self, id: &str) -> Option<ApiKey> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, secret_hash, scopes, created_at FROM api_keys WHERE id = ?1",
            [id],
            row_to_key,
        )
        .optional()
        .unwrap_or(None)
    }

    pub fn get_key_by_secret_hash(&self, secret_hash: &str) -> Option<ApiKey> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, secret_hash, scopes, created_at FROM api_keys WHERE secret_hash = ?1",
            [secret_hash],
            row_to_key,
        )
        .optional()
        .unwrap_or(None)
    }

    pub fn list_keys(&self) -> Vec<ApiKey> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT id, secret_hash, scopes, created_at FROM api_keys ORDER BY created_at DESC")
            .unwrap();
        let rows = stmt.query_map([], row_to_key).unwrap();
        rows.filter_map(Result::ok).collect()
    }

    pub fn delete_key(&self, id: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM api_keys WHERE id = ?1", [id]).unwrap_or(0) > 0
    }

    // ---- matches ----

    pub fn upsert_match(
        &self,
        code: &str,
        owner_key_id: &str,
        seed: i64,
        seats: &[String],
        snapshot: &Value,
        status: &str,
    ) {
        let conn = self.conn.lock().unwrap();
        let existing_created: Option<i64> = conn
            .query_row("SELECT created_at FROM matches WHERE code = ?1", [code], |r| r.get(0))
            .optional()
            .unwrap_or(None);
        let n = now();
        let created = existing_created.unwrap_or(n);
        conn.execute(
            "INSERT INTO matches (code, owner_key_id, seed, seats, snapshot, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(code) DO UPDATE SET
                owner_key_id = excluded.owner_key_id,
                seed         = excluded.seed,
                seats        = excluded.seats,
                snapshot     = excluded.snapshot,
                status       = excluded.status,
                updated_at   = excluded.updated_at",
            params![
                code,
                owner_key_id,
                seed,
                scopes_json(seats),
                snapshot.to_string(),
                status,
                created,
                n
            ],
        )
        .unwrap();
    }

    pub fn get_match(&self, code: &str) -> Option<MatchRow> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT code, owner_key_id, seed, seats, snapshot, status, created_at, updated_at
             FROM matches WHERE code = ?1",
            [code],
            row_to_match,
        )
        .optional()
        .unwrap_or(None)
    }

    /// All matches (admin), or just those owned by `owner` when `Some`.
    pub fn list_matches(&self, owner: Option<&str>) -> Vec<MatchRow> {
        let conn = self.conn.lock().unwrap();
        match owner {
            Some(o) => {
                let mut stmt = conn
                    .prepare(
                        "SELECT code, owner_key_id, seed, seats, snapshot, status, created_at, updated_at
                         FROM matches WHERE owner_key_id = ?1 ORDER BY updated_at DESC",
                    )
                    .unwrap();
                let rows = stmt.query_map([o], row_to_match).unwrap();
                rows.filter_map(Result::ok).collect()
            }
            None => {
                let mut stmt = conn
                    .prepare(
                        "SELECT code, owner_key_id, seed, seats, snapshot, status, created_at, updated_at
                         FROM matches ORDER BY updated_at DESC",
                    )
                    .unwrap();
                let rows = stmt.query_map([], row_to_match).unwrap();
                rows.filter_map(Result::ok).collect()
            }
        }
    }

    pub fn delete_match(&self, code: &str) -> bool {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM matches WHERE code = ?1", [code]).unwrap_or(0) > 0
    }
}

fn row_to_key(r: &rusqlite::Row) -> rusqlite::Result<ApiKey> {
    let scopes_text: String = r.get(2)?;
    Ok(ApiKey {
        id: r.get(0)?,
        secret_hash: r.get(1)?,
        scopes: parse_string_array(&scopes_text),
        created_at: r.get(3)?,
    })
}

fn row_to_match(r: &rusqlite::Row) -> rusqlite::Result<MatchRow> {
    let seats_text: String = r.get(3)?;
    let snapshot_text: String = r.get(4)?;
    Ok(MatchRow {
        code: r.get(0)?,
        owner_key_id: r.get(1)?,
        seed: r.get(2)?,
        seats: parse_string_array(&seats_text),
        snapshot: serde_json::from_str(&snapshot_text).unwrap_or(Value::Null),
        status: r.get(5)?,
        created_at: r.get(6)?,
        updated_at: r.get(7)?,
    })
}

fn scopes_json(scopes: &[String]) -> String {
    serde_json::to_string(scopes).unwrap_or_else(|_| "[]".into())
}

fn parse_string_array(text: &str) -> Vec<String> {
    serde_json::from_str(text).unwrap_or_default()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) fn temp_db() -> (Db, String) {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir()
        .join(format!("jen_test_{}_{}.db", std::process::id(), n))
        .to_string_lossy()
        .into_owned();
    let _ = std::fs::remove_file(&path);
    (Db::open(&path).unwrap(), path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_crud() {
        let (db, _p) = temp_db();
        assert!(db.insert_key("alice", "hashA", &["host_match".into()]));
        // duplicate id rejected
        assert!(!db.insert_key("alice", "hashX", &["admin".into()]));
        assert!(db.key_exists("alice"));

        let by_id = db.get_key_by_id("alice").unwrap();
        assert_eq!(by_id.scopes, vec!["host_match".to_string()]);
        let by_hash = db.get_key_by_secret_hash("hashA").unwrap();
        assert_eq!(by_hash.id, "alice");
        assert!(db.get_key_by_secret_hash("nope").is_none());

        db.insert_key("bob", "hashB", &["join_match".into()]);
        assert_eq!(db.list_keys().len(), 2);
        assert!(db.delete_key("alice"));
        assert_eq!(db.list_keys().len(), 1);
    }

    #[test]
    fn upsert_key_replaces_secret() {
        let (db, _p) = temp_db();
        db.upsert_key("admin", "h1", &["admin".into()]);
        db.upsert_key("admin", "h2", &["admin".into(), "host_match".into()]);
        let k = db.get_key_by_id("admin").unwrap();
        assert_eq!(k.secret_hash, "h2");
        assert_eq!(k.scopes.len(), 2);
        assert_eq!(db.list_keys().len(), 1); // still one row
    }

    #[test]
    fn match_upsert_get_list_delete() {
        let (db, _p) = temp_db();
        let snap = json!({ "state": 1 });
        db.upsert_match("AB12", "alice", 7, &["human".into()], &snap, "open");
        db.upsert_match("CD34", "bob", 9, &["human".into()], &snap, "open");

        let m = db.get_match("AB12").unwrap();
        assert_eq!(m.owner_key_id, "alice");
        assert_eq!(m.seed, 7);
        assert_eq!(m.snapshot, snap);

        assert_eq!(db.list_matches(None).len(), 2);
        assert_eq!(db.list_matches(Some("alice")).len(), 1);

        // update keeps created_at, moves status
        db.upsert_match("AB12", "alice", 7, &["human".into()], &snap, "over");
        assert_eq!(db.get_match("AB12").unwrap().status, "over");

        assert!(db.delete_match("AB12"));
        assert!(db.get_match("AB12").is_none());
    }
}
