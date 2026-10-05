//! Milestone 9.0.6 decision 8: every run request's precondition, one function per
//! request, each returning the handler's refusal text byte for byte (`None`: the request
//! is not refused by the run's state). The handlers call these instead of inline checks,
//! so `check` and the handlers cannot drift apart. Pure (design decision 2).
//!
//! `restore::resume` asks [`resume`] first, and `merge::resume`'s refusals reply with its
//! text too; the actions matrix (`tests/actions_matrix.rs`) proves every rule agrees
//! with its handler in every fixture state.

use proto::{
    BlockReason, FinishAction, HoldState, MessageKind, MessageTarget, PlanEdit, RunPath, RunState,
    Size, TaskState,
};

use crate::run::edits_orch::{one_edit_rule, plan_message, refresh_refusal};
use crate::run::edits_state::state_label;
use crate::run::engine::dispatch::finishing_as;
use crate::run::engine::gates::{OVERRIDE_APPLIES, OVERRIDE_KINDS};
use crate::run::engine::{delivery, full, goal_rounds, orch_window, schedule, worker_messages};
use crate::run::model::Run;
use crate::run::orch::contract_rounds as rounds;
use crate::run::plan::PlanError;

/// The refusal of a handler whose rule, asked again, no longer refuses: a rule and its
/// handler drifted apart. The engine answers instead of panicking.
pub(crate) const DRIFT: &str = "internal: request precondition changed";

/// A handler's refusal from its rule's answer (`rule`), [`DRIFT`] if it has none.
pub(crate) fn refused(rule: Option<String>) -> String {
    rule.unwrap_or_else(|| DRIFT.to_string())
}

/// A run with an `Accept` or `Discard` op in flight refuses every request that would
/// change it.
fn being_finished(run: &Run) -> Option<String> {
    finishing_as(run).map(|how| format!("run {} is being {how}", run.id))
}

fn is(run: &Run) -> String {
    format!("run {} is {}", run.id, run.state.label())
}

/// The batch's own "no such task" error (`Batch::find`), as its reply prints it.
fn no_such_task(task_id: &str) -> String {
    PlanError::new(Some(task_id), "task_id", "13", "no such task").to_string()
}

/// `run approve` (`requests::approve`).
pub(crate) fn approve(run: &Run) -> Option<String> {
    let run_id = &run.id;
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    // Milestone 9.6 ruling T1-O1: the brainstorm and spec gates take `--gate`.
    if let Some(text) = crate::run::engine::design_gate::plain_approve_refusal(run) {
        return Some(text);
    }
    // Ruling T7-3: the plan gate's approve is its own action's (`design_gate::act`).
    if let Some(text) = crate::run::engine::design_gate::plan_approve_refusal(run) {
        return Some(text);
    }
    // Milestone 9.6 decision 18: the plan at its gate passes its checks again.
    if let Some(text) = crate::run::engine::design::plan::approve_refusal(run) {
        return Some(text);
    }
    if run.state == RunState::Planning {
        return Some(format!(
            "run {run_id} is still being planned; approve it when the orchestrator has submitted the plan"
        ));
    }
    (run.state != RunState::AwaitingApproval).then(|| is(run))
}

/// `run reject` (`requests::reject`): a run at its gate, or still being planned, also
/// when a daemon restart paused it there (M9.7 review fixes, ruling 4).
pub(crate) fn reject(run: &Run) -> Option<String> {
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    let planning = run.state == RunState::Planning
        || (run.state == RunState::Paused && run.paused_from == Some(RunState::Planning))
        || crate::run::engine::design_gate::in_doc_phase(run);
    (!planning && run.state != RunState::AwaitingApproval).then(|| {
        let label = run.state.label();
        format!(
            "run {} is {label}; reject applies only while its plan awaits approval",
            run.id
        )
    })
}

