//! Milestone 9 decision 28's approval holds: work the orchestrator adds after the plan
//! gate waits for the user's approval, without blocking a tool call or the running
//! work. Not `holds.rs`, which is M8a ruling N5's dependency hold (`awaiting_deps`):
//! nothing here reads or changes that. Pure (design decision 2).
//!
//! A hold is `promotion` (decision 29: everything a promoted run's orchestrator adds
//! until the user approves the promotion) or `epic:<e>` (a new epic of a running run). A task whose hold is
//! not `Approved` is not runnable and is not pre-warmed (`dispatch.rs`).

use proto::{HoldKind, HoldState, PlanEdit, RunState, TaskState};

use super::complete::cancel_now;
use super::requests::log;
use super::{Effect, EngineState, ReplyId};
use crate::run::model::{Run, Task};
use crate::run::orch::GateHoldRecord;
use crate::run::orch::json::label;

/// Decision 29's hold id.
pub(super) const PROMOTION: &str = "promotion";

/// Decision 28: the task has no approval hold, or its hold is approved.
pub(super) fn released(run: &Run, task: &Task) -> bool {
    task.orch.gate_hold.as_ref().is_none_or(|id| {
        run.orch
            .gate_holds
            .iter()
            .any(|h| &h.id == id && h.state == HoldState::Approved)
    })
}

/// Decision 29: the run was promoted, and its orchestrator exists. A fast-path run
/// started under milestone 8b has no `approved_at`, so the promotion, not the approval,
/// is what puts its additions past the gate (M9.7 review fixes, ruling 1).
pub(super) fn promoted(run: &Run) -> bool {
    run.promote_requested_at.is_some() && run.orch.orchestrator.is_some()
}

/// Decision 29's window: the run was promoted and no `promotion` round has been
/// approved. A rejected round does not close it: the orchestrator's next addition or
/// submit opens a new round, so its work never bypasses the user's "no" (M9.7 second
/// review, ruling 7).
pub(super) fn promotion_open(run: &Run) -> bool {
    promoted(run)
        && !run
            .orch
            .gate_holds
            .iter()
            .any(|h| is_promotion(h) && h.state == HoldState::Approved)
}

fn is_promotion(hold: &GateHoldRecord) -> bool {
    matches!(hold.kind, HoldKind::Promotion)
}

/// The undecided `promotion` round, if any: the last one, when it is `Drafting` or
/// `Awaiting`.
fn open_round(run: &Run) -> Option<&GateHoldRecord> {
    run.orch
        .gate_holds
        .iter()
        .rev()
        .find(|h| is_promotion(h))
        .filter(|h| matches!(h.state, HoldState::Drafting | HoldState::Awaiting))
}

/// The open `promotion` round, or a new `Drafting` one: `promotion` first, then
/// `promotion-2`, `promotion-3` after each rejection, so every round keeps its own id
/// in the history, the digest and the run view.
fn promotion_round(run: &mut Run, now: u64) -> String {
    if let Some(open) = open_round(run) {
        return open.id.clone();
    }
    let rounds = run
        .orch
        .gate_holds
        .iter()
        .filter(|h| is_promotion(h))
        .count();
    let id = fresh(run, PROMOTION, '-', rounds);
    create(run, &id, HoldKind::Promotion, now)
}

/// The orchestrator's `submit` on a promoted running run (decision 27): the open
/// `promotion` round awaits the user (or is approved by `--yes`), created with no task
/// when the orchestrator added nothing since the last round, so the user's approval is
/// still what ends the promotion window (M9.7 second review, rulings 1 and 7). Returns
/// whether a round was submitted now.
pub(super) fn submit_promotion(run: &mut Run, now: u64) -> bool {
    if !promotion_open(run) {
        return false;
    }
    if open_round(run).is_some_and(|h| h.state == HoldState::Awaiting) {
        return false;
    }
    let id = promotion_round(run, now);
    submitted(run, &id, now);
    true
}

/// The run's plan was approved once (by the user, `--yes` or the fast path), or the run
/// was promoted: work added from now on is past the gate.
fn past_gate(run: &Run) -> bool {
    run.approved_at.is_some() || promoted(run)
}

