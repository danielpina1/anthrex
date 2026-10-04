//! Decision 38's escalation ladder — gate failures, stalls and hard budget breaches, and
//! rungs 1 to 4 — and decision 40's budgets. Rung 2's fresh session is started here
//! once the old session is gone (`DiffSoFar`, then decision 30's hand-over prompt).
//! Pure (design decision 2).

use crate::run::phases::set_state;
use proto::{BlockReason, GateKind, TaskState};

use super::dispatch::{block, history, launch_fresh};
use super::schedule::op_in_flight;
use super::{Effect, OpKind, OpResult, done, emit_op, next_op, outbox};
use crate::run::model::{AgentRound, FreshSession, Run, Task, writes};
use crate::run::route_pick::{every_route_failed, review_route, rung2_route};
use crate::run::validate::resolve_task_lenient;

pub(super) use super::ladder_budget::{breached, ceiling, check_budget, reached};
pub(crate) use super::ladder_budget::{round_spend, total_spend};

/// The note rung 3 adds to a task it raises (M8a.6's `rung3` fixture uses the same).
const RAISED_NOTE: &str = "size raised by rung 3 (decision 38)";

/// The task's current worker round, if it has one: milestone 9.5 decision 25 counts a
/// paired task's test writer, and decision 23 (ruling RR-9) a racer in its lane's view
/// or once its lane is crowned or adopted (`model::writes`).
pub(super) fn worker_round(task: &Task) -> Option<usize> {
    task.rounds.iter().rposition(|r| writes(task, r))
}

/// A worker round whose session the engine still counts on: started, not ended and
/// not being killed.
pub(super) fn live(round: &AgentRound) -> bool {
    round.window_id.is_some() && !round.ended && !round.retiring
}

/// Kills every live worker session of task `i` (decision 38, rungs 2 to 4), ending its
/// claim (ruling T12-I1) and every op its sessions awaited (ruling T12-N).
pub(super) fn kill_worker(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    done::drop_claim(
        run,
        i,
        "this session is being stopped; its task_done no longer applies",
        fx,
    );
    supersede(run, i);
    let task = &mut run.tasks[i];
    let writing: Vec<bool> = task.rounds.iter().map(|r| writes(task, r)).collect();
    for (round, _) in
        (task.rounds.iter_mut().zip(writing)).filter(|(r, w)| *w && !r.ended && !r.retiring)
    {
        if let Some(window_id) = round.window_id {
            round.retiring = true;
            fx.push(Effect::KillWindow { window_id });
        }
    }
}

/// Decision 36 step 6 (M8a.25): a working task whose current worker session the engine
/// stopped — rung 3 or a block, before an override sent the task to the merge queue and
/// its candidate was handed back — takes its messages as a resume of that session once
/// its process has ended ("decision 29 resumes an ended session to carry it"). A task
/// with a fresh session coming (rung 2, a retry) is left to it.
pub(super) fn reopen_stopped(task: &mut Task) {
    if task.state != TaskState::Working || task.fresh_session.is_some() {
        return;
    }
    let Some(r) = worker_round(task) else {
        return;
    };
    let round = &mut task.rounds[r];
    if round.ended && round.retiring && round.session_id.is_some() {
        round.retiring = false;
    }
}

/// Ruling T12-N: task `i`'s worker sessions so far are superseded. The results of the
/// ops they awaited (a resume, a commit count) are dropped when they come, and the
/// messages in flight to them (a `Deliver`'s or a resume's) leave the outbox, so their
/// `Delivered` finds nothing either.
pub(super) fn supersede(run: &mut Run, i: usize) {
    let task = &mut run.tasks[i];
    let writing: Vec<bool> = task.rounds.iter().map(|r| writes(task, r)).collect();
    for (round, _) in (task.rounds.iter_mut().zip(writing)).filter(|(_, w)| *w) {
        round.resume_op = None;
        round.count_op = None;
    }
    // Ruling T14-R2 (N2): a replaced or stopped session ends the hand-back it was
    // resolving, so its successor's claim passes every gate.
    end_hand_back(&mut run.tasks[i]);
    let id = run.tasks[i].id().to_string();
    run.outbox
        .retain(|m| m.task_id != id || m.delivered_at.is_none());
}

