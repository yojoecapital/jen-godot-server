//! The shipped version, which the game and the server share.
//!
//! Client and server run the same `jen_core`, and online play is deterministic action-replay — the
//! client re-resolves the server's actions locally rather than being told the outcome. Two peers on
//! different builds would therefore diverge silently rather than fail loudly, so versions are
//! checked at the handshake instead.

/// Comes from `[package] version` in Cargo.toml, kept in step with the game's
/// `application/config/version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Whether a peer's reported version is compatible with ours.
///
/// Exact match, deliberately: with a shared rules engine there is no version skew that is safe by
/// construction, and a desync mid-match is far worse to debug than a refused connection.
pub fn is_compatible(other: &str) -> bool {
    normalize(other) == normalize(VERSION)
}

/// Godot's project setting carries a leading `v` (`v0.0.4-alpha`) and Cargo's cannot, so the two
/// are compared without it.
fn normalize(v: &str) -> &str {
    v.trim().trim_start_matches(['v', 'V'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_v_prefix_is_ignored() {
        assert!(is_compatible(VERSION));
        assert!(is_compatible(&format!("v{VERSION}")));
        assert!(is_compatible(&format!("  v{VERSION}  ")));
    }

    #[test]
    fn a_different_version_is_rejected() {
        assert!(!is_compatible("0.0.1-alpha"));
        assert!(!is_compatible("v9.9.9"));
        // A client too old to send one at all cannot be trusted to replay identically.
        assert!(!is_compatible(""));
    }
}
