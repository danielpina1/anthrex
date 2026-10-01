//! What a confirm modal says about its action (milestone 9.0.6 decision 5): the verb on
//! its key hint and in the `press y to <verb>` toast, and whether it is destructive, in
//! which case only `y` confirms and the title and verb draw in `Failed`.

use super::PendingAction;

impl PendingAction {
    /// The action word of the confirm's hint and its Enter toast.
    pub fn verb(&self) -> &'static str {
        match self {
            PendingAction::Kill(_) => "kill",
            PendingAction::Restart(_) => "restart",
            PendingAction::StopDaemon => "stop the daemon",
            PendingAction::ApproveRun(_) => "approve",
            PendingAction::RejectRun(_) => "reject",
            PendingAction::RemoveTask { .. } => "remove",
            PendingAction::RejectHold { .. } => "reject hold",
            PendingAction::SubmitPlan(_) => "submit",
        }
    }

    /// Destructive confirms (decision 5, ruling F11): a bare Enter does nothing.
    pub fn destructive(&self) -> bool {
        matches!(
            self,
            PendingAction::RejectRun(_)
                | PendingAction::RemoveTask { .. }
                | PendingAction::RejectHold { .. }
                | PendingAction::StopDaemon
                | PendingAction::Kill(_)
        )
    }
}
