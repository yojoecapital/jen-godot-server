//! API-key auth (port of `auth.gd`, unified with the former admin API).
//!
//! Every REST call and every WS `auth` message presents a key *secret*; we store only its SHA-256,
//! and resolve a caller by hashing the presented secret and looking the row up by hash — so a wrong
//! secret leaks nothing and no plaintext secret is ever persisted. Scopes gate capability:
//! `admin` (mint/revoke keys + manage every match), `host_match` (create/join/delete own matches),
//! `join_match` (join only).

use rand::RngCore;
use sha2::{Digest, Sha256};

pub const ADMIN: &str = "admin";
pub const HOST_MATCH: &str = "host_match";
pub const JOIN_MATCH: &str = "join_match";

pub const VALID_SCOPES: [&str; 3] = [ADMIN, HOST_MATCH, JOIN_MATCH];

/// 24 random bytes, hex-encoded — the one-time plaintext handed back at key creation.
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex(&bytes)
}

/// `JenAuth.hash_secret` — SHA-256 hex, the value stored and indexed in the DB.
pub fn hash_secret(secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    hex(&h.finalize())
}

/// `JenAuth.normalize_scopes` — keep only recognized scopes, de-duplicated, in first-seen order.
pub fn normalize_scopes(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in raw {
        if VALID_SCOPES.contains(&s.as_str()) && !out.iter().any(|k| k == s) {
            out.push(s.clone());
        }
    }
    out
}

pub fn has_scope(scopes: &[String], scope: &str) -> bool {
    scopes.iter().any(|s| s == scope)
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_is_deterministic_and_hex() {
        assert_eq!(hash_secret("x"), hash_secret("x"));
        assert_ne!(hash_secret("x"), hash_secret("y"));
        assert_eq!(hash_secret("x").len(), 64); // SHA-256 hex
    }

    #[test]
    fn secret_is_random_and_long() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b);
        assert_eq!(a.len(), 48); // 24 bytes hex
    }

    #[test]
    fn scopes_dedup_and_filter() {
        let got = normalize_scopes(&[
            "host_match".into(),
            "host_match".into(),
            "bogus".into(),
            "admin".into(),
        ]);
        assert_eq!(got, vec!["host_match".to_string(), "admin".to_string()]);
    }
}
