//! Milestone 9.0.6 decisions 8 and 9: what a client may do on a run, a stage or a task.
//! `rules.rs` holds every run request's precondition, the one source of each refusal
//! text, which the request handlers call; [`check`] composes them in each handler's
//! order, so a listed action is refused in exactly the handler's words; [`available`]
//! lists a node's relevant kinds for the snapshot. Pure (design decision 2).

mod effects;
pub(crate) mod rules;

pub(crate) use effects::label;
use proto::{
    ActionInfo, ActionKind, ActionNeeds, BlockReason, FinishAction, HoldState, MessageKind,
    MessageTarget, PlanEdit, RunPath, RunState, TaskState,
};

use super::{full, orch, schedule};
use crate::run::edits_state::is_paused;
use crate::run::model::{Run, StageLayout};

/// A node of a run's tree an action applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ActionNode<'a> {
    Run,
    Stage(u16),
    Task(&'a str),
}

/// Every request-backed kind for `node`, relevant or not (the matrix test iterates
/// these): a hold's verdicts for every hold the run has.
pub(crate) fn request_kinds(run: &Run, node: &ActionNode) -> Vec<ActionKind> {
    use ActionKind::*;
    match node {
        ActionNode::Run => {
            let mut kinds = vec![Approve, Reject, Submit];
            for h in &run.orch.gate_holds {
                kinds.push(ApproveHold { hold: h.id.clone() });
                kinds.push(RejectHold { hold: h.id.clone() });
            }
            kinds.extend([Pause, Unpause, Resume, Cancel, Promote, Accept, Discard]);
            kinds
        }
        ActionNode::Stage(stage) => vec![MessageStage { stage: *stage }],
        ActionNode::Task(_) => vec![Answer, Message, Refresh, Retry, Override, CancelTask],
    }
}

/// Decision 9: the relevant kinds of `node`, each with its label, its effect line and
/// `refused_why` (`check`'s refusal), both sanitised and cut to `ACTION_TEXT_MAX`.
/// A terminal run lists nothing.
pub(crate) fn available(run: &Run, node: &ActionNode) -> Vec<ActionInfo> {
    if run.state.is_terminal() {
        return Vec::new();
    }
    request_kinds(run, node)
        .into_iter()
        .filter(|kind| relevant(run, node, kind))
        .map(|kind| ActionInfo {
            label: clean(&label(&kind)),
            effect: clean(&effects::effect(run, node, &kind)),
            needs: needs(run, &kind),
            destructive: kind.destructive(),
            refused_why: check(run, node, &kind).err().map(|text| clean(&text)),
            kind,
        })
        .collect()
}

/// Preflight F32: a held tier 3's resume takes no form (a plain `Resume`).
fn needs(run: &Run, kind: &ActionKind) -> ActionNeeds {
    match kind {
        ActionKind::Resume if run.state == RunState::Running => ActionNeeds::Confirm,
        kind => kind.needs(),
    }
}

/// `text` on one line, at most `ACTION_TEXT_MAX` characters, cut with `…`.
fn clean(text: &str) -> String {
    let line = proto::safe_text::one_line(text);
    if line.chars().count() <= proto::ACTION_TEXT_MAX {
        return line;
    }
    let mut out: String = line.chars().take(proto::ACTION_TEXT_MAX - 1).collect();
    out.push('…');
    out
}

/// Decision 9, row by row.
fn relevant(run: &Run, node: &ActionNode, kind: &ActionKind) -> bool {
    use ActionKind::*;
    let state = run.state;
    let at_gate = matches!(state, RunState::AwaitingApproval | RunState::Planning)
        || (state == RunState::Paused && run.paused_from == Some(RunState::Planning));
    let live = matches!(
        state,
        RunState::Running | RunState::Paused | RunState::Halted
    );
    let complete = state == RunState::Complete;
    match (node, kind) {
        // A complete run lists accept and discard only.
        (ActionNode::Run, Accept | Discard) => live || complete,
        (_, _) if complete => false,
        (ActionNode::Run, Approve | Reject) => at_gate,
        (ActionNode::Run, Submit) => {
            state == RunState::Planning || rules::promoted_unsubmitted(run)
        }
        (ActionNode::Run, ApproveHold { hold } | RejectHold { hold }) => run
            .orch
            .gate_holds
            .iter()
            .any(|h| h.id == *hold && h.state == HoldState::Awaiting),
        (ActionNode::Run, Pause) => state == RunState::Running,
        (ActionNode::Run, Unpause) => state == RunState::Paused,
        // Preflight F32: a running run whose tier 3 is held resumes it too.
        (ActionNode::Run, Resume) => {
            state == RunState::Halted || (state == RunState::Running && full::held(run))
        }
        (ActionNode::Run, Cancel) => live,
        (ActionNode::Run, Promote) => run.path == Some(RunPath::Fast),
        // Milestone 9.1 decision 55: only a `Multi` run has stage nodes.
        (ActionNode::Stage(n), MessageStage { stage }) => {
            run.stage_layout == StageLayout::Multi
                && n == stage
                && run
                    .tasks
                    .iter()
                    .any(|t| t.stage() == *n && !t.state.is_finished())
        }
        (ActionNode::Task(id), kind) => task_relevant(run, id, kind),
        _ => false,
    }
}

