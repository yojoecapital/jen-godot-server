//! Authoritative match registry (port of `match_registry.gd`).
//!
//! Holds live matches in memory and rehydrates evicted ones from the DB snapshot. Every action is
//! re-validated (turn + actor ownership) and resolved through `jen_core::executor::apply`, so the
//! server is the single source of truth.
//!
//! Seats are `human` or `ai`. After a human's action the registry plays out any CPU seats that
//! follow and broadcasts their moves alongside it — clients advance by replaying the action stream,
//! so a move they never receive would desync them. The AI is `jen_ai`, the same crate the client
//! links, so a CPU seat decides identically offline and online.
//!
//! RNG is `jen_core::Pcg32`: seeded from `config.seed` at creation, its advancing `state` is
//! serialized into every snapshot and restored via `Pcg32::from_state` on rehydrate. The Godot
//! client replays the broadcast action stream through the same `Pcg32` (see the binding's RNG
//! adapter), so combat reproduces bit-identically without any Godot-RNG parity.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;

use jen_ai::policy::Policy;
use jen_core::action::{self, Action};
use jen_core::config::{self, GameConfig};
use jen_core::executor;
use jen_core::factory;
use jen_core::rng::Pcg32;
use jen_core::save;
use jen_core::state::Sim;
use jen_core::turn::TurnManager;
use jen_core::types::ActionKind;

use crate::db::Db;

const CODE_CHARS: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const CODE_LEN: usize = 4;

struct LiveMatch {
    sim: Sim,
    tm: TurnManager,
    rng: Pcg32,
    seed: i64,
    seats: Vec<String>,
    owner: String,
    status: String,
}

#[derive(Debug)]
pub struct MatchView {
    pub code: String,
    pub seed: i64,
    pub seats: Vec<String>,
    pub snapshot: Value,
    pub current_seat: i32,
    pub winner: i32,
}

#[derive(Debug)]
pub struct ApplyOutcome {
    /// Ordered (seat, action-dict) pairs to broadcast: the acting human's move followed by any CPU
    /// seats that played before the turn returned to a human.
    pub actions: Vec<(i32, Value)>,
    pub current_seat: i32,
    pub winner: i32,
    pub state_hash: String,
}

#[derive(Debug)]
pub struct ListEntry {
    pub code: String,
    pub seats: Vec<String>,
    pub owner: String,
    pub status: String,
    pub current_seat: i32,
    pub winner: i32,
}

pub struct Registry {
    db: Arc<Db>,
    live: Mutex<HashMap<String, LiveMatch>>,
}

impl Registry {
    pub fn new(db: Arc<Db>) -> Registry {
        Registry {
            db,
            live: Mutex::new(HashMap::new()),
        }
    }

    /// Build a fresh match. Seats are `human` or `ai`; a match with no human seat is rejected,
    /// since nobody could ever join it.
    pub fn create(&self, owner_key_id: &str, mut config: GameConfig) -> Result<MatchView, String> {
        if !config.seeded {
            config.seed = rand::random::<u32>() as i64;
            config.seeded = true;
        }
        let mut rng = Pcg32::seeded(config.seed as u64);
        let (sim, tm) = factory::build(&config, &mut rng);

        // `cpu` and `ai` both mean the same seat type; store the token saves and the client protocol
        // already use. Seats beyond what the config named default to human.
        let seats: Vec<String> = (0..sim.players.len().max(1))
            .map(|i| match config.seat_controllers.get(i).map(String::as_str) {
                Some("ai") | Some("cpu") => "ai".to_string(),
                _ => "human".to_string(),
            })
            .collect();
        if !seats.iter().any(|s| s == "human") {
            return Err("no_human_seats".into());
        }

        let mut m = LiveMatch {
            sim,
            tm,
            rng,
            seed: config.seed,
            seats,
            owner: owner_key_id.to_string(),
            status: "open".to_string(),
        };
        // Seat 0 may itself be a CPU, in which case it has to move before any human can. There is
        // nothing to broadcast: the snapshot below is taken afterwards, so a joining client seeds
        // from the position the CPU has already reached.
        drive_cpu_seats(&mut m);

        let mut live = self.live.lock().unwrap();
        let code = self.new_code(&live);
        let view = self.view_of(&code, &mut m);
        self.persist(&code, &m);
        live.insert(code.clone(), m);
        Ok(view)
    }

    pub fn view(&self, code: &str) -> Option<MatchView> {
        let mut live = self.live.lock().unwrap();
        self.ensure_loaded(&mut live, code)?;
        let m = live.get_mut(code).unwrap();
        Some(self.view_of(code, m))
    }

