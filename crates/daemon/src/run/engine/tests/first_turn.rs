//! Milestone 9.5 decision 38 (task M9.5.5a, review ruling C1), engine side: the
//! orchestrator launches with no prompt, and its first turn goes out through the
//! whole-paste wake-up once its anthrex server has announced its tools; past
//! `FIRST_TURN_WAIT_SECS` the run says so and keeps waiting.

use super::fixture::*;
use super::orch::launched;
use super::orch::{ORCH, add, first_turn_woken, launched_waiting, mcp_ready, planned, planned_on};
use super::orch_restore::{restart, resume};
use crate::run::engine::first_turn::{FIRST_TURN_WAIT_SECS, MCP_READY_GRACE_SECS};
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent};
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
    // Fix round 1 (m2): the driver starts a pending first turn's session fresh.
    let effects = fx.done(restarts[0].0, OpResult::RestartedFresh);
    assert_eq!(wakes(&effects), vec![], "nothing before the first turn");
    // The new session's server: the first prompt, before any note.
    let effects = mcp_ready(&mut fx, ORCH);
    assert_eq!(wakes(&effects), vec![(first, true)]);
    first_turn_woken(&mut fx);
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert!(
        !o.notes
            .iter()
            .any(|n| n.contains("your session was resumed")),
        "untrue of a fresh session: {:?}",
        o.notes
    );
}

/// The driver's report that window `window` has sent its first signal, made when the
/// record's `launches` was `launch`.
fn first_signal(fx: &mut Fixture, now: u64, window: u32, launch: u64) -> Vec<Effect> {
    fx.send(
        now,
        EventKind::Orch(OrchEvent::FirstSignal {
            run_id: RUN_ID.into(),
            window_id: window,
            launch,
        }),
    )
}

/// Fix round 1, ruling T5a-1: a window that has sent a signal waits for its server's
/// notice at most `MCP_READY_GRACE_SECS`; then its first turn goes anyway, and the run
/// says so. A report about another window or an earlier launch starts no grace.
#[test]
fn a_signalled_window_gets_its_first_turn_after_the_mcp_ready_grace() {
    let mut fx = launched_waiting(false);
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    let t = fx.now + 1;
    first_signal(&mut fx, t, ORCH + 1, launch);
    first_signal(&mut fx, t, ORCH, launch + 1);
    assert_eq!(fx.run().orch.first_signal_at, None, "not this window's");
    first_signal(&mut fx, t, ORCH, launch);
    assert_eq!(fx.run().orch.first_signal_at, Some(t));
    // A later report keeps the first.
    first_signal(&mut fx, t + 5, ORCH, launch);
    assert_eq!(fx.run().orch.first_signal_at, Some(t));
    let line = "first turn sent without the MCP ready notice after 30 s";
    let logged = |fx: &Fixture| fx.run().log.iter().filter(|e| e.text == line).count();
    let effects = fx.send(t + MCP_READY_GRACE_SECS - 1, EventKind::Tick);
    assert_eq!(wakes(&effects), vec![], "29 s: nothing yet");
    assert_eq!(logged(&fx), 0);
    let effects = fx.send(t + MCP_READY_GRACE_SECS, EventKind::Tick);
    assert_eq!(wakes(&effects), vec![(first_prompt(&fx), true)], "30 s");
    assert_eq!(logged(&fx), 1, "{:#?}", fx.run().log);
    fx.tick();
    assert_eq!(logged(&fx), 1, "once");
    first_turn_woken(&mut fx);
    assert!(!pending(&fx));
}

/// With the notice in time, the grace says nothing.
#[test]
fn the_notice_inside_the_grace_sends_the_first_turn_without_the_line() {
    let mut fx = launched_waiting(false);
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    let t = fx.now + 1;
    first_signal(&mut fx, t, ORCH, launch);
    mcp_ready(&mut fx, ORCH);
    fx.send(t + MCP_READY_GRACE_SECS, EventKind::Tick);
    assert!(
        !fx.run()
            .log
            .iter()
            .any(|e| e.text.starts_with("first turn sent without")),
        "{:#?}",
        fx.run().log
    );
}