/// Gives each task of `added` (the new tasks of an orchestrator's or sub-planner's
/// batch, `edits`; a user's own `run edit` never comes here) the hold it waits under,
/// if any, and returns the first such hold. A task split from a task whose hold is not
/// `Approved` inherits that hold, whatever epic it names, and the split parent, which
/// the split cancelled, leaves it (M9.7 review fixes, ruling 2). Otherwise, until the
/// user decides a promoted run's `promotion` hold, every task waits under it (decision
/// 29, whatever the gate's state; joining it while it awaits the user, M9.7 second
/// review, ruling 1); after that, a task of an epic whose hold is still undecided waits
/// under that hold.
pub(super) fn assign(
    run: &mut Run,
    edits: &[PlanEdit],
    added: &[String],
    now: u64,
) -> Option<String> {
    run.orch.orchestrator.as_ref()?;
    let mut first = None;
    for id in added {
        let parent = split_parent(edits, id);
        let hold = match parent.and_then(|p| unreleased_hold(run, p)) {
            Some(inherited) => Some(inherited),
            // An epic's rule comes first: its work waits for its own round, never
            // the promotion's (M9.7 second review, ruling 9).
            None if names_an_epic(run, id) => epic_hold(run, id, now),
            None if promotion_open(run) => Some(promotion_round(run, now)),
            None => None,
        };
        let Some(hold) = hold else {
            continue;
        };
        hold_task(run, &hold, id);
        first.get_or_insert(hold);
    }
    for parent in split_parents(edits) {
        release_split_parent(run, parent);
    }
    first
}

/// The task `id` was split from by this batch, if any.
fn split_parent<'a>(edits: &'a [PlanEdit], id: &str) -> Option<&'a str> {
    edits.iter().find_map(|e| match e {
        PlanEdit::SplitTask { task_id, into } if into.iter().any(|t| t.id == id) => {
            Some(task_id.as_str())
        }
        _ => None,
    })
}

fn split_parents(edits: &[PlanEdit]) -> impl Iterator<Item = &str> {
    edits.iter().filter_map(|e| match e {
        PlanEdit::SplitTask { task_id, .. } => Some(task_id.as_str()),
        _ => None,
    })
}

/// Task `id`'s hold, when it is not yet approved.
fn unreleased_hold(run: &Run, id: &str) -> Option<String> {
    let hold = run.task(id)?.orch.gate_hold.as_ref()?;
    run.orch
        .gate_holds
        .iter()
        .find(|h| &h.id == hold && h.state != HoldState::Approved)
        .map(|h| h.id.clone())
}

fn hold_task(run: &mut Run, hold: &str, id: &str) {
    if let Some(record) = run.orch.gate_holds.iter_mut().find(|h| h.id == hold) {
        record.tasks.push(id.to_string());
    }
    if let Some(task) = run.tasks.iter_mut().find(|t| t.id() == id) {
        task.orch.gate_hold = Some(hold.to_string());
    }
}

/// A split cancels its parent: the parent is no longer among the tasks its hold would
/// start or cancel, so a verdict counts only its children.
fn release_split_parent(run: &mut Run, parent: &str) {
    let Some(task) = run.tasks.iter_mut().find(|t| t.id() == parent) else {
        return;
    };
    if task.state != TaskState::Cancelled {
        return;
    }
    let Some(hold) = task.orch.gate_hold.take() else {
        return;
    };
    if let Some(record) = run.orch.gate_holds.iter_mut().find(|h| h.id == hold) {
        record.tasks.retain(|t| t != parent);
    }
}

fn is_round_of(hold: &GateHoldRecord, epic: &str) -> bool {
    matches!(&hold.kind, HoldKind::Epic { epic: e } if e == epic)
}

/// Whether `epic` has a round and none of its rounds is approved: its work is held
/// until the user approves one (decision 28; after a rejection, M9.7 second review,
/// ruling 8).
fn epic_open(run: &Run, epic: &str) -> bool {
    // The record names a round once one was opened; it keeps naming it when an empty
    // round is dropped (`drop_empty_rounds`), so the epic stays held.
    let opened = run
        .orch
        .epics
        .iter()
        .any(|e| e.epic == epic && e.gate_hold.is_some());
    let rounds: Vec<_> = run
        .orch
        .gate_holds
        .iter()
        .filter(|h| is_round_of(h, epic))
        .collect();
    opened && rounds.iter().all(|h| h.state != HoldState::Approved)
}

