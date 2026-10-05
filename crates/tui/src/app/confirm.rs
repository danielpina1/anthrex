//! What a confirm modal says about its action (milestone 9.0.6 decision 5): the verb on
//! its key hint and in the `press y to <verb>` toast, and whether it is destructive, in
//! which case only `y` confirms and the title and verb draw in `Failed`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingAction {
    Kill(u32),
    /// Decision 23: a live window's restart confirmation. Carries the window id
    /// directly, the same way `Kill` does, rather than an index or a reliance on
    /// `App.focused` staying put — a dialog that instead trusted focus to still name
    /// the right window was milestone 5's worst defect (see `app/modal_keys.rs`'s
    /// `open_force_remove` doc comment for the sibling case this mirrors).
    Restart(u32),
    StopDaemon,
    /// Milestone 8c decision 32: the plan gate's requests, each carrying its run.
    ApproveRun(String),
    RejectRun(String),
    RemoveTask {
        run_id: String,
        task_id: String,
    },
    /// Milestone 9 decisions 28 and 13: a hold's rejection, and the user's submit.
    RejectHold {
        run_id: String,
        hold: String,
    },
    SubmitPlan(String),
    /// Milestone 9.6 decision 35: a document gate's action, after its page.
    DocGate {
        run_id: String,
        kind: proto::DocGateKind,
        action: proto::DocGateAction,
    },
}

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
            PendingAction::DocGate { action, .. } => match action {
                proto::DocGateAction::Approve { .. } => "approve",
                proto::DocGateAction::Reject => "reject",
                proto::DocGateAction::Rethink { .. } => "rethink",
                proto::DocGateAction::Back { .. } => "go back",
                proto::DocGateAction::Changes { .. } => "ask for changes",
                proto::DocGateAction::Edit { .. } => "save",
            },
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
                | PendingAction::DocGate {
                    action: proto::DocGateAction::Reject,
                    ..
                }
        )
    }
}