/// M9.9 review fixes, C1: a promoted running run whose orchestrator has not submitted
/// its plan.
pub(crate) fn promoted_unsubmitted(run: &Run) -> bool {
    run.state == RunState::Running
        && run
            .orch
            .orchestrator
            .as_ref()
            .is_some_and(|o| !o.plan_submitted)
}

/// `run edit --submit`'s own check (`requests::submit_edit`), after [`edit_run`].
pub(crate) fn submit(run: &Run) -> Option<String> {
    (run.state != RunState::Planning && !promoted_unsubmitted(run)).then(|| {
        let label = run.state.label();
        format!(
            "run {} is {label}; only a run being planned can be submitted",
            run.id
        )
    })
}

/// `run edit`'s run-wide checks (`requests::edit`), before the batch is applied.
pub(crate) fn edit_run(run: &Run, edits: &[PlanEdit], submit: bool) -> Option<String> {
    let run_id = &run.id;
    // Milestone 9 decision 42: a `message` or `refresh` is alone in its request.
    if let Err(error) = one_edit_rule(edits, submit, false) {
        return Some(error.to_string());
    }
    if run.state.is_terminal() || run.state == RunState::Complete {
        return Some(is(run));
    }
    // Whole-branch review m1: a fast-path run runs one task.
    let adds = |e: &PlanEdit| matches!(e, PlanEdit::AddTask { .. } | PlanEdit::SplitTask { .. });
    if run.path == Some(RunPath::Fast) && edits.iter().any(adds) {
        return Some(format!(
            "run {run_id} is on the fast path: it runs one task; start a planned run instead"
        ));
    }
    // M9.9 second review, C-1: a cancelled run only loses work.
    (run.cancelled && (submit || edits.iter().any(adds)))
        .then(|| format!("run {run_id} was cancelled"))
}

/// Decision 45: a `pause` edit applies only to a running run (`batch.rs`).
pub(crate) fn pause(run: &Run) -> Option<String> {
    pause_in(&run.id, run.state)
}

/// [`pause`] on a run `id` in `state`: a batch checks each edit against the state the
/// edits before it leave.
pub(crate) fn pause_in(id: &str, state: RunState) -> Option<String> {
    (state != RunState::Running).then(|| {
        format!(
            "run {id} is {}; only a running run can be paused",
            state.label()
        )
    })
}

/// Decision 45: a `resume` edit applies only to a paused run (`batch.rs`).
pub(crate) fn unpause(run: &Run) -> Option<String> {
    unpause_in(&run.id, run.state)
}

/// [`unpause`] on a run `id` in `state`.
pub(crate) fn unpause_in(id: &str, state: RunState) -> Option<String> {
    (state != RunState::Paused).then(|| {
        format!(
            "run {id} is {}; only a paused run can be resumed",
            state.label()
        )
    })
}

/// `run resume` (`restore::resume`, then `merge::resume`): a running run with a held
/// tier 3 (ruling C-18), a paused run, a run at its gate or planning whose orchestrator
/// restarts (milestone 9 decision 11), or a halted run that rebaselines or whose halt is
/// retryable (review m1).
pub(crate) fn resume(run: &Run, rebaseline: bool) -> Option<String> {
    let retries = match run.state {
        RunState::Running => full::retryable(run) || delivery::held(run),
        RunState::Paused => true,
        RunState::AwaitingApproval
        | RunState::Planning
        | RunState::Brainstorming
        | RunState::Specifying => orch_window::relaunchable(run),
        _ => false,
    };
    if retries {
        return None;
    }
    if run.state != RunState::Halted {
        return Some(is(run));
    }
    // Milestone 9.6 ruling T7-9: a run cancelled before its plan was approved is
    // discarded, never resumed into its phase.
    if run.cancelled && crate::run::engine::design::halted_phase(run).is_some() {
        let id = &run.id;
        return Some(format!(
            "run {id} was cancelled before its plan was approved; discard it with anthrex run discard {id}"
        ));
    }
    // Milestone 9.6 ruling T7-1: a phase budget's halt resumes into its phase only.
    if let Some(phase) = crate::run::engine::design::halted_phase(run).filter(|_| rebaseline) {
        return Some(format!(
            "run {} halted in its {phase} phase; resume it without --rebaseline",
            run.id
        ));
    }
    // Milestone 9.6 (task 7's carry): a halt before the plan's approval resumes into
    // its phase.
    let phase = crate::run::engine::design::halted_phase(run).is_some();
    (!rebaseline && !run.halt_retryable && !phase).then(|| {
        let reason = run.halted_reason.clone().unwrap_or_default();
        format!(
            "run {} is halted: {reason}; check the refs, then resume with --rebaseline",
            run.id
        )
    })
}

