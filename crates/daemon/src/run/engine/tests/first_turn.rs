//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), engine side: the
//! orchestrator launches with no prompt, and its first turn goes out through the
//! whole-paste wake-up once its anthrex server has announced its tools; past
//! `FIRST_TURN_WAIT_SECS` the run says so and keeps waiting.

use super::fixture::*;
use super::orch::{ORCH, add, first_turn_woken, launched_waiting, mcp_ready, planned};
use super::orch_restore::{restart, resume};
use crate::run::engine::first_turn::FIRST_TURN_WAIT_SECS;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::orch::START_PROMPT;
use crate::run::orch::contract::orchestrator_first_prompt;
use crate::run::snapshot::attention;

/// Each wake-up's `(text, first_turn)`.
fn wakes(effects: &[Effect]) -> Vec<(String, bool)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeOrchestrator {
                text, first_turn, ..
            } => Some((text.clone(), *first_turn)),
            _ => None,
        })
        .collect()
}

/// A user edit: it adds a wake note (decision 39).
fn user_edit(fx: &mut Fixture) -> Vec<Effect> {
    let n = fx.run().tasks.len();
    let edit = add(&format!("t{n}"), &format!("m{n}"));
    super::dispatch::edit(fx, vec![serde_json::from_value(edit).unwrap()])
}

fn first_prompt(fx: &Fixture) -> String {
    fx.run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .first_prompt
        .clone()
}

fn pending(fx: &Fixture) -> bool {
    fx.run()
        .orch
        .orchestrator
        .as_ref()
        .unwrap()
        .first_turn_pending
}

fn start_prompts(fx: &Fixture) -> usize {
    attention(fx.run(), fx.now)
        .iter()
        .filter(|l| *l == START_PROMPT)
        .count()
}

#[test]
fn a_fresh_orchestrator_launches_without_a_prompt() {
    let fx = planned(false);
    let (_, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.initial_prompt, None);
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert!(o.first_turn_pending);
    assert_eq!(o.first_prompt, orchestrator_first_prompt(fx.run()));
    assert!(!fx.run().orch.mcp_ready);
}

#[test]
fn the_first_turn_waits_for_mcp_ready() {
    let mut fx = launched_waiting(false);
    // A note is pending, the window is live: nothing goes before the first turn.
    let effects = user_edit(&mut fx);
    assert_eq!(wakes(&effects), vec![], "{effects:#?}");
    assert_eq!(wakes(&fx.tick()), vec![]);
    // Another window's notice changes nothing.
    let effects = mcp_ready(&mut fx, ORCH + 1);
    assert!(!fx.run().orch.mcp_ready);
    assert_eq!(wakes(&effects), vec![]);
    // The orchestrator's: one wake-up, the first prompt whole, marked as the first turn.
    let effects = mcp_ready(&mut fx, ORCH);
    assert!(fx.run().orch.mcp_ready);
    assert_eq!(wakes(&effects), vec![(first_prompt(&fx), true)]);
    assert!(pending(&fx), "held until the driver pasted it");
    // Pasted: the first turn is over, and the held note follows as a plain wake-up.
    let effects = first_turn_woken(&mut fx);
    assert!(!pending(&fx));
    let after = wakes(&effects);
    assert_eq!(after.len(), 1, "{effects:#?}");
    assert!(!after[0].1, "{after:?}");
    assert!(after[0].0.starts_with("[anthrex] Run "), "{after:?}");
}

#[test]
fn past_the_bound_nothing_is_pasted_and_the_start_prompt_shows() {
    let mut fx = launched_waiting(false);
    let since = fx.run().orch.first_turn_since.expect("set by the launch");
    let effects = fx.send(
        since + FIRST_TURN_WAIT_SECS - 1,
        crate::run::engine::EventKind::Tick,
    );
    assert_eq!(wakes(&effects), vec![]);
    assert_eq!(start_prompts(&fx), 0, "not yet late");
    let effects = fx.send(
        since + FIRST_TURN_WAIT_SECS,
        crate::run::engine::EventKind::Tick,
    );
    assert_eq!(wakes(&effects), vec![], "nothing is pasted");
    assert!(fx.run().orch.first_turn_late);
    assert_eq!(start_prompts(&fx), 1, "{:?}", attention(fx.run(), fx.now));
    let line = "first turn still waiting 60 s after launch: no MCP ready notice yet";
    let logged = |fx: &Fixture| fx.run().log.iter().filter(|e| e.text == line).count();
    assert_eq!(logged(&fx), 1, "{:#?}", fx.run().log);
    // Later ticks say nothing more.
    fx.tick();
    assert_eq!((logged(&fx), start_prompts(&fx)), (1, 1));
    // The notice comes: the wake-up goes; its paste clears the attention line.
    let effects = mcp_ready(&mut fx, ORCH);
    assert_eq!(wakes(&effects), vec![(first_prompt(&fx), true)]);
    assert_eq!(start_prompts(&fx), 1, "until it is pasted");
    first_turn_woken(&mut fx);
    assert!(!fx.run().orch.first_turn_late);
    assert_eq!(start_prompts(&fx), 0);
}

/// The bound's other reason: the notice came, but the driver still holds the paste.
#[test]
fn past_the_bound_with_the_notice_the_window_is_not_ready() {
    let mut fx = launched_waiting(false);
    let since = fx.run().orch.first_turn_since.unwrap();
    mcp_ready(&mut fx, ORCH);
    fx.send(
        since + FIRST_TURN_WAIT_SECS,
        crate::run::engine::EventKind::Tick,
    );
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text
                == "first turn still waiting 60 s after launch: the window is not ready yet"),
        "{:#?}",
        fx.run().log
    );
    assert_eq!(start_prompts(&fx), 1);
}

#[test]
fn a_relaunch_before_the_first_turn_delivers_it_after_the_new_servers_notice() {
    let mut fx = launched_waiting(false);
    let first = first_prompt(&fx);
    // The old session's server announced its tools; the paste never happened.
    mcp_ready(&mut fx, ORCH);
    // What `run.json` keeps: the pending first turn, nothing of the old server.
    let json = serde_json::to_string(fx.run()).unwrap();
    let back: crate::run::model::Run = serde_json::from_str(&json).unwrap();
    assert!(back.orch.orchestrator.as_ref().unwrap().first_turn_pending);
    assert_eq!(
        (back.orch.mcp_ready, back.orch.first_turn_since),
        (false, None)
    );
    restart(&mut fx);
    assert!(pending(&fx), "persisted");
    assert!(!fx.run().orch.mcp_ready, "not persisted");
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    assert!(!fx.run().orch.mcp_ready, "reset by the relaunch");
    assert!(fx.run().orch.first_turn_since.is_some());
    let effects = fx.done(restarts[0].0, OpResult::Restarted);
    assert_eq!(wakes(&effects), vec![], "the restart's note waits too");
    // The new session's server: the first prompt, before any note.
    let effects = mcp_ready(&mut fx, ORCH);
    assert_eq!(wakes(&effects), vec![(first, true)]);
    let effects = first_turn_woken(&mut fx);
    let after = wakes(&effects);
    assert_eq!(after.len(), 1, "{effects:#?}");
    assert!(
        after[0]
            .0
            .contains("the daemon restarted and your session was resumed"),
        "{after:?}"
    );
}