/// Task `id` names an epic the run has.
fn names_an_epic(run: &Run, id: &str) -> bool {
    run.task(id)
        .and_then(|t| t.spec.epic.as_deref())
        .is_some_and(|epic| run.orch.epics.iter().any(|e| e.epic == epic))
}

/// Decision 38: a round still `Drafting` with no task, whose epic's sub-planner has
/// ended without submitting, is dropped, so it cannot keep the run from completing.
/// The epic's record still names it, so the epic stays held and its next re-plan or
/// addition opens a round again (M9.7 second review, ruling 9).
pub(super) fn drop_empty_rounds(run: &mut Run, now: u64) {
    let ended = |run: &Run, epic: &str| {
        run.orch
            .epics
            .iter()
            .any(|e| e.epic == epic && !e.phase.is_live())
    };
    let dropped: Vec<String> = run
        .orch
        .gate_holds
        .iter()
        .filter(|h| h.state == HoldState::Drafting && h.tasks.is_empty())
        .filter(|h| matches!(&h.kind, HoldKind::Epic { epic } if ended(run, epic)))
        .map(|h| h.id.clone())
        .collect();
    for id in dropped {
        run.orch.gate_holds.retain(|h| h.id != id);
        log(
            run,
            now,
            format!("hold {id} dropped: its sub-planner ended with no task"),
        );
    }
}

/// `epic`'s undecided round, or a new `Drafting` one: `epic:<e>` first, then
/// `epic:<e>.2`, `epic:<e>.3` after each rejection. The separator is `.`, which an
/// epic id (`^[a-z0-9][a-z0-9-]{0,10}$`) cannot hold, so a round never takes the id of
/// another epic's hold. The epic's record names its current round.
fn epic_round(run: &mut Run, epic: &str, now: u64) -> String {
    let open = run
        .orch
        .gate_holds
        .iter()
        .rev()
        .find(|h| is_round_of(h, epic))
        .filter(|h| matches!(h.state, HoldState::Drafting | HoldState::Awaiting))
        .map(|h| h.id.clone());
    let id = match open {
        Some(id) => id,
        None => {
            let rounds = run
                .orch
                .gate_holds
                .iter()
                .filter(|h| is_round_of(h, epic))
                .count();
            let id = fresh(run, &format!("epic:{epic}"), '.', rounds);
            create(run, &id, HoldKind::Epic { epic: epic.into() }, now)
        }
    };
    if let Some(record) = run.orch.epics.iter_mut().find(|e| e.epic == epic) {
        record.gate_hold = Some(id.clone());
    }
    id
}

/// The round task `id` of an epic waits under: its epic's open round, opened anew
/// after a rejection; none when the epic has no round or one was approved.
fn epic_hold(run: &mut Run, id: &str, now: u64) -> Option<String> {
    let epic = run.task(id)?.spec.epic.clone()?;
    epic_open(run, &epic).then(|| epic_round(run, &epic, now))
}

/// The first free id of round `rounds + 1` of `base`: `base` itself for the first,
/// then `base<sep><n>`.
fn fresh(run: &Run, base: &str, sep: char, rounds: usize) -> String {
    let taken = |id: &str| run.orch.gate_holds.iter().any(|h| h.id == id);
    let name = |n: usize| match n {
        1 => base.to_string(),
        n => format!("{base}{sep}{n}"),
    };
    let mut n = rounds + 1;
    while taken(&name(n)) {
        n += 1;
    }
    name(n)
}

/// A new `Drafting` hold `id`. Callers pass a free id ([`fresh`]), so no rejected or
/// decided hold is ever handed back as the one to join.
fn create(run: &mut Run, id: &str, kind: HoldKind, now: u64) -> String {
    debug_assert!(!run.orch.gate_holds.iter().any(|h| h.id == id), "{id}");
    run.orch.gate_holds.push(GateHoldRecord {
        id: id.to_string(),
        kind,
        state: HoldState::Drafting,
        tasks: Vec::new(),
        created_at: now,
        decided_at: None,
        decided_by: None,
    });
    log(run, now, format!("hold {id} created"));
    id.to_string()
}