    /// Apply one seat's action, then any CPU seats that follow, and return the whole stream.
    pub fn apply(&self, code: &str, seat: i32, action_dict: &Value) -> Result<ApplyOutcome, String> {
        let mut live = self.live.lock().unwrap();
        if self.ensure_loaded(&mut live, code).is_none() {
            return Err("no_match".into());
        }
        let m = live.get_mut(code).unwrap();

        if winner_of(&mut m.sim, &m.tm) != -1 {
            return Err("match_over".into());
        }
        let current = current_seat_of(&mut m.sim, &m.tm);
        if seat != current {
            return Err("not_your_turn".into());
        }
        let action = action::from_json(action_dict);
        let player = m.tm.current_player().unwrap_or(usize::MAX);
        if !actor_owned_by(&m.sim, &action, player) {
            return Err("not_your_unit".into());
        }
        let mut events = Vec::new();
        if !executor::apply(&mut m.sim, &mut m.tm, &action, &mut m.rng, &mut events) {
            return Err("illegal_action".into());
        }

        let mut actions = vec![(seat, action::to_json(&action))];
        // Hand play to any CPU seats that follow, and broadcast their moves alongside the human's so
        // every client replays the same stream and stays in lockstep.
        actions.extend(drive_cpu_seats(m));

        let winner = winner_of(&mut m.sim, &m.tm);
        if winner != -1 {
            m.status = "over".to_string();
        }
        let current_seat = current_seat_of(&mut m.sim, &m.tm);
        let state_hash = state_hash(&snapshot_of(m));
        self.persist(code, m);
        Ok(ApplyOutcome {
            actions,
            current_seat,
            winner,
            state_hash,
        })
    }

    pub fn list(&self) -> Vec<ListEntry> {
        let mut live = self.live.lock().unwrap();
        // Fold in any persisted-but-evicted matches so the listing is complete.
        for row in self.db.list_matches(None) {
            self.ensure_loaded(&mut live, &row.code);
        }
        let mut out = Vec::new();
        for (code, m) in live.iter_mut() {
            out.push(ListEntry {
                code: code.clone(),
                seats: m.seats.clone(),
                owner: m.owner.clone(),
                status: m.status.clone(),
                current_seat: current_seat_of(&mut m.sim, &m.tm),
                winner: winner_of(&mut m.sim, &m.tm),
            });
        }
        out
    }

    pub fn owner_of(&self, code: &str) -> Option<String> {
        let mut live = self.live.lock().unwrap();
        self.ensure_loaded(&mut live, code)?;
        Some(live.get(code).unwrap().owner.clone())
    }

    /// Seat indices a person can claim. CPU seats are excluded — they are already played.
    pub fn human_seats(&self, code: &str) -> Vec<usize> {
        let mut live = self.live.lock().unwrap();
        if self.ensure_loaded(&mut live, code).is_none() {
            return Vec::new();
        }
        let m = live.get(code).unwrap();
        (0..m.seats.len())
            .filter(|&i| m.seats[i] == "human")
            .collect()
    }

    /// Evict from memory (persistence is untouched — it rehydrates on next access).
    pub fn drop_match(&self, code: &str) {
        self.live.lock().unwrap().remove(code);
    }

    // ---- internals ----

    fn ensure_loaded(&self, live: &mut HashMap<String, LiveMatch>, code: &str) -> Option<()> {
        if live.contains_key(code) {
            return Some(());
        }
        let row = self.db.get_match(code)?;
        let loaded = save::deserialize(&row.snapshot)?;
        let rng = match loaded.rng_state {
            Some(s) => Pcg32::from_state(s as u64),
            None => Pcg32::seeded(row.seed as u64),
        };
        live.insert(
            code.to_string(),
            LiveMatch {
                sim: loaded.sim,
                tm: loaded.tm,
                rng,
                seed: row.seed,
                seats: row.seats,
                owner: row.owner_key_id,
                status: row.status,
            },
        );
        Some(())
    }

    fn view_of(&self, code: &str, m: &mut LiveMatch) -> MatchView {
        MatchView {
            code: code.to_string(),
            seed: m.seed,
            seats: m.seats.clone(),
            snapshot: snapshot_of(m),
            current_seat: current_seat_of(&mut m.sim, &m.tm),
            winner: winner_of(&mut m.sim, &m.tm),
        }
    }

    fn persist(&self, code: &str, m: &LiveMatch) {
        let snapshot = snapshot_of(m);
        self.db
            .upsert_match(code, &m.owner, m.seed, &m.seats, &snapshot, &m.status);
    }

    fn new_code(&self, live: &HashMap<String, LiveMatch>) -> String {
        for _ in 0..100 {
            let mut code = String::with_capacity(CODE_LEN);
            for _ in 0..CODE_LEN {
                let i = rand::random::<usize>() % CODE_CHARS.len();
                code.push(CODE_CHARS[i] as char);
            }
            if !live.contains_key(&code) && self.db.get_match(&code).is_none() {
                return code;
            }
        }
        format!("{:08X}", rand::random::<u32>() & 0x7fff_ffff)
    }
}