/// Ruling T14-R2 (N2): the hand-back context ends; a later claim is not a resolution.
pub(super) fn end_hand_back(task: &mut crate::run::model::Task) {
    task.handed_back = false;
    task.resolution = None;
}

/// Undelivered messages to task `i`'s worker are dropped when its session is replaced
/// or stopped: the hand-over prompt carries the failure record instead.
pub(super) fn drop_queued(run: &mut Run, i: usize) {
    let id = run.tasks[i].id().to_string();
    run.outbox
        .retain(|m| m.task_id != id || m.delivered_at.is_some());
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or(text)
        .trim()
}

fn gate_label(gate: GateKind) -> &'static str {
    match gate {
        GateKind::Done => "done",
        GateKind::Proof => "proof",
        GateKind::Check => "check",
        GateKind::Review => "review",
        GateKind::Merge => "merge",
    }
}

/// A gate failure (decision 38): `bounces[gate] += 1; failures += 1`; rung 3 past
/// `max_bounces` or at three failures, rung 2 at two, else rung 1 — `text` to the same
/// session as its next turn, unless `told` (the tool reply already carried it, decision
/// 55). The text joins the failure record either way. Returns the rung taken.
pub(super) fn gate_failure(
    run: &mut Run,
    i: usize,
    gate: GateKind,
    text: String,
    told: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) -> u8 {
    let rung = count_failure(run, i, gate);
    take_rung(run, i, gate, rung, text, told, now, fx);
    rung
}

/// The counting half of [`gate_failure`]: the bounce and the failure are counted, and
/// the rung they call for returned. M8b decision 20 takes it later ([`take_rung`]),
/// once a failed check's summary is decided.
pub(super) fn count_failure(run: &mut Run, i: usize, gate: GateKind) -> u8 {
    let task = &mut run.tasks[i];
    let bounces = bounces_mut(task, gate);
    *bounces = bounces.saturating_add(1);
    let bounced = *bounces;
    task.failures = task.failures.saturating_add(1);
    if bounced > run.limits.max_bounces || task.failures >= 3 {
        3
    } else if task.failures == 2 {
        2
    } else {
        1
    }
}

fn bounces_mut(task: &mut Task, gate: GateKind) -> &mut u8 {
    match gate {
        GateKind::Done => &mut task.bounces.done,
        GateKind::Proof => &mut task.bounces.proof,
        GateKind::Check => &mut task.bounces.check,
        GateKind::Review => &mut task.bounces.review,
        GateKind::Merge => &mut task.bounces.merge,
    }
}

/// The acting half of [`gate_failure`]: `rung`, as [`count_failure`] returned it,
/// with the counts as they stand.
#[allow(clippy::too_many_arguments)]
pub(super) fn take_rung(
    run: &mut Run,
    i: usize,
    gate: GateKind,
    rung: u8,
    text: String,
    told: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task = &mut run.tasks[i];
    let bounced = *bounces_mut(task, gate);
    task.failure_log.push(text.clone());
    let failures = task.failures;
    let label = gate_label(gate);
    if rung >= 3 {
        let cause = format!(
            "the {label} gate failed {bounced} times ({failures} failures in all); last: {}",
            first_line(&text)
        );
        rung3(run, i, cause, now, fx);
    } else if rung == 2 {
        let reason = format!("the {label} gate failed again: {}", first_line(&text));
        rung2(run, i, reason, now, fx);
    } else {
        let task = &mut run.tasks[i];
        task.rung = 1;
        set_state(task, TaskState::Working, now);
        // M8a.13: the time the gates took is not the worker's silence; a turn still
        // open is watched from here.
        // M8c: sent back, whether or not the text is queued (`told`, decision 55).
        if let Some(r) = worker_round(task) {
            task.rounds[r].last_event = now;
            task.rounds[r].sent_back_at.push(now);
        }
        history(run, i, now, format!("the {label} gate bounced it (rung 1)"));
        if !told {
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, text, now);
        }
    }
}

