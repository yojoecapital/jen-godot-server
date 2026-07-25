//! Cross-runtime determinism vector generator.
//!
//! Plays a recorded, randomized self-play game with `jen_core` (exactly what the server runs), then
//! emits `{ snapshot0, actions, final, combat }` as JSON. The Godot client replays the same action
//! list from `snapshot0` through its own `jen_core` (via the RNG adapter) and must land on the same
//! `final` game state — proving both peers consume the shared `Pcg32` identically. `combat` counts
//! RNG-consuming events so the check can assert the RNG path was actually exercised.

use jen_core::action::{self, Action};
use jen_core::config::{Color, GameConfig};
use jen_core::event::CoreEvent;
use jen_core::executor;
use jen_core::factory;
use jen_core::legal_moves;
use jen_core::rng::{Pcg32, Rng};
use jen_core::save;
use jen_core::types::ActionKind;
use serde_json::json;

fn main() {
    let mut c = GameConfig::default();
    c.dim = 8;
    c.compact_spawn = true;
    c.player_colors = vec![Color::MAGENTA, Color::CYAN];
    c.seat_controllers = vec!["human".into(), "human".into()];
    c.seed = 20_260_711;
    c.seeded = true;

    // Game RNG (the one that must stay in lockstep with the client).
    let mut rng = Pcg32::seeded(c.seed as u64);
    let (mut sim, mut tm) = factory::build(&c, &mut rng);
    let seats = vec!["human".to_string(); sim.players.len()];
    let snapshot0 = save::serialize(&sim, &tm, &seats, Some((c.seed, rng.state() as i64)));

    // Separate RNG for *action selection* only — never touches the game stream, so the recorded
    // action list fully determines replay.
    let mut sel = Pcg32::seeded(0xC0FFEE);
    let mut actions = Vec::new();
    let mut combat = 0u32;

    for _ in 0..400 {
        if sim.is_over {
            break;
        }
        let moves = legal_moves::enumerate(&sim, &tm);
        if moves.is_empty() {
            break;
        }
        // Bias away from END_TURN so tanks actually roam and clash; fall back to it when it's all
        // that's legal.
        let non_end: Vec<&Action> = moves.iter().filter(|a| a.kind != ActionKind::EndTurn).collect();
        let pick = if !non_end.is_empty() && sel.randi_range(0, 3) != 0 {
            (*non_end[sel.randi_range(0, non_end.len() as i32 - 1) as usize]).clone()
        } else {
            moves[sel.randi_range(0, moves.len() as i32 - 1) as usize].clone()
        };

        let mut events = Vec::new();
        if !executor::apply(&mut sim, &mut tm, &pick, &mut rng, &mut events) {
            continue;
        }
        for e in &events {
            if matches!(
                e,
                CoreEvent::Attack { .. }
                    | CoreEvent::LongShot { .. }
                    | CoreEvent::MissileIntercepted { .. }
            ) {
                combat += 1;
            }
        }
        actions.push(action::to_json(&pick));
    }

    let final_snap = save::serialize(&sim, &tm, &seats, None);
    let out = json!({
        "snapshot0": snapshot0,
        "actions": actions,
        "final": final_snap,
        "combat": combat,
    });
    println!("{}", serde_json::to_string(&out).unwrap());
}