/// Decision 28: a new epic of a running run, whose plan was submitted, gets hold
/// `epic:<e>`, `Drafting` until its sub-planner's epic is accepted. So does every new
/// epic of a promoted run, the promotion window included: approving the promotion
/// never releases an epic's work (M9.7 second review, ruling 9).
pub(super) fn create_epic_hold(run: &mut Run, epic: &str, now: u64) -> Option<String> {
    run.orch.orchestrator.as_ref()?;
    if !past_gate(run) {
        return None;
    }
    Some(epic_round(run, epic, now))
}

/// A re-planned epic whose every round was rejected opens a new round (M9.7 second
/// review, ruling 8); any other re-plan is held as before.
pub(super) fn replan_epic_hold(run: &mut Run, epic: &str, now: u64) -> Option<String> {
    epic_open(run, epic).then(|| epic_round(run, epic, now))
}

/// A drafted hold is submitted (the orchestrator's `submit`, a sub-planner's accepted
/// epic): it awaits the user, or is approved at once for a run started with `--yes`.
pub(super) fn submitted(run: &mut Run, id: &str, now: u64) {
    let yes = run.orch.yes;
    let Some(hold) = run
        .orch
        .gate_holds
        .iter_mut()
        .find(|h| h.id == id && h.state == HoldState::Drafting)
    else {
        return;
    };
    if yes {
        hold.state = HoldState::Approved;
        hold.decided_at = Some(now);
        hold.decided_by = Some("--yes".into());
        log(run, now, format!("hold {id} approved by --yes"));
    } else {
        hold.state = HoldState::Awaiting;
        log(run, now, format!("hold {id} awaits approval"));
    }
}

/// `run approve --hold` (`approve`) or `run reject --hold`: an awaiting hold is
/// approved, and its tasks become runnable, or rejected, and its tasks are cancelled
/// (none has started; dependents become `blocked(dep_cancelled)`, decision 13).
pub(super) fn verdict(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, id): (&str, &str),
    approve: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = decide(state, (run_id, id), approve, now, fx);
    fx.push(Effect::Reply { reply, result });
}

fn decide(
    state: &mut EngineState,
    (run_id, id): (&str, &str),
    approve: bool,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    let Some(run) = state.runs.get_mut(run_id) else {
        return Err(format!("unknown run {run_id}"));
    };
    if run.state.is_terminal() || run.state == RunState::Complete {
        return Err(format!("run {run_id} is {}", run.state.label()));
    }
    let Some(hold) = run.orch.gate_holds.iter_mut().find(|h| h.id == id) else {
        return Err(format!("run {run_id} has no hold {id}"));
    };
    if hold.state != HoldState::Awaiting {
        return Err(format!("hold {id} is {}", label(&hold.state)));
    }
    hold.decided_at = Some(now);
    hold.decided_by = Some("user".into());
    let tasks = hold.tasks.clone();
    if approve {
        hold.state = HoldState::Approved;
        log(run, now, format!("hold {id} approved by the user"));
        // Only live work counts: a task the user's own `run edit` cancelled or split
        // (its children are the user's and carry no hold) is released already (M9.7
        // second review, ruling 3).
        let n = tasks
            .iter()
            .filter(|t| run.task(t).is_some_and(|t| !t.state.is_finished()))
            .count();
        return Ok(format!(
            "hold {id} of run {run_id} approved: {n} task{} may start",
            plural(n)
        ));
    }
    hold.state = HoldState::Rejected;
    let mut cancelled = 0;
    for task in &tasks {
        let Some(i) = run.tasks.iter().position(|t| t.id() == task) else {
            continue;
        };
        if run.tasks[i].state.is_finished() {
            continue;
        }
        cancel_now(run, i, &format!("its hold {id} was rejected"), now, fx);
        cancelled += 1;
    }
    log(run, now, format!("hold {id} rejected by the user"));
    Ok(format!(
        "hold {id} of run {run_id} rejected: {cancelled} task{} cancelled",
        plural(cancelled)
    ))
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