/// Fix round 1 (m1): a restart that could not resume its session (no session id) is a
/// fresh session, which needs the first prompt again; the restart's "resumed" note is
/// untrue of it.
#[test]
fn a_restart_that_cannot_resume_sends_the_first_prompt_again() {
    let mut fx = launched(false);
    let first = first_prompt(&fx);
    assert!(!pending(&fx), "delivered once");
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    let effects = fx.done(restarts[0].0, OpResult::RestartedFresh);
    assert!(pending(&fx), "the fresh session waits for its first turn");
    assert!(fx.run().orch.first_turn_since.is_some());
    assert_eq!(wakes(&effects), vec![]);
    let effects = mcp_ready(&mut fx, ORCH);
    assert_eq!(wakes(&effects), vec![(first, true)]);
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert!(
        !o.notes
            .iter()
            .any(|n| n.contains("your session was resumed")),
        "{:?}",
        o.notes
    );
}

/// Fix round 1 (m4): a notice that comes while the restart is in flight is the old
/// session's and arms nothing; the new session's own notice does.
#[test]
fn a_notice_during_the_restart_is_the_old_sessions() {
    let mut fx = launched_waiting(false);
    let first = first_prompt(&fx);
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    assert_eq!(wakes(&mcp_ready(&mut fx, ORCH)), vec![]);
    let effects = fx.done(restarts[0].0, OpResult::RestartedFresh);
    assert_eq!(wakes(&effects), vec![], "the old notice armed nothing");
    assert!(!fx.run().orch.mcp_ready);
    let effects = mcp_ready(&mut fx, ORCH);
    assert_eq!(wakes(&effects), vec![(first, true)]);
}

/// Ruling T5a-2: the first-turn gate is Claude's only. A Codex orchestrator keeps 9.3's
/// launch: its first prompt on the command line, nothing pending, nothing pasted, and a
/// restart (fresh or resumed) as in 9.3.
#[test]
fn a_codex_orchestrator_gets_its_first_prompt_on_the_command_line() {
    let mut fx = planned_on(false, Some(proto::Runtime::Codex));
    let (op, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, proto::Runtime::Codex);
    assert_eq!(
        spec.initial_prompt.as_deref(),
        Some(orchestrator_first_prompt(fx.run()).as_str())
    );
    assert!(!pending(&fx));
    assert_eq!(fx.run().orch.first_turn_since, None);
    let (branch, _) = fx.op("CreateRunBranch");
    fx.done(branch, OpResult::Worktree { head: BASE.into() });
    let effects = fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    let mut all = wakes(&effects);
    all.extend(wakes(&mcp_ready(&mut fx, ORCH)));
    let now = fx.now + 1;
    all.extend(wakes(&first_signal(&mut fx, now, ORCH, 1)));
    let late = fx.now + FIRST_TURN_WAIT_SECS + MCP_READY_GRACE_SECS;
    all.extend(wakes(&fx.send(late, EventKind::Tick)));
    assert!(all.iter().all(|(_, first)| !first), "{all:#?}");
    assert_eq!(start_prompts(&fx), 0);
    assert!(
        !fx.run()
            .log
            .iter()
            .any(|e| e.text.starts_with("first turn"))
    );
    // A restart that could not resume is 9.3's restart: no first turn pending.
    restart(&mut fx);
    let effects = resume(&mut fx);
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    let effects = fx.done(restarts[0].0, OpResult::RestartedFresh);
    assert!(!pending(&fx));
    assert!(wakes(&effects).iter().all(|(_, first)| !first));
}

/// Ruling T5a-2: a Claude orchestrator chosen by name still waits for its first turn.
#[test]
fn a_claude_orchestrator_still_waits_for_its_first_turn() {
    let fx = planned_on(false, Some(proto::Runtime::Claude));
    let (_, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, proto::Runtime::Claude);
    assert_eq!(spec.initial_prompt, None);
    assert!(pending(&fx));
}