/// A stall (decision 38): `stalls += 1; failures += 1`; rung 3 at three failures, else
/// rung 2.
pub(super) fn stall(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.stalls = task.stalls.saturating_add(1);
    task.failures = task.failures.saturating_add(1);
    let (stalls, failures) = (task.stalls, task.failures);
    history(run, i, now, format!("stalled: {reason}"));
    if failures >= 3 {
        let cause = format!("stalled {stalls} times ({failures} failures in all); last: {reason}");
        rung3(run, i, cause, now, fx);
    } else {
        rung2(
            run,
            i,
            format!("the last session stalled: {reason}"),
            now,
            fx,
        );
    }
}

/// A hard budget breach (decision 38): rung 3 at the second, else rung 2.
pub(super) fn breach(run: &mut Run, i: usize, what: String, now: u64, fx: &mut Vec<Effect>) {
    let task = &mut run.tasks[i];
    task.budget_exceeded = task.budget_exceeded.saturating_add(1);
    if task.budget_exceeded >= 2 {
        rung3(
            run,
            i,
            format!("exceeded its budget twice; last: {what}"),
            now,
            fx,
        );
    } else {
        rung2(
            run,
            i,
            format!("the last session exceeded its budget: {what}"),
            now,
            fx,
        );
    }
}

/// Rung 2: the session killed; a fresh one on `roster::escalate(route)` starts in the
/// same worktree once the old one has exited ([`start_fresh_sessions`]). Milestone 9.5
/// decision 9a: a task with a model list takes its next candidate instead, and ruling
/// RL-1 skips a route that failed in this task (`route_pick::rung2_route`).
pub(super) fn rung2(run: &mut Run, i: usize, reason: String, now: u64, fx: &mut Vec<Effect>) {
    // Milestone 9.5 decision 20: past rung 1, a lane leaves its race.
    if run.tasks[i].lane_view.is_some() {
        return super::race::lane_out(run, i, 2, BlockReason::Human, reason, now, fx);
    }
    done::drop_claim(
        run,
        i,
        "this session is being replaced by a fresh one; its task_done no longer applies",
        fx,
    );
    kill_worker(run, i, fx);
    drop_queued(run, i);
    // Milestone 9.5 decision 25: a fresh test writer, while the test is being written.
    if !super::pair::escalate_writer(run, i, now) {
        let (route, step) = rung2_route(run, i);
        if let Some(text) = every_route_failed(run, i, &route) {
            super::requests::log(run, now, text);
        }
        let task = &mut run.tasks[i];
        task.list_escalation = step;
        // M8b decision 33a: the next worker launch records this escalation, its pool
        // stepping from the route the selector stepped from (a second escalation
        // before the launch overwrites the first: the intermediate route never ran).
        task.escalated_from = Some(std::mem::replace(&mut task.route, route));
    }
    let task = &mut run.tasks[i];
    task.rung = 2;
    set_state(task, TaskState::Working, now);
    task.fresh_session = Some(FreshSession {
        reason: reason.clone(),
        append: None,
    });
    history(run, i, now, format!("rung 2: a fresh session ({reason})"));
}

/// Rung 3: `blocked(mis_sized)`, the size raised one step, the worker killed and the
/// worktree kept.
pub(super) fn rung3(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    // Milestone 9.5 decision 20: the size is the task's; a lane only leaves its race.
    if run.tasks[i].lane_view.is_some() {
        return super::race::lane_out(run, i, 3, BlockReason::MisSized, text, now, fx);
    }
    kill_worker(run, i, fx);
    drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.size = task.size.raised();
    task.raised_size = Some(task.size);
    task.rung = 3;
    reresolve(run, i);
    let task = &mut run.tasks[i];
    task.fresh_session = None;
    if !task.notes.iter().any(|n| n == RAISED_NOTE) {
        task.notes.push(RAISED_NOTE.to_string());
    }
    block(run, i, BlockReason::MisSized, text, now);
}

/// Ruling T13-minors (m3): what depends on the size is resolved again for the raised
/// size — the review level, its reviewer and a budget the plan did not set — as an
/// edit's re-resolution does (`edits.rs`), so a retried task is reviewed and budgeted
/// as what it now is. The route is the task's own (rung 2 may have escalated it); the
/// route the raised size resolves to is returned (M8b decision 19 takes its effort).
pub(crate) fn reresolve(run: &mut Run, i: usize) -> proto::Route {
    let mut spec = run.tasks[i].spec.clone();
    spec.size = spec.size.max(run.tasks[i].size);
    let (resolved, _) = resolve_task_lenient(
        spec,
        &run.profile,
        &run.limits,
        &run.roster,
        run.limits.default_runtime,
    );
    let task = &mut run.tasks[i];
    task.review_level = resolved.review_level;
    let (lists, installed) = (&run.limits.route_lists, &run.orch.installed);
    task.review_route = (resolved.review_level)
        .map(|level| review_route(lists, &run.roster, &task.route, level, installed));
    task.budget = resolved.budget;
    resolved.route
}