/// Decision 9's task row: only an unfinished task lists anything.
fn task_relevant(run: &Run, id: &str, kind: &ActionKind) -> bool {
    use ActionKind::*;
    let Some(task) = run.task(id).filter(|t| !t.state.is_finished()) else {
        return false;
    };
    let blocked = task.state == TaskState::Blocked;
    let working = task.state == TaskState::Working;
    match kind {
        Answer => {
            working
                || (blocked
                    && task
                        .block
                        .as_ref()
                        .is_some_and(|b| b.reason == BlockReason::Question))
        }
        Message => !schedule::is_reader_task(task),
        Refresh => working || is_paused(task),
        Retry => blocked,
        Override => blocked || task.state == TaskState::Review,
        CancelTask => true,
        _ => false,
    }
}

/// Decision 8: `Err(text)` exactly when the engine's handler for `kind` on `node`,
/// given a valid input (a resume with a rebaseline, preflight F31; a message of kind
/// `info`), refuses it, with that handler's text. Each arm asks the rules in the
/// handler's order: a `run edit`'s run-wide checks, then its edit's.
pub(crate) fn check(run: &Run, node: &ActionNode, kind: &ActionKind) -> Result<(), String> {
    use ActionKind::*;
    let edit = |edits: &[PlanEdit], rule: &dyn Fn() -> Option<String>| {
        rules::edit_run(run, edits, false).or_else(rule)
    };
    let task = || match node {
        ActionNode::Task(id) => *id,
        _ => "",
    };
    let refusal = match kind {
        Approve => rules::approve(run),
        Reject => rules::reject(run),
        Submit => rules::edit_run(run, &[], true)
            .or_else(|| rules::submit(run))
            .or_else(|| orch::submit_refusal(run)),
        ApproveHold { hold } | RejectHold { hold } => rules::hold(run, hold),
        Pause => edit(&[PlanEdit::Pause], &|| rules::pause(run)),
        Unpause => edit(&[PlanEdit::Resume], &|| rules::unpause(run)),
        Resume => rules::resume(run, true),
        Cancel => rules::cancel(run),
        Promote => rules::promote(run),
        Accept => rules::finish(run, FinishAction::Accept),
        Discard => rules::finish(run, FinishAction::Discard),
        MessageStage { stage } => {
            let to = MessageTarget::Stage(u32::from(*stage));
            edit(&[message(to)], &|| rules::message_stage(run, *stage))
        }
        Answer => {
            let answer = PlanEdit::Answer {
                task_id: task().into(),
                text: String::new(),
            };
            edit(&[answer], &|| rules::answer(run, task()))
        }
        Message => {
            let to = MessageTarget::Tasks(vec![task().into()]);
            edit(&[message(to)], &|| rules::message(run, task()))
        }
        Refresh => {
            let refresh = PlanEdit::Refresh {
                task_id: task().into(),
            };
            edit(&[refresh], &|| rules::refresh(run, task()))
        }
        Retry => rules::retry(run, task()),
        Override => rules::override_task(run, task()),
        CancelTask => {
            let cancel = PlanEdit::CancelTask {
                task_id: task().into(),
            };
            edit(&[cancel], &|| rules::cancel_task(run, task()))
        }
        // Client-only kinds change nothing in the daemon (decision 10).
        ReviewPlan | Stats | OpenConversation => None,
    };
    refusal.map_or(Ok(()), Err)
}

/// An `info` message to `to` (the rules model `info` and `change`, never
/// `stop_and_wait`).
fn message(to: MessageTarget) -> PlanEdit {
    PlanEdit::Message {
        to,
        text: String::new(),
        kind: MessageKind::Info,
    }
}