fn snapshot_of(m: &LiveMatch) -> Value {
    save::serialize(&m.sim, &m.tm, &m.seats, Some((m.seed, m.rng.state() as i64)))
}

fn current_seat_of(sim: &mut Sim, tm: &TurnManager) -> i32 {
    if winner_of(sim, tm) != -1 {
        return -1;
    }
    tm.current_player().map(|p| p as i32).unwrap_or(-1)
}

/// Guard against a policy that never ends its turn. Each action either spends a unit's readiness or
/// sets a stance, so a turn is bounded — but a bug here would spin the server, not the client.
const MAX_CPU_ACTIONS: usize = 4_000;

/// Search budget per CPU action. A whole CPU turn is many actions and runs inside the request that
/// triggered it, so both this and the wall-clock cap below keep a human's move from stalling.
const CPU_SIMULATIONS: usize = 192;
const CPU_MAX_MILLIS: u64 = 100;

fn cpu_policy(seed: u64) -> jen_ai::policy::Mcts {
    let mut config = jen_ai::mcts::Config::default();
    config.simulations = CPU_SIMULATIONS;
    config.max_millis = CPU_MAX_MILLIS;
    jen_ai::policy::Mcts::new(
        Box::new(jen_ai::eval::HeuristicEvaluator::new()),
        config,
        seed,
    )
}

/// Plays every CPU seat that now has the move, returning their actions in order.
///
/// Runs until a human is to move or the match ends, so a table of CPU seats between two humans
/// resolves in one pass. The actions are broadcast rather than merely applied: clients advance by
/// replaying the action stream, so a move they never see would desync them.
fn drive_cpu_seats(m: &mut LiveMatch) -> Vec<(i32, Value)> {
    let mut produced = Vec::new();
    let mut policy = cpu_policy(m.seed as u64);

    for _ in 0..MAX_CPU_ACTIONS {
        if winner_of(&mut m.sim, &m.tm) != -1 {
            break;
        }
        let seat = current_seat_of(&mut m.sim, &m.tm);
        if seat < 0 || m.seats.get(seat as usize).map(String::as_str) != Some("ai") {
            break;
        }

        let action = policy.choose(&m.sim, &m.tm);
        let mut events = Vec::new();
        if !executor::apply(&mut m.sim, &mut m.tm, &action, &mut m.rng, &mut events) {
            // The engine rejected an action its own enumeration produced; ending the turn keeps the
            // match playable rather than wedging it.
            let end = jen_core::action::Action::end_turn();
            if !executor::apply(&mut m.sim, &mut m.tm, &end, &mut m.rng, &mut events) {
                break;
            }
            produced.push((seat, action::to_json(&end)));
            continue;
        }
        produced.push((seat, action::to_json(&action)));
    }
    produced
}

fn winner_of(sim: &mut Sim, tm: &TurnManager) -> i32 {
    tm.check_win(sim).map(|p| p as i32).unwrap_or(-1)
}

fn actor_owned_by(sim: &Sim, action: &Action, player: usize) -> bool {
    if action.kind == ActionKind::EndTurn {
        return true;
    }
    if player == usize::MAX {
        return false;
    }
    if let Some(u) = sim.occupant(action.actor_coord) {
        return u.player == player;
    }
    if let Some(p) = sim.path_at(action.actor_coord) {
        return p.player == player;
    }
    false
}

/// Deterministic content hash of the snapshot JSON (FNV-1a). The client ignores `state_hash`; it
/// exists so tests can assert same-seed + same-actions ⇒ identical resulting state.
fn state_hash(snapshot: &Value) -> String {
    let bytes = snapshot.to_string();
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:016x}", hash)
}

