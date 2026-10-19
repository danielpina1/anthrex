//! Review round 1 of the host-sleep fix (M1, M3, M4): each of `clock::slept`'s shifts
//! not covered by `host_sleep.rs`, one test each. A failed turn's continue, a count
//! retry, a design phase's budget, a design agent's session seconds and a stopped
//! lane's exit wait count awake time; a stopped clock (a task's or a racing lane's)
//! is not excused the sleep a second time.

use proto::{AgentRole, DocGateAction, DocGateKind, RaceLane, RunState, TokenUsage};

use super::control::blocked;
use super::design_agents::{agents, brainstorming};
use super::design_fixture::{act, at_spec_gate};
use super::fixture::*;
use super::liveness::assert_alive;
use super::race::{lane, racing};
use super::race_end::removal;
use super::race_lanes::{HEAD_B, passes};
use super::turns::{killed_exit, working};
use crate::run::design::state::DesignAgentState;
use crate::run::engine::clock;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::FailedTurn;

/// A host sleep longer than every wait below.
const SLEPT: u64 = 3 * 3_600;

fn sleep(fx: &mut Fixture, secs: u64) -> Vec<Effect> {
    let now = fx.now + secs;
    fx.send(now, EventKind::Slept { secs })
}

/// A failed turn's continue (decision 32) waits its seconds awake: not sent at the
/// first tick after a sleep, sent once the wait has passed awake.
#[test]
fn a_sleep_does_not_send_a_failed_turns_continue_early() {
    let (mut fx, _window) = working();
    let at = fx.now + 30;
    let round = &mut fx.run_mut().tasks[0].rounds[0];
    round.failed_turn = FailedTurn::WaitingContinue {
        at,
        rate_limit: false,
    };
    round.failed_error = Some("overloaded".into());
    sleep(&mut fx, SLEPT);
    fx.tick();
    let round = &fx.task("t1").rounds[0];
    let waiting = FailedTurn::WaitingContinue {
        at: at + SLEPT,
        rate_limit: false,
    };
    assert_eq!(round.failed_turn, waiting);
    fx.send(at + SLEPT, EventKind::Tick);
    let round = &fx.task("t1").rounds[0];
    assert!(
        matches!(round.failed_turn, FailedTurn::ContinueSent { .. }),
        "{round:#?}"
    );
    assert_alive(&fx);
}

/// A failed `CountCommits`' retry (ruling T12-A2) waits its seconds awake.
#[test]
fn a_sleep_does_not_retry_a_count_early() {
    let (mut fx, _window) = working();
    let at = fx.now + 10;
    fx.run_mut().tasks[0].rounds[0].count_retry_at = Some(at);
    sleep(&mut fx, SLEPT);
    let effects = fx.tick();
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").rounds[0].count_retry_at, Some(at + SLEPT));
    assert_alive(&fx);
}

/// A blocked task with no turn open has its clock stopped: the stop excuses the sleep
/// already, so `slept` excuses nothing more, and a restart excuses the stop once.
#[test]
fn a_stopped_clock_is_not_excused_the_sleep_twice() {
    let (mut fx, window) = working();
    fx.now += 200;
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    assert!(fx.task("t1").clock.stopped.is_some(), "the clock stopped");
    let excused = fx.task("t1").rounds[0].excused_secs;
    let now = fx.now + SLEPT;
    clock::slept(fx.run_mut(), SLEPT, now);
    assert_eq!(fx.task("t1").rounds[0].excused_secs, excused);
}