/// `run cancel` (`complete::cancel`).
pub(crate) fn cancel(run: &Run) -> Option<String> {
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    match run.state {
        RunState::Running | RunState::Halted | RunState::Paused => None,
        _ => Some(is(run)),
    }
}

/// `run promote` (`promote::promote`): a run already marked is answered, not refused.
pub(crate) fn promote(run: &Run) -> Option<String> {
    if run.promote_requested_at.is_some() && run.orch.orchestrator.is_some() {
        return None;
    }
    if run.path != Some(RunPath::Fast) {
        return Some(format!("run {} is not a fast-path run", run.id));
    }
    // M8b review m3: a `complete` run only waits for accept; nothing is left to promote.
    (run.state.is_terminal() || run.state == RunState::Complete).then(|| is(run))
}

/// `accept` or `discard`.
pub(crate) fn finish_verb(action: FinishAction) -> &'static str {
    match action {
        FinishAction::Accept => "accept",
        FinishAction::Discard => "discard",
    }
}

/// `run accept` and `run discard` (`complete::finish`).
pub(crate) fn finish(run: &Run, action: FinishAction) -> Option<String> {
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    let verb = finish_verb(action);
    // Review m2: a cancelled run that is halted has nothing left to verify for a
    // discard, so it needs no rebaseline first.
    let discardable = action == FinishAction::Discard
        && run.state == RunState::Halted
        && run.cancelled
        && run.tasks.iter().all(|t| t.state.is_finished())
        && run.pending_ops.is_empty()
        // The final fix wave's m7: never beside a race lane's salvage.
        && !crate::run::engine::race_salvage::pending(run);
    let state = || {
        let label = run.state.label();
        format!(
            "run {} is {label}; {verb} applies only to a complete run",
            run.id
        )
    };
    // Milestone 9.2 decisions 38-39 and ruling R-1: delivered by pull request; an
    // ended run's state comes first, and a discardable one is not refused (task
    // M9.2.7's review m4).
    if run.state.is_terminal() {
        return Some(state());
    }
    if let Some(text) = delivery::finish_refusal(run, action).filter(|_| !discardable) {
        return Some(text);
    }
    (run.state != RunState::Complete && !discardable).then(state)
}

/// Milestone 9.3 decision 9: `run iterate` and `edit_plan`'s `iterate`
/// (`goal_rounds::iterate`), in order: a run being finished, one with no orchestrator,
/// `ROUNDS_MAX` reached, `STAGES_MAX` reached (the final fix wave, A-M5), halted, ended, one whose chain started a next goal since (D17),
/// then a settled run passes; a cancelled `pr`
/// run (D7) and every other state are refused.
pub(crate) fn iterate(run: &Run) -> Option<String> {
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    let h4 = run.short();
    if run.orch.orchestrator.is_none() {
        return Some(rounds::no_orchestrator(h4));
    }
    if run.round() >= proto::ROUNDS_MAX {
        return Some(rounds::rounds_max(h4));
    }
    // The final fix wave (A-M5): a round needs a stage after the last.
    if crate::run::snapshot_stages::stage_count(run) >= proto::STAGES_MAX {
        return Some(rounds::stages_max(h4));
    }
    match run.state {
        RunState::Halted => Some(rounds::halted(h4)),
        RunState::Accepted | RunState::Discarded | RunState::Failed => {
            Some(rounds::ended(h4, run.state.label()))
        }
        // D17: a next goal started from its chain since.
        _ if run.continued_by.is_some() => Some(rounds::superseded(h4)),
        _ if goal_rounds::settled(run) => None,
        RunState::Complete if delivery::pr(run) && run.cancelled => Some(rounds::cancelled(h4)),
        _ => Some(rounds::not_settled(h4, run.state.label())),
    }
}