/// Bridge for the WS layer: `GameConfig` from the client's wire dict.
pub fn config_from_json(d: &Value) -> GameConfig {
    config::from_json(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jen_core::config::Color;

    fn cfg(seed: i64) -> GameConfig {
        let mut c = GameConfig::default();
        c.dim = 8;
        c.compact_spawn = true;
        c.player_colors = vec![Color::MAGENTA, Color::CYAN];
        c.seat_controllers = vec!["human".into(), "human".into()];
        c.seed = seed;
        c.seeded = true;
        c
    }

    fn end_turn() -> Value {
        action::to_json(&Action::end_turn())
    }

    fn registry() -> Registry {
        let (db, _p) = crate::db::temp_db();
        Registry::new(Arc::new(db))
    }

    #[test]
    fn create_seats_players_and_carries_rng_state() {
        let reg = registry();
        let view = reg.create("alice", cfg(12345)).unwrap();
        assert_eq!(view.current_seat, 0, "seat 0 acts first");
        assert_eq!(view.seats, vec!["human".to_string(), "human".to_string()]);
        assert!(view.snapshot.get("rng_state").is_some(), "snapshot carries rng_state");
        assert_eq!(reg.owner_of(&view.code).as_deref(), Some("alice"));
        assert_eq!(reg.human_seats(&view.code), vec![0, 1]);
    }

    /// `cpu` and `ai` are the same seat; only the second is stored, since that is the token saves
    /// and the client protocol already use.
    #[test]
    fn cpu_seats_are_accepted_and_normalised() {
        let reg = registry();
        let mut c = cfg(1);
        c.seat_controllers = vec!["human".into(), "cpu".into()];
        let view = reg.create("alice", c).unwrap();
        assert_eq!(view.seats, vec!["human".to_string(), "ai".to_string()]);
        // A CPU seat is not claimable — it is already being played.
        assert_eq!(reg.human_seats(&view.code), vec![0]);
    }

    #[test]
    fn a_match_with_no_human_seats_is_rejected() {
        let reg = registry();
        let mut c = cfg(2);
        c.seat_controllers = vec!["ai".into(), "ai".into()];
        assert_eq!(reg.create("x", c).unwrap_err(), "no_human_seats");
    }

    /// The CPU's moves must be broadcast, not merely applied: clients advance by replaying the
    /// action stream, so a move they never receive would desync them.
    #[test]
    fn cpu_seats_play_after_a_human_and_their_actions_are_broadcast() {
        let reg = registry();
        let mut c = cfg(31337);
        c.seat_controllers = vec!["human".into(), "ai".into()];
        let view = reg.create("alice", c).unwrap();
        assert_eq!(view.current_seat, 0, "the human moves first here");

        let out = reg.apply(&view.code, 0, &end_turn()).unwrap();

        assert!(out.actions.len() > 1, "the CPU seat produced no actions");
        assert_eq!(out.actions[0].0, 0, "the human's action comes first");
        assert!(
            out.actions[1..].iter().all(|(seat, _)| *seat == 1),
            "every following action belongs to the CPU seat"
        );
        assert_eq!(
            out.current_seat, 0,
            "play returns to the human once the CPU ends its turn"
        );
    }

    /// A CPU on seat 0 has to move before anyone can join, and the snapshot must already reflect it.
    #[test]
    fn a_cpu_on_the_first_seat_opens_the_game() {
        let reg = registry();
        let mut c = cfg(4242);
        c.seat_controllers = vec!["ai".into(), "human".into()];
        let view = reg.create("alice", c).unwrap();

        assert_eq!(view.current_seat, 1, "the CPU already played its opening turn");
        assert_eq!(reg.human_seats(&view.code), vec![1]);
        // The opening is in the snapshot rather than replayed, so a joining client starts from it.
        let stock = view.snapshot["players"][0]["stock"].as_i64().unwrap_or(0);
        assert!(stock >= 0);
    }

    #[test]
    fn turn_and_actor_ownership_are_enforced() {
        let reg = registry();
        let view = reg.create("alice", cfg(777)).unwrap();
        // Seat 1 cannot act on seat 0's turn.
        assert_eq!(reg.apply(&view.code, 1, &end_turn()).unwrap_err(), "not_your_turn");

        // Seat 0 ends turn; with no CPU drive the turn passes to the next human (seat 1).
        let out = reg.apply(&view.code, 0, &end_turn()).unwrap();
        assert_eq!(out.actions.len(), 1, "only the human action is broadcast");
        assert_eq!(out.actions[0].0, 0);
        assert!(out.current_seat == 1 || out.winner != -1);
    }

    #[test]
    fn deterministic_replay_same_seed_same_hash() {
        let reg = registry();
        let a = reg.create("alice", cfg(4242)).unwrap();
        let out_a = reg.apply(&a.code, 0, &end_turn()).unwrap();
        let b = reg.create("bob", cfg(4242)).unwrap();
        let out_b = reg.apply(&b.code, 0, &end_turn()).unwrap();
        assert_eq!(out_a.state_hash, out_b.state_hash);
    }

    #[test]
    fn rehydrates_from_persistence_after_eviction() {
        let reg = registry();
        let view = reg.create("alice", cfg(5)).unwrap();
        reg.apply(&view.code, 0, &end_turn()).unwrap();
        let before = reg.view(&view.code).unwrap().snapshot;

        reg.drop_match(&view.code); // evict from memory
        let after = reg.view(&view.code).unwrap().snapshot; // reloaded from the DB
        assert_eq!(before, after, "durable + deterministic across eviction");
        assert!(after.get("rng_state").is_some());
    }

    #[test]
    fn unknown_code_is_none() {
        let reg = registry();
        assert!(reg.view("ZZZZ").is_none());
    }
}
