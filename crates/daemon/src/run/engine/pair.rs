//! Milestone 9.5 decisions 25 and 26: a paired task's test writer, then its
//! implementer, in one checkout and one writer slot. The pair starts at dispatch with a
//! test writer on the peer runtime; the writer's claim must end on its red commit; a
//! red-only proof (ruling RP-1) then confirms the test fails there, which retires the
//! writer and starts the implementer, whose claim inherits the test and red. Every
//! other step is a worker's: the ladder, `run retry` and a restart act on the session
//! of the phase the task is in (`model::writes_task`). Pure (design decision 1).

use proto::{AgentRole, GateKind, PairPhase, Route, Runtime, TaskState};

use super::dispatch::{history, launch_implementer};
use super::schedule::op_in_flight;
use super::signals::end_round;
use super::tools::DoneArgs;
use super::{Effect, OpKind, clock, ladder, requests};
use crate::run::contract_patterns::{
    implementer_wrong_test, red_check_failed_message, red_confirmed_line, writer_not_on_red,
    writer_paths_unlimited_line,
};
use crate::run::model::{Pair, ProofRecord, Run, Task};
use crate::run::phases::set_state;
use crate::run::route_pick::{writer_route, writer_route_failed, writer_step};

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
/// peer runtime at the task's strength and effort when the roster has one, after the
/// workers' installed and overlap skips (ruling T16-2, `route_pick::writer_route`),
/// else on the task's own route. A task already paired keeps its pair (a retry that
/// dispatches it again resumes the phase it was in).
pub(super) fn begin(run: &mut Run, i: usize) {
    let task = &run.tasks[i];
    if !task.spec.pair || task.pair.is_some() {
        return;
    }
    let writer_route = writer_route(run, i);
    run.tasks[i].pair = Some(Pair {
        phase: PairPhase::Writing,
        writer_route,
        test: None,
        red: None,
        red_checked: None,
        writer_failures: 0,
        writer_sessions: 0,
        escalated_from: None,
        writer_signals: Vec::new(),
        writer_signals_more: 0,
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
/// session is a test writer on the workers' skipping ladder from its route (ruling
/// T16-2, `route_pick::writer_step`, with the every-route-failed line); the
/// implementer's route is left as it is. Returns whether it applied.
pub(super) fn escalate_writer(run: &mut Run, i: usize, now: u64) -> bool {
    let Some(current) = (run.tasks[i].pair.as_ref())
        .filter(|p| p.phase == PairPhase::Writing)
        .map(|p| p.writer_route.clone())
    else {
        return false;
    };
    let next = writer_step(run, i, &current);
    if let Some(text) = writer_route_failed(run, i, &current, &next) {
        requests::log(run, now, text);
    }
    if let Some(pair) = run.tasks[i].pair.as_mut() {
        pair.writer_route = next;
        pair.escalated_from = Some(current);
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
    let red_ok = (args.red.as_deref())
        .is_none_or(|r| r.len() >= 4 && red.starts_with(&r.to_ascii_lowercase()));
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
/// the red commit to a fresh implementer in the same checkout, which the running pass
/// launches ([`launch_due`]; ruling T16-3: never while the run is paused or halted).
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
    let task = &run.tasks[i];
    let claim = task.done.as_ref();
    let test = claim.and_then(|c| c.test.clone()).unwrap_or_default();
    let red = claim.and_then(|c| c.red.clone()).unwrap_or_default();
    // Minor m3: the red check is on record, marked red-only.
    let record = ProofRecord {
        at: now,
        test: test.clone(),
        red: red.clone(),
        head: task.head.clone().unwrap_or_default(),
        red_failed,
        head_passed: false,
        matched: false,
        red_tail: tail.to_string(),
        head_tail: String::new(),
        lane: None,
        red_only: true,
    };
    run.tasks[i].proofs.push(record);
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
    // Ruling T16-9 (1): the writer's kept signals go to the implementer's reviewer.
    let (kept, more) = (
        std::mem::take(&mut task.signals),
        std::mem::take(&mut task.signals_more),
    );
    if let Some(pair) = task.pair.as_mut() {
        (pair.writer_signals, pair.writer_signals_more) = (kept, more);
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
}

/// Ruling T16-9 (2): a claim whose writer-path read ran without a pathspec says so in
/// the run log, once per claim.
pub(super) fn log_unlimited(run: &mut Run, i: usize, paths: u32, now: u64) {
    if paths > 0 {
        let line = writer_paths_unlimited_line(run.tasks[i].id(), paths);
        requests::log(run, now, line);
    }
}

/// Ruling T16-3 (as ruling T14-I2 for every launch an op result asks for): each running
/// pass launches the implementer of a paired task whose red was confirmed and that has
/// had none yet. A task blocked meanwhile, or given a fresh session, goes the usual way.
pub(super) fn launch_due(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let due = handed(task).is_some()
            && task.state == TaskState::Working
            && !task.awaiting_deps
            && task.fresh_session.is_none()
            && !task.rounds.iter().any(|r| r.role == AgentRole::Worker)
            && !op_in_flight(run, task.id(), |k| matches!(k, OpKind::CreateWindow { .. }));
        if due {
            launch_implementer(run, i, now, fx);
        }
    }
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
