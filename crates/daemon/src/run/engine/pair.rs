//! Milestone 9.5 decisions 25 and 26: a paired task's test writer, then its
//! implementer, in one checkout and one writer slot. The pair starts at dispatch with a
//! test writer on the peer runtime; the writer's claim must end on its red commit; a
//! red-only proof (ruling RP-1) then confirms the test fails there, which retires the
//! writer and starts the implementer, whose claim inherits the test and red. Every
//! other step is a worker's: the ladder, `run retry` and a restart act on the session
//! of the phase the task is in (`model::writes_task`). Pure (design decision 1).

use proto::{AgentRole, GateKind, PairPhase, Route, Runtime, TaskState};

use super::dispatch::{history, launch_implementer};
use super::signals::end_round;
use super::tools::DoneArgs;
use super::{Effect, clock, ladder, requests};
use crate::run::contract_patterns::{
    implementer_wrong_test, red_check_failed_message, red_confirmed_line, writer_not_on_red,
};
use crate::run::model::{Pair, Run, Task};
use crate::run::phases::set_state;
use crate::run::validate_patterns::peer_route;

/// Whether task `task` is a paired task whose test writer is (or will be) at work.
pub(crate) fn writing(task: &Task) -> bool {
    task.pair
        .as_ref()
        .is_some_and(|p| p.phase == PairPhase::Writing)
}

/// The test and red commit a paired task's implementer works against, once confirmed.
fn handed(task: &Task) -> Option<(&str, &str)> {
    let pair = task
        .pair
        .as_ref()
        .filter(|p| p.phase == PairPhase::Implementing)?;
    Some((pair.test.as_deref()?, pair.red.as_deref()?))
}

/// Decision 25, at dispatch: a paired task starts in `Writing`, its test writer on the
/// peer runtime at the task's strength and effort when the roster has one (after the
/// installed skip), else on the task's own route. A task already paired keeps its pair
/// (a retry that dispatches it again resumes the phase it was in).
pub(super) fn begin(run: &mut Run, i: usize) {
    let task = &run.tasks[i];
    if !task.spec.pair || task.pair.is_some() {
        return;
    }
    let writer_route = peer_route(&run.roster, &task.route, &run.orch.installed)
        .unwrap_or_else(|| task.route.clone());
    run.tasks[i].pair = Some(Pair {
        phase: PairPhase::Writing,
        writer_route,
        test: None,
        red: None,
        red_checked: None,
        writer_failures: 0,
        writer_sessions: 0,
    });
}

/// The role and route of task `task`'s next writing session.
pub(super) fn next_session(task: &Task) -> (AgentRole, Route) {
    match task.pair.as_ref().filter(|_| writing(task)) {
        Some(pair) => (AgentRole::TestWriter, pair.writer_route.clone()),
        None => (AgentRole::Worker, task.route.clone()),
    }
}

/// A test writer session of task `i` is being launched (its session number counted).
pub(super) fn writer_launched(run: &mut Run, i: usize) {
    if let Some(pair) = run.tasks[i].pair.as_mut() {
        pair.writer_sessions += 1;
    }
}

/// Decision 38's rung 2 (and `run retry`) while the test writer works: the fresh
/// session is a test writer on `roster::escalate` of its route; the implementer's route
/// is left as it is. Returns whether it applied.
pub(super) fn escalate_writer(run: &mut Run, i: usize) -> bool {
    if !writing(&run.tasks[i]) {
        return false;
    }
    let roster = &run.roster;
    if let Some(pair) = run.tasks[i].pair.as_mut() {
        pair.writer_route = crate::run::roster::escalate(roster, &pair.writer_route);
    }
    true
}

/// Decision 25: a test writer's claim must name its last commit as red (a prefix of its
/// head that git would take, at least four hex digits); the rejection, else `None`.
pub(super) fn writer_rejection(task: &Task, red: Option<&str>, head: &str) -> Option<String> {
    let on_head = red.is_some_and(|r| r.len() >= 4 && head.starts_with(&r.to_ascii_lowercase()));
    (writing(task) && !on_head).then(|| writer_not_on_red(head, red))
}

/// Decision 26: an implementer's claim that names another test or red than the test
/// writer's is refused.
pub(super) fn implementer_mismatch(task: &Task, args: &DoneArgs) -> Option<String> {
    let (test, red) = handed(task)?;
    let red_ok = (args.red.as_deref()).is_none_or(|r| r.len() >= 4 && red.starts_with(r));
    let test_ok = args.test.as_deref().is_none_or(|t| t == test);
    (!(red_ok && test_ok)).then(|| implementer_wrong_test(test, red))
}

/// Decision 26: an implementer's claim takes the writer's test and red where it named
/// none (the turn-end fallback's included).
pub(super) fn fill(task: &Task, args: DoneArgs) -> DoneArgs {
    let Some((test, red)) = handed(task) else {
        return args;
    };
    DoneArgs {
        test: args.test.or_else(|| Some(test.to_string())),
        red: args.red.or_else(|| Some(red.to_string())),
        ..args
    }
}

/// Decision 25: the red-only proof's result for task `i`. A test that passed at red is
/// a gate failure of `proof` for the test writer; one that failed, as it should, hands
/// the red commit to a fresh implementer in the same checkout.
#[allow(clippy::too_many_arguments)]
pub(super) fn on_red_checked(
    run: &mut Run,
    i: usize,
    red_failed: bool,
    tail: &str,
    command: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let claim = run.tasks[i].done.as_ref();
    let test = claim.and_then(|c| c.test.clone()).unwrap_or_default();
    let red = claim.and_then(|c| c.red.clone()).unwrap_or_default();
    if !red_failed {
        let text = red_check_failed_message(&red, &test, command, tail);
        ladder::gate_failure(run, i, GateKind::Proof, text, false, now, fx);
        return;
    }
    retire_writer(run, i, now, fx);
    let task = &mut run.tasks[i];
    let writer_failures = task.failures;
    if let Some(pair) = task.pair.as_mut() {
        pair.phase = PairPhase::Implementing;
        pair.test = Some(test);
        pair.red = Some(red.clone());
        pair.red_checked = Some(true);
        pair.writer_failures = writer_failures;
    }
    // The implementer starts afresh: the writer's counters are the pair's now.
    task.failures = 0;
    task.bounces = Default::default();
    task.stalls = 0;
    task.budget_exceeded = 0;
    task.rung = 0;
    task.done = None;
    task.head = None;
    clock::new_epoch(task);
    set_state(task, TaskState::Working, now);
    let line = red_confirmed_line(task.id(), &red);
    history(run, i, now, line.clone());
    requests::log(run, now, line);
    launch_implementer(run, i, now, fx);
}

/// The test writer's session is done with: retired, as a merged task's worker is.
fn retire_writer(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let writers = (run.tasks[i].rounds.iter_mut())
        .filter(|r| r.role == AgentRole::TestWriter && !r.ended && !r.retiring);
    for round in writers {
        round.retiring = true;
        if let Some(window_id) = round.window_id {
            fx.push(Effect::RetireWindow { window_id });
        }
        // A Codex session between turns has no process to wait for (one per turn).
        if round.route.runtime == Runtime::Codex && !round.turn_open && round.pid.is_none() {
            end_round(round, now);
        }
    }
}