/// `run approve --hold` and `run reject --hold` (`gate_holds::decide`).
pub(crate) fn hold(run: &Run, id: &str) -> Option<String> {
    if run.state.is_terminal() || run.state == RunState::Complete {
        return Some(is(run));
    }
    let Some(hold) = run.orch.gate_holds.iter().find(|h| h.id == id) else {
        return Some(format!("run {} has no hold {id}", run.id));
    };
    (hold.state != HoldState::Awaiting).then(|| {
        format!(
            "hold {id} is {}",
            crate::run::orch::json::label(&hold.state)
        )
    })
}

/// An `info` message to task `task_id` (`edits_orch::apply_message`), past its text
/// check: the run's own per-turn limit. It models `info` and `change` only: a
/// `stop_and_wait` refuses more (a task already paused, or not working), so `check`
/// must not use it for one.
pub(crate) fn message(run: &Run, task_id: &str) -> Option<String> {
    let to = MessageTarget::Tasks(vec![task_id.to_string()]);
    message_to(run, &to)
}

/// An `info` message to every unfinished task of stage `stage` (milestone 9.1 decision
/// 56). Like [`message`], it models `info` and `change` only, never `stop_and_wait`.
pub(crate) fn message_stage(run: &Run, stage: u16) -> Option<String> {
    message_to(run, &MessageTarget::Stage(u32::from(stage)))
}

fn message_to(run: &Run, to: &MessageTarget) -> Option<String> {
    let limit = run.limits.orch.message_max_per_turn;
    plan_message(run, to, MessageKind::Info, limit)
        .err()
        .map(|e| e.to_string())
}

/// An `answer` edit (`Batch::answer`): a `blocked(question)` or `working` task.
pub(crate) fn answer(run: &Run, task_id: &str) -> Option<String> {
    let Some(task) = run.task(task_id) else {
        return Some(no_such_task(task_id));
    };
    let question = task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::Question);
    (!question && task.state != TaskState::Working).then(|| {
        format!(
            "task {task_id} is {}; only blocked(question) or working tasks can be answered",
            state_label(task)
        )
    })
}

/// A `refresh` edit from the user (`edits_orch::apply_refresh`).
pub(crate) fn refresh(run: &Run, task_id: &str) -> Option<String> {
    refresh_refusal(run, task_id).map(|e| e.to_string())
}

/// `run retry` (`requests::retry`, decision 42).
pub(crate) fn retry(run: &Run, task_id: &str) -> Option<String> {
    if let Some(text) = being_finished(run) {
        return Some(text);
    }
    if !matches!(run.state, RunState::Running | RunState::Paused) {
        return Some(is(run));
    }
    let Some(task) = run.task(task_id) else {
        return Some(format!("unknown task {task_id}"));
    };
    match &task.block {
        // Milestone 9.5 decision 23: nothing to retry until one racer is left (a
        // winner whose crown failed is blocked, and retried).
        _ if task.racing() && (task.state != TaskState::Blocked || has_live_lane(task)) => Some(
            format!("task {task_id} is racing: it has nothing to retry until one racer is left"),
        ),
        // Milestone 9 decision 42c.
        _ if worker_messages::paused_refusal(task).is_some() => {
            worker_messages::paused_refusal(task)
        }
        _ if task.state != TaskState::Blocked => Some(format!(
            "task {task_id} is {}; retry applies only to a blocked task",
            task.state.label()
        )),
        Some(b) if b.reason == BlockReason::DepCancelled => Some(format!(
            "task {task_id} is blocked(dep_cancelled); retry cannot bring back a cancelled dependency"
        )),
        _ if task.size == Size::L => Some(format!("task {task_id} is L; split it first")),
        _ if task.awaiting_deps && !schedule::deps_done(run, task) => Some(format!(
            "task {task_id} waits for its dependencies; retry it once they are merged"
        )),
        _ => None,
    }
}