/// A racing lane's rounds are charged on the lane's clock (`race_view`): a lane whose
/// clock is stopped is not excused the sleep; the other lane, whose clock runs, is.
#[test]
fn a_lane_with_its_clock_stopped_is_not_excused_the_sleep() {
    let (mut fx, _, _) = racing();
    let since = fx.now;
    let race = fx.task_mut("t1").race.as_mut().expect("a race");
    let a = (race.lanes.iter_mut())
        .find(|l| l.lane == RaceLane::A)
        .unwrap();
    a.gates.clock.stopped = Some(since);
    let excused = |fx: &Fixture, lane: RaceLane| {
        (fx.task("t1").rounds.iter())
            .find(|r| r.role == AgentRole::Racer && r.lane == Some(lane))
            .map(|r| r.excused_secs)
            .expect("the lane's racer")
    };
    let (a0, b0) = (excused(&fx, RaceLane::A), excused(&fx, RaceLane::B));
    let now = fx.now + SLEPT;
    clock::slept(fx.run_mut(), SLEPT, now);
    assert_eq!(excused(&fx, RaceLane::A), a0, "lane a's clock is stopped");
    assert_eq!(excused(&fx, RaceLane::B), b0 + SLEPT, "lane b's runs");
}

/// A design phase's budget (milestone 9.6 decision 8) counts awake time: a sleep past
/// the planning phase's 60 minutes does not halt the run; the awake minutes do.
#[test]
fn a_sleep_does_not_spend_a_design_phases_budget() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    let start = fx.now;
    sleep(&mut fx, SLEPT);
    fx.tick();
    assert_eq!(
        fx.run().state,
        RunState::Planning,
        "{:?}",
        fx.run().halted_reason
    );
    fx.send(start + SLEPT + 60 * 60 + 1, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
}

/// Review M4: a design agent's session seconds (`design_spend::session_ended`) count
/// awake time, so a session that fails after a sleep is recorded as failed, with its
/// awake seconds, not as over budget.
#[test]
fn a_sleep_is_not_charged_to_a_design_agents_session() {
    let mut fx = brainstorming();
    let agent = agents(&fx)[0].clone();
    assert_eq!(agent.state, DesignAgentState::Running);
    let started = agent.started.expect("a started brainstormer");
    sleep(&mut fx, SLEPT);
    let agent = agents(&fx)[0].clone();
    assert_eq!(agent.started, Some(started + SLEPT));
    let (round, now) = (fx.run().round(), fx.now);
    let usage = TokenUsage::default();
    let failure = Some("its process exited");
    crate::run::engine::design_spend::session_ended(
        fx.run_mut(),
        &agent,
        (0, &usage),
        failure,
        now,
    );
    let design = fx.run().orch.design.as_ref().unwrap();
    let spend = design.phase_spend(round, "brainstorming").unwrap();
    let session = (spend.agents.iter())
        .find(|a| a.label == agent.label)
        .unwrap();
    assert_eq!(session.outcome, "failed: its process exited");
    assert!(session.secs <= now - SLEPT - started, "{}", session.secs);
}

/// Review M1: a stopped racer's exit wait (`LANE_EXIT_WAIT_SECS` after its kill) counts
/// awake time. A racer that exits after a sleep is salvaged as one that exited: its
/// checkout removed and its locks cleared, with no "its racer did not exit".
#[test]
fn a_sleep_between_a_lanes_kill_and_its_exit_salvages_normally() {
    let (mut fx, a, _) = racing();
    passes(&mut fx, RaceLane::B, HEAD_B);
    assert!(
        lane(&fx, RaceLane::A).kill_sent_at.is_some(),
        "lane a was stopped"
    );
    let mut effects = sleep(&mut fx, SLEPT);
    effects.extend(fx.tick());
    assert!(
        ops_in(&effects, "RemoveWorktree").is_empty(),
        "{effects:#?}"
    );
    let effects = killed_exit(&mut fx, a);
    let kinds: Vec<_> = ops_in(&effects, "RemoveWorktree")
        .into_iter()
        .map(|(_, kind)| kind)
        .collect();
    assert_eq!(kinds, [removal(RaceLane::A, 1, false)]);
    let line = "race t1: kept a checkout: its racer did not exit";
    assert!(
        !fx.run().log.iter().any(|l| l.text == line),
        "{:#?}",
        fx.run().log
    );
}
