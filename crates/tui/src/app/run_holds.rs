//! Milestone 9 in the run view's keys: `a` and `x` on an approval hold (decision 28)
//! and `s` on a planning run's root (decision 13). Human-only: every request leaves
//! from the user's key — `a` at once, `x` and `s` after a confirm — and nothing here
//! ever approves, rejects or submits on its own.

use super::run_gate::confirm;
use super::runs::state_text;
use super::{App, Effect, PendingAction};
use crate::tree::{NodeKey, awaiting_holds, task_held};
use proto::{ClientMsg, HoldInfo, HoldState, RunInfo, RunRequest, RunState};

fn hold_state_text(state: HoldState) -> &'static str {
    match state {
        HoldState::Drafting => "drafting",
        HoldState::Awaiting => "awaiting approval",
        HoldState::Approved => "approved",
        HoldState::Rejected => "rejected",
        HoldState::Moot => "moot",
    }
}

/// `x`'s confirm: the hold's tasks are cancelled (none has started).
fn reject_message(run_id: &str, hold: &HoldInfo) -> String {
    let id = &hold.id;
    match hold.tasks.len() {
        1 => format!("Reject hold {id} of run {run_id}? Its 1 task is cancelled."),
        n => format!("Reject hold {id} of run {run_id}? Its {n} tasks are cancelled."),
    }
}

/// Decision 13's confirm, word for word.
fn submit_message(run_id: &str) -> String {
    format!("submit the plan of {run_id} yourself? (y/n)")
}

/// What `a` or `x` acts on.
enum HoldTarget<'a> {
    Hold(&'a HoldInfo),
    NotAwaiting(&'a HoldInfo),
    Several,
}

/// The hold `a` or `x` names for `selected`: a held task's own, or on the root the
/// one hold awaiting approval. `None` when neither applies, so the plan gate's rule
/// does.
fn hold_target<'a>(run: &'a RunInfo, selected: Option<&NodeKey>) -> Option<HoldTarget<'a>> {
    match selected? {
        NodeKey::Task { run: r, id } if *r == run.run_id => {
            let task = run.tasks.iter().find(|task| task.id == *id)?;
            if !task_held(run, task) {
                return None;
            }
            let hold = run
                .holds
                .iter()
                .find(|h| Some(&h.id) == task.hold.as_ref())?;
            Some(if hold.state == HoldState::Awaiting {
                HoldTarget::Hold(hold)
            } else {
                HoldTarget::NotAwaiting(hold)
            })
        }
        NodeKey::Run(r) if *r == run.run_id => {
            let mut awaiting = awaiting_holds(run);
            match (awaiting.next(), awaiting.next()) {
                (Some(hold), None) => Some(HoldTarget::Hold(hold)),
                (Some(_), Some(_)) => Some(HoldTarget::Several),
                (None, _) => None,
            }
        }
        _ => None,
    }
}

impl App {
    fn shown_run(&self, run_id: &str) -> Option<&RunInfo> {
        self.runs.runs.iter().find(|run| run.run_id == run_id)
    }

    /// Decision 28's keys on a run past its gate: `a` sends `ApproveHold`, `x` asks,
    /// then sends `RejectHold`, on the hold `selected` names. `None`: not a hold's key
    /// here.
    pub(super) fn on_hold_key(
        &mut self,
        run_id: &str,
        key: char,
        selected: Option<&NodeKey>,
    ) -> Option<Vec<Effect>> {
        let run = self.shown_run(run_id)?;
        if run.state == RunState::AwaitingApproval || !matches!(key, 'a' | 'x') {
            return None;
        }
        let verb = if key == 'a' { "approve" } else { "reject" };
        let hold = match hold_target(run, selected)? {
            HoldTarget::Hold(hold) | HoldTarget::NotAwaiting(hold) => hold.id.clone(),
            HoldTarget::Several => {
                self.toast(format!("select a held task to {verb} its hold"));
                return Some(vec![]);
            }
        };
        Some(self.decide_hold(run_id, key, &hold))
    }

    /// `a` (sends `ApproveHold` at once) or `x` (asks, then `RejectHold`) on the hold
    /// `hold` of `run_id`, named by the caller: the run view's selection, or the plan
    /// review's own hold (milestone 9.0.5 review finding 6). A hold not awaiting
    /// approval, or not listed, toasts why.
    pub(super) fn decide_hold(&mut self, run_id: &str, key: char, hold: &str) -> Vec<Effect> {
        let done = if key == 'a' { "approved" } else { "rejected" };
        let found = self
            .shown_run(run_id)
            .and_then(|run| run.holds.iter().find(|h| h.id == hold));
        let message = match found {
            Some(h) if h.state == HoldState::Awaiting => reject_message(run_id, h),
            Some(h) => {
                let text = format!(
                    "hold {} is {}; it can be {done} once it awaits approval",
                    h.id,
                    hold_state_text(h.state)
                );
                self.toast(text);
                return vec![];
            }
            None => {
                self.toast(format!("run {run_id} has no hold {hold}"));
                return vec![];
            }
        };
        let (run_id, hold) = (run_id.to_owned(), hold.to_owned());
        if key == 'a' {
            let request = RunRequest::ApproveHold { run_id, hold };
            return vec![Effect::Send(ClientMsg::Run(request))];
        }
        self.modal = Some(confirm(message, PendingAction::RejectHold { run_id, hold }));
        vec![]
    }

    /// Decision 13's `s`: on a planning run's root, the confirm that sends the user's
    /// own submit. `false`: not here, so `s` does nothing new.
    pub(super) fn on_submit_key(&mut self, run_id: &str) -> bool {
        let planning = self
            .shown_run(run_id)
            .is_some_and(|run| run.state == RunState::Planning);
        if !planning || self.tree.selected != Some(NodeKey::Run(run_id.to_owned())) {
            return false;
        }
        let action = PendingAction::SubmitPlan(run_id.to_owned());
        self.modal = Some(confirm(submit_message(run_id), action));
        true
    }

    /// The user's submit, tagged (decision 2); its reply is toasted.
    pub(super) fn send_submit(&mut self, run_id: String) -> Vec<Effect> {
        let request = RunRequest::Edit {
            run_id,
            edits: vec![],
            submit: true,
        };
        vec![self.tagged_request(request).1]
    }

    /// A reject confirm whose hold was decided, or left, meanwhile.
    pub(super) fn stale_hold(&self, run_id: &str, hold: &str) -> Option<String> {
        let found = self
            .shown_run(run_id)
            .and_then(|run| run.holds.iter().find(|h| h.id == hold));
        match found {
            None => Some(format!("run {run_id} has no hold {hold}")),
            Some(h) if h.state != HoldState::Awaiting => {
                Some(format!("hold {hold} is {}", hold_state_text(h.state)))
            }
            Some(_) => None,
        }
    }

    /// A submit confirm whose run is no longer being planned.
    pub(super) fn stale_submit(&self, run_id: &str) -> Option<String> {
        match self.shown_run(run_id) {
            Some(run) if run.state == RunState::Planning => None,
            Some(run) => Some(format!(
                "run {run_id} is {}; only a run being planned can be submitted",
                state_text(run.state)
            )),
            None => Some(format!("run {run_id} is gone")),
        }
    }
}