/// Task 17b's review, m2: a lane of the task's race is still racing.
fn has_live_lane(task: &crate::run::model::Task) -> bool {
    let mut lanes = task.race.iter().flat_map(|r| &r.lanes);
    lanes.any(|l| super::super::race_view::live(l.state))
}

/// `run override` (`gates::override_task`, decision 35), up to the count of a blocked
/// task's commits, whose result can still refuse it.
pub(crate) fn override_task(run: &Run, task_id: &str) -> Option<String> {
    if !matches!(run.state, RunState::Running | RunState::Paused) {
        return Some(is(run));
    }
    let Some(i) = run.tasks.iter().position(|t| t.id() == task_id) else {
        return Some(format!("unknown task {task_id}"));
    };
    let task = &run.tasks[i];
    // M9.9 review fixes, M1: research and review tasks, integration reviews among
    // them, merge nothing, so nothing is overridden into the merge queue.
    if schedule::is_reader_task(task) {
        return Some(OVERRIDE_KINDS.to_string());
    }
    // Milestone 9 decision 42c.
    let paused = worker_messages::paused_refusal(task);
    if let Some(text) = paused.or_else(|| override_refusal(run, i)) {
        return Some(text);
    }
    if task.override_count.is_some() {
        return Some(format!(
            "task {task_id}'s commits are being counted for an override; wait for its reply"
        ));
    }
    (task.state != TaskState::Review && task.start_commit.is_none())
        .then(|| format!("task {task_id} has no commits; {OVERRIDE_APPLIES}"))
}

/// Why task `i` cannot be overridden now, if it cannot: a paired task whose test is
/// still being written waits for its implementer (milestone 9.5 ruling T16-5: a failing
/// test alone never merges); a held or `dep_cancelled` task waits for its dependencies;
/// any task but one in `review` or `blocked` is refused.
pub(crate) fn override_refusal(run: &Run, i: usize) -> Option<String> {
    let task = &run.tasks[i];
    let id = task.id();
    if super::super::pair::writing(task) {
        return Some(format!(
            "task {id} is still writing its test; override it once its implementer has started"
        ));
    }
    // Milestone 9.5 decision 23.
    if task.racing() {
        return Some(format!(
            "task {id} is racing: there is no failed gate to override"
        ));
    }
    let dep_cancelled = task
        .block
        .as_ref()
        .is_some_and(|b| b.reason == BlockReason::DepCancelled);
    if task.awaiting_deps || dep_cancelled {
        return Some(format!(
            "task {id} waits for its dependencies; override it once they are merged"
        ));
    }
    if !matches!(task.state, TaskState::Review | TaskState::Blocked) {
        return Some(format!(
            "task {id} is {}; {OVERRIDE_APPLIES}",
            task.state.label()
        ));
    }
    None
}

/// A `cancel_task` edit (`Batch::cancel`): an unfinished task.
pub(crate) fn cancel_task(run: &Run, task_id: &str) -> Option<String> {
    let Some(task) = run.task(task_id) else {
        return Some(no_such_task(task_id));
    };
    task.state.is_finished().then(|| {
        format!(
            "task {task_id} is {}; only unfinished tasks can be cancelled",
            state_label(task)
        )
    })
}
