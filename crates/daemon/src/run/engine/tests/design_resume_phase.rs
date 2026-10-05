//! Ruling WB-A-I1 (the final fix wave's FW-3): a design run's `run resume` acts only on
//! the phase the run left, in the current round. It relaunches brainstormers only when
//! the run left Brainstorming and they are the current round's; it restarts that
//! phase's clock only when the phase has one (none at an open gate, none before the
//! drafts are in); and a dropped round's design agents leave the live set.

use proto::{RoundDesign, RunState};

use super::design_agents::{CLAUDE, CODEX, launches, started};
use super::design_fixture::*;
use super::design_rounds_commit::pr_complete;
use super::design_rounds_fixture::*;
use super::fixture::*;
use super::goal_rounds_start::reply;
use super::orch_restore::resume;
use crate::run::design::state::DesignAgentState;
use crate::run::engine::EventKind;

const RELAUNCHED: &str = "resumed; the brainstormers are relaunched";

/// A full round's brainstormers, both started in their windows.
fn brainstorming(fx: &mut Fixture) {
    assert_eq!(fx.run().state, RunState::Brainstorming);
    start_brainstorm(fx);
    let labels: Vec<String> = (launches(fx).iter().rev().take(2))
        .map(|(_, s)| s.kind.label())
        .collect();
    for (label, window) in labels.iter().zip([CLAUDE, CODEX]) {
        started(fx, label, window);
    }
    let design = fx.run().orch.design.as_ref().unwrap();
    let states: Vec<&DesignAgentState> = design.brainstormers.iter().map(|a| &a.state).collect();
    assert_eq!(
        states,
        [&DesignAgentState::Running, &DesignAgentState::Running]
    );
}

/// Review A's I-1: round 2 (`full`) rejected while it brainstorms; round 3 (`amend`)
/// halted by its specifying budget. The resume restarts the specifying clock and
/// queues nothing: round 2's brainstormers are gone from the live set.
#[test]
fn a_resume_after_a_dropped_full_round_restarts_its_own_clock() {
    let mut fx = design_complete();
    iterate_with(&mut fx, Some(RoundDesign::Full)).unwrap();
    brainstorming(&mut fx);
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    });
    assert!(reply(&effects).is_ok(), "{effects:?}");
    assert_eq!(fx.run().state, RunState::Complete);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(
        design.brainstormers.is_empty(),
        "{:?}",
        design.brainstormers
    );
    iterate_with(&mut fx, None).unwrap();
    assert_eq!(fx.run().state, RunState::Specifying);
    let late = fx.now + u64::from(fx.run().limits.orch.design.phase_minutes) * 60 + 1;
    fx.send(late, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    resume(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Specifying);
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(design.phase_started, Some(fx.now), "the clock restarts");
    assert!(design.brainstormers.is_empty(), "nothing is queued");
    let lines = log_lines(&fx);
    assert!(!lines.iter().any(|l| l == RELAUNCHED), "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l == "resumed; the phase's clock restarts")
    );
}

/// Review A's I-1, the second way: a `pr` round halted in Brainstorming by a delivery
/// result while both brainstormers run. The resume starts no clock before the drafts
/// are in (decision 8) and relaunches nothing.
#[test]
fn a_brainstorming_halt_resumes_with_no_clock_before_the_drafts() {
    let mut fx = pr_complete();
    iterate_with(&mut fx, Some(RoundDesign::Full)).unwrap();
    brainstorming(&mut fx);
    let now = fx.now;
    let run = fx.run_mut();
    crate::run::engine::merge::halt(run, "remote stage branch moved".into(), now);
    run.halt_retryable = true;
    let starts = launches(&fx).len();
    resume(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Brainstorming);
    let design = run.orch.design.as_ref().unwrap();
    assert_eq!(design.phase_started, None, "no clock before the drafts");
    let states: Vec<&DesignAgentState> = design.brainstormers.iter().map(|a| &a.state).collect();
    assert_eq!(
        states,
        [&DesignAgentState::Running, &DesignAgentState::Running]
    );
    assert_eq!(launches(&fx).len(), starts, "nothing relaunched");
    let lines = log_lines(&fx);
    assert!(!lines.iter().any(|l| l == RELAUNCHED), "{lines:#?}");
    assert!(
        lines
            .iter()
            .any(|l| l == "resumed; the brainstorm waits for its drafts"),
        "{lines:#?}"
    );
}
