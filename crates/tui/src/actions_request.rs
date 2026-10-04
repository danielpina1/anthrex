//! The one request each action menu entry sends (milestone 9.0.6 §1.1, Global
//! Constraint 2): every daemon-backed `ActionKind` maps to exactly one existing
//! `RunRequest`, the one the CLI sends for the same step. Pure, and public so the
//! end-to-end tests build their requests with the TUI's own function.

use proto::{
    ActionKind, BaseMovedInfo, FinishAction, MessageKind, MessageTarget, OrchestratorChoice,
    PlanEdit, RunInfo, RunRequest,
};

/// The node an action applies to, inside its run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionTarget {
    Run,
    Stage(u16),
    Task(String),
}

/// What the user gave an action's input form (task 11 builds the forms); `None` for a
/// `Confirm` action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionInput {
    None,
    Answer(String),
    Message { kind: MessageKind, text: String },
    Reason(String),
    Resume { rebaseline: bool },
    Promote(Option<OrchestratorChoice>),
}

/// The request `kind` on `target` of `run` sends, given `input`. `None` for the kinds
/// the client handles itself (decision 10), for a kind on the wrong node, and for an
/// input form's kind without its input.
pub fn request_for(
    run: &RunInfo,
    target: &ActionTarget,
    kind: &ActionKind,
    input: &ActionInput,
) -> Option<RunRequest> {
    let run_id = run.run_id.clone();
    let edit = |edits: Vec<PlanEdit>, submit: bool| RunRequest::Edit {
        run_id: run.run_id.clone(),
        edits,
        submit,
    };
    let task = match target {
        ActionTarget::Task(id) => Some(id.clone()),
        _ => None,
    };
    match (kind, target) {
        (ActionKind::Approve, ActionTarget::Run) => Some(RunRequest::Approve { run_id }),
        (ActionKind::Reject, ActionTarget::Run) => Some(RunRequest::Reject { run_id }),
        (ActionKind::Submit, ActionTarget::Run) => Some(edit(vec![], true)),
        (ActionKind::ApproveHold { hold }, ActionTarget::Run) => Some(RunRequest::ApproveHold {
            run_id,
            hold: hold.clone(),
        }),
        (ActionKind::RejectHold { hold }, ActionTarget::Run) => Some(RunRequest::RejectHold {
            run_id,
            hold: hold.clone(),
        }),
        (ActionKind::Pause, ActionTarget::Run) => Some(edit(vec![PlanEdit::Pause], false)),
        (ActionKind::Unpause, ActionTarget::Run) => Some(edit(vec![PlanEdit::Resume], false)),
        // A held tier 3 on a running run is retried with no form (preflight F32).
        (ActionKind::Resume, ActionTarget::Run) => match input {
            ActionInput::Resume { rebaseline } => Some(RunRequest::Resume {
                run_id,
                rebaseline: *rebaseline,
            }),
            ActionInput::None => Some(RunRequest::Resume {
                run_id,
                rebaseline: false,
            }),
            _ => None,
        },
        (ActionKind::Cancel, ActionTarget::Run) => Some(RunRequest::Cancel { run_id }),
        (ActionKind::Promote, ActionTarget::Run) => match input {
            ActionInput::Promote(orchestrator) => Some(RunRequest::Promote {
                run_id,
                orchestrator: orchestrator.clone(),
            }),
            ActionInput::None => Some(RunRequest::Promote {
                run_id,
                orchestrator: None,
            }),
            _ => None,
        },
        (ActionKind::Accept, ActionTarget::Run) => Some(finish(&run.run_id, FinishAction::Accept)),
        (ActionKind::Discard, ActionTarget::Run) => {
            Some(finish(&run.run_id, FinishAction::Discard))
        }
        (ActionKind::MessageStage { stage }, ActionTarget::Stage(n)) if stage == n => {
            let ActionInput::Message { kind, text } = input else {
                return None;
            };
            Some(edit(
                vec![PlanEdit::Message {
                    to: MessageTarget::Stage(u32::from(*stage)),
                    text: text.clone(),
                    kind: *kind,
                }],
                false,
            ))
        }
        (ActionKind::Answer, ActionTarget::Task(_)) => {
            let ActionInput::Answer(text) = input else {
                return None;
            };
            Some(edit(
                vec![PlanEdit::Answer {
                    task_id: task?,
                    text: text.clone(),
                }],
                false,
            ))
        }
        (ActionKind::Message, ActionTarget::Task(_)) => {
            let ActionInput::Message { kind, text } = input else {
                return None;
            };
            Some(edit(
                vec![PlanEdit::Message {
                    to: MessageTarget::Tasks(vec![task?]),
                    text: text.clone(),
                    kind: *kind,
                }],
                false,
            ))
        }
        (ActionKind::Refresh, ActionTarget::Task(_)) => {
            Some(edit(vec![PlanEdit::Refresh { task_id: task? }], false))
        }
        (ActionKind::Retry, ActionTarget::Task(_)) => Some(RunRequest::Retry {
            run_id,
            task_id: task?,
        }),
        (ActionKind::Override, ActionTarget::Task(_)) => {
            let ActionInput::Reason(reason) = input else {
                return None;
            };
            Some(RunRequest::Override {
                run_id,
                task_id: task?,
                reason: reason.clone(),
            })
        }
        (ActionKind::CancelTask, ActionTarget::Task(_)) => {
            Some(edit(vec![PlanEdit::CancelTask { task_id: task? }], false))
        }
        // Milestone 9.3 decision 32: `iterate` is local here; the menu entry opens the
        // iterate dialog, whose Ctrl-S sends [`iterate`] itself.
        _ => None,
    }
}

/// Milestone 9.3 decision 32: `anthrex run iterate <run> <text>`'s request.
pub fn iterate(run_id: &str, goal: &str) -> RunRequest {
    RunRequest::Iterate {
        run: run_id.to_string(),
        goal: goal.to_string(),
        design: None,
    }
}

/// Decision 18: accept and discard name the run they finish, so the driver's own
/// confirmation is already given.
fn finish(run_id: &str, action: FinishAction) -> RunRequest {
    RunRequest::Finish {
        run_id: run_id.to_string(),
        action,
        confirm: Some(run_id.to_string()),
    }
}

/// Decision 18, review focus 4: the confirmation an accept onto a moved base resends,
/// `"<run id>@<to>"` with the **full** `to` sha, the string the driver compares.
pub fn moved_base_confirm(run_id: &str, moved: &BaseMovedInfo) -> String {
    format!("{run_id}@{}", moved.to)
}

/// A run's short id: the last four characters of its id (principle 5).
pub fn short_id(run_id: &str) -> &str {
    let start = run_id
        .char_indices()
        .rev()
        .nth(3)
        .map(|(at, _)| at)
        .unwrap_or(0);
    &run_id[start..]
}

#[cfg(test)]
#[path = "actions_request_tests.rs"]
mod tests;