/// Rung 4: `blocked(human)`, the worker killed.
pub(super) fn rung4(run: &mut Run, i: usize, text: String, now: u64, fx: &mut Vec<Effect>) {
    kill_worker(run, i, fx);
    drop_queued(run, i);
    let task = &mut run.tasks[i];
    task.rung = 4;
    task.fresh_session = None;
    block(run, i, BlockReason::Human, text, now);
}

/// Rung 2, or a resume that failed (decision 28): a working task with a fresh session
/// decided on, no live worker session left and none being prepared gets `DiffSoFar`
/// for its hand-over prompt. A held task never does (M8a.6 ruling N5): it is not
/// `working` until its hand-back.
pub(super) fn start_fresh_sessions(run: &mut Run, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if !fresh_due(task)
            || op_in_flight(run, task.id(), |k| {
                matches!(k, OpKind::DiffSoFar { .. } | OpKind::CreateWindow { .. })
            })
        {
            continue;
        }
        let (id, worktree) = (task.id().to_string(), task.worktree.clone());
        let start = task
            .start_commit
            .clone()
            .unwrap_or_else(|| run.head_for(task).to_string());
        let kind = OpKind::DiffSoFar {
            worktree,
            start,
            run_head: run.head_for(task).to_string(),
        };
        let op = next_op(run);
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// Carry T12-P2 (M8a.13): a working task whose worker session ended with no session id
/// (a Codex process that exited before its first `thread.started`, while the task was
/// not working: held, blocked or in a gate) has nothing a delivery could resume. It
/// gets a fresh session at the same rung and route, with no failure counted, whose
/// hand-over prompt carries the messages waiting for it (ruling T12-I3's `append`).
/// A held task never does (M8a.6 ruling N5): it is not `working`.
pub(super) fn recover_sessionless(run: &mut Run, now: u64) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let lost = worker_round(task).is_some_and(|r| {
            let round = &task.rounds[r];
            round.ended && !round.retiring && round.session_id.is_none()
        });
        if task.state != TaskState::Working
            || task.awaiting_deps
            || task.fresh_session.is_some()
            || !lost
        {
            continue;
        }
        let reason = "its session ended before it had an id to resume".to_string();
        history(run, i, now, format!("a fresh session: {reason}"));
        run.tasks[i].fresh_session = Some(FreshSession {
            reason,
            append: None,
        });
    }
}

fn fresh_due(task: &Task) -> bool {
    task.state == TaskState::Working
        && !task.awaiting_deps
        && task.fresh_session.is_some()
        && task
            .rounds
            .iter()
            .filter(|r| writes(task, r))
            .all(|r| r.ended)
}

/// `DiffSoFar`'s result: the fresh session, with decision 30's hand-over prompt, when
/// it is still due. A diff that could not be computed is named in the prompt.
pub(super) fn fresh_diff(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    // Ruling T14-I2: no session starts while the run is not running; the running pass
    // asks for the diff again (`start_fresh_sessions`).
    if !fresh_due(&run.tasks[i]) || run.state != proto::RunState::Running {
        return;
    }
    let (stat, patch) = match result {
        OpResult::Diff { stat, patch } => (stat, patch),
        OpResult::Failed { message } => (
            format!("(the diff could not be read: {message})"),
            String::new(),
        ),
        _ => return,
    };
    let Some(fresh) = run.tasks[i].fresh_session.clone() else {
        return;
    };
    let rounds = run.tasks[i].rounds.len();
    launch_fresh(run, i, &fresh, &stat, &patch, now, fx);
    // Kept when the launch was refused (the window limit blocks the task), so the
    // messages it carries are not lost.
    if run.tasks[i].rounds.len() > rounds {
        run.tasks[i].fresh_session = None;
    }
}
