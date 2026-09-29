//! Milestone 9 task M9.13, engine side: the driver's reports of the orchestrator's
//! window (decision 13: it exited, or it is back), its OTLP token (decision 14a), and
//! decision 13's attention line for a launch that failed.

use proto::RunState;

use super::dispatch::edit;
use super::fixture::*;
use super::orch::{ORCH, add, launched, planned};
use crate::run::engine::{Effect, EventKind, OpResult, OrchEvent};
use crate::run::snapshot::attention;

/// The driver's report, made with the record as it is now.
fn window(fx: &mut Fixture, window_id: u32, live: bool) -> Vec<Effect> {
    let launch = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    window_at(fx, window_id, live, launch)
}

/// The driver's report, made when the record's `launches` was `launch`.
fn window_at(fx: &mut Fixture, window_id: u32, live: bool, launch: u64) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::OrchestratorWindow {
        run_id: RUN_ID.into(),
        window_id,
        live,
        launch,
    }))
}

fn token(fx: &mut Fixture, token: &str) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::OtlpToken {
        run_id: RUN_ID.into(),
        token: token.into(),
    }))
}

fn wakes(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::WakeOrchestrator { .. }))
        .count()
}

/// A user edit: it adds a wake note (decision 39), so a live orchestrator is woken.
fn user_edit(fx: &mut Fixture) -> Vec<Effect> {
    let n = fx.run().tasks.len();
    let edit_json = add(&format!("t{n}"), &format!("m{n}"));
    edit(fx, vec![serde_json::from_value(edit_json).unwrap()])
}

const EXITED: &str = "the orchestrator (window 90) exited; restart it with anthrex restart 90";

#[test]
fn an_exited_orchestrator_is_not_live_the_run_says_so_and_nothing_wakes_it() {
    let mut fx = launched(false);
    let first = user_edit(&mut fx);
    assert!(
        first
            .iter()
            .any(|e| matches!(e, Effect::WakeOrchestrator { .. })),
        "{first:#?}"
    );
    window(&mut fx, ORCH, false);
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert!(!o.live);
    assert!(o.exited_at.is_some());
    assert!(
        attention(fx.run(), fx.now).contains(&EXITED.to_string()),
        "{:?}",
        attention(fx.run(), fx.now)
    );
    // Wakes are suspended; the note waits in the digest.
    let effects = user_edit(&mut fx);
    assert_eq!(wakes(&effects), 0, "{effects:#?}");
    assert!(
        !fx.run()
            .orch
            .orchestrator
            .as_ref()
            .unwrap()
            .notes
            .is_empty()
    );
    // The run itself goes on.
    assert_eq!(fx.run().state, RunState::Planning);
}

#[test]
fn a_window_that_is_back_is_live_again_and_the_line_goes() {
    let mut fx = launched(false);
    window(&mut fx, ORCH, false);
    assert_eq!(wakes(&user_edit(&mut fx)), 0);
    let effects = window(&mut fx, ORCH, true);
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert!(o.live);
    assert_eq!(o.exited_at, None);
    assert!(!attention(fx.run(), fx.now).contains(&EXITED.to_string()));
    // Its pending notes wake it now.
    assert_eq!(wakes(&effects), 1, "{effects:#?}");
}

#[test]
fn another_windows_report_changes_nothing() {
    let mut fx = launched(false);
    let before = fx.run().clone();
    window(&mut fx, ORCH + 1, false);
    assert_eq!(fx.run().orch.orchestrator, before.orch.orchestrator);
    // A terminal run's orchestrator is never made live again.
    let mut fx = launched(false);
    window(&mut fx, ORCH, false);
    fx.run_mut().state = RunState::Discarded;
    window(&mut fx, ORCH, true);
    assert!(!fx.run().orch.orchestrator.as_ref().unwrap().live);
    assert!(
        !attention(fx.run(), fx.now).contains(&EXITED.to_string()),
        "a terminal run's exited orchestrator is no longer the user's concern"
    );
}

#[test]
fn a_failed_launch_is_an_attention_line_until_it_starts() {
    let mut fx = planned(false);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Failed {
            message: "no such binary".into(),
        },
    );
    let line = "the orchestrator could not start: no such binary".to_string();
    assert!(
        attention(fx.run(), fx.now).contains(&line),
        "{:?}",
        attention(fx.run(), fx.now)
    );
    // `run resume` launches it again; once it starts, the line goes.
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    assert!(
        !attention(fx.run(), fx.now).contains(&line),
        "{:?}",
        attention(fx.run(), fx.now)
    );
}

#[test]
fn the_otlp_token_is_set_once_and_persisted() {
    let mut fx = launched(false);
    let effects = token(&mut fx, "0123456789abcdef0123456789abcdef");
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert_eq!(o.otlp_token, "0123456789abcdef0123456789abcdef");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Persist { urgent: true, .. })),
        "{effects:#?}"
    );
    // A second token never replaces the first: a restart re-passes the same one.
    token(&mut fx, "ffffffffffffffffffffffffffffffff");
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert_eq!(o.otlp_token, "0123456789abcdef0123456789abcdef");
    // It is never in the snapshot.
    let snap = serde_json::to_string(&crate::run::snapshot::snapshot(&fx.state, 2_000)).unwrap();
    assert!(!snap.contains("0123456789abcdef0123456789abcdef"));
}

/// M9.13 review, item 3: an exit the driver saw before `run resume`'s `Restarted` was
/// applied, reaching the engine after it, is dropped; one seen after it counts.
#[test]
fn a_stale_exit_from_before_the_restart_changes_nothing() {
    let mut fx = launched(false);
    let before = fx.run().orch.orchestrator.as_ref().unwrap().launches;
    assert_eq!(before, 1, "one launch");
    window(&mut fx, ORCH, false);
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    let (op, _) = fx.op("RestartOrchestrator");
    fx.done(op, OpResult::Restarted);
    assert!(fx.run().orch.orchestrator.as_ref().unwrap().live);
    // The stale report: made at `before`, applied now.
    window_at(&mut fx, ORCH, false, before);
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert!(o.live, "a stale exit made it not live");
    assert!(!attention(fx.run(), fx.now).contains(&EXITED.to_string()));
    assert_eq!(wakes(&user_edit(&mut fx)), 1, "wakes still reach it");
    // A report made after the restart counts.
    window(&mut fx, ORCH, false);
    assert!(!fx.run().orch.orchestrator.as_ref().unwrap().live);
}
