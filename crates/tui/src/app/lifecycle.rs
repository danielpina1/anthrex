//! Task M6.10: the restart confirmation (decision 23) and dispatching `PendingAction`.
//! Split out of `app/mod.rs` (`AGENTS.md` hard rule 8) — the same shape
//! `modal_keys.rs` and `windows.rs` already give their own tasks' `impl App` blocks.
//! `C-b Q`'s wait to stop the daemon (decision 36) used to be armed here too, but
//! decision 39 names `app/link.rs` as where all of `stopping`'s pure logic belongs;
//! a follow-up (closing the M6.10 review's naming-mismatch finding) moved
//! `start_stopping` and the pre-confirm guard there alongside `on_link_lost`,
//! `on_send_failed` and `check_stopping_timeout`, the three ways the wait ends. This
//! file's `perform` arm for `PendingAction::StopDaemon` only dispatches to it.

use super::{App, Effect, Modal, PendingAction};
use proto::{ClientMsg, PlanEdit, RunRequest, Status};

fn run(request: RunRequest) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::Run(request))]
}

impl App {
    pub(super) fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
            PendingAction::Restart(id) => {
                vec![Effect::Send(ClientMsg::Restart { window_id: id })]
            }
            // Decision 36: only the send. Quitting happens once the daemon actually
            // confirms — see `app/link.rs`'s `on_link_lost`, `on_send_failed` and
            // `check_stopping_timeout` for the three ways that wait can end.
            // `start_stopping` lives there too (decision 39): this arm only dispatches.
            PendingAction::StopDaemon => self.start_stopping(),
            // Milestone 8c decision 32: the user's own gate requests, after a `y`.
            PendingAction::ApproveRun(run_id) => run(RunRequest::Approve { run_id }),
            PendingAction::RejectRun(run_id) => run(RunRequest::Reject { run_id }),
            PendingAction::RemoveTask { run_id, task_id } => run(RunRequest::Edit {
                run_id,
                edits: vec![PlanEdit::CancelTask { task_id }],
                submit: false,
            }),
            // Milestone 9 decisions 28 and 13, after a `y`.
            PendingAction::RejectHold { run_id, hold } => {
                run(RunRequest::RejectHold { run_id, hold })
            }
            PendingAction::SubmitPlan(run_id) => self.send_submit(run_id),
        }
    }

    /// Decision 23: `C-b R`. An exited window restarts at once; a live one asks first,
    /// through the same generic `Confirm` modal `confirm_focused` uses for `Kill`, with
    /// `PendingAction::Restart(id)` carrying the window id so the eventual `y` cannot
    /// act on whatever happens to be focused by the time it is pressed.
    pub(super) fn restart_focused(&mut self) -> Vec<Effect> {
        if self.refuse_headless_control() {
            return vec![];
        }
        let Some(w) = self.focused_window() else {
            return vec![];
        };
        let (id, name) = (w.id, w.name.clone());
        if w.status == Status::Exited {
            self.toast(format!("restarting {name}"));
            vec![Effect::Send(ClientMsg::Restart { window_id: id })]
        } else {
            self.modal = Some(Modal::Confirm {
                message: format!("Restart '{name}'? It is running and will be stopped first."),
                action: PendingAction::Restart(id),
            });
            vec![]
        }
    }
}
