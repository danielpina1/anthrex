//! The action menu's wire types (milestone 9.0.6 §1): what a run, stage or task node
//! can be asked to do right now, built by the daemon into the snapshot.

use serde::{Deserialize, Serialize};

/// Characters kept in an `ActionInfo`'s `effect` and `refused_why` (decision 8).
pub const ACTION_TEXT_MAX: usize = 200;

/// Every entry the action menu can list. `ReviewPlan`, `Stats` and `OpenConversation`
/// change nothing in the daemon; the client adds them itself (decision 10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionKind {
    ReviewPlan,
    Approve,
    Reject,
    Submit,
    ApproveHold {
        hold: String,
    },
    RejectHold {
        hold: String,
    },
    Pause,
    Unpause,
    Resume,
    Cancel,
    Promote,
    Accept,
    Discard,
    Stats,
    MessageStage {
        stage: u16,
    },
    Answer,
    Message,
    Refresh,
    Retry,
    Override,
    CancelTask,
    OpenConversation,
    /// Milestone 9.3 (KG §2.2): opens the iterate dialog.
    Iterate,
    /// Milestone 9.6 decision 34: opens the brainstorm or spec gate's screen.
    ReviewDoc,
}

/// Which input form an action opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputKind {
    Answer,
    Message,
    Reason,
    Resume,
    Promote,
}

/// What picking an action asks of the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionNeeds {
    Confirm,
    Input(InputKind),
    Open,
}

/// One menu entry. `refused_why` is `Some` when the action is listed but cannot be
/// taken now, with the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub kind: ActionKind,
    pub label: String,
    pub effect: String,
    pub needs: ActionNeeds,
    pub destructive: bool,
    #[serde(default)]
    pub refused_why: Option<String>,
}

impl ActionKind {
    /// The kinds only the client adds (decision 10).
    pub fn is_local(&self) -> bool {
        matches!(
            self,
            ActionKind::ReviewPlan | ActionKind::Stats | ActionKind::OpenConversation
        )
    }

    /// §1.1's "Needs" column.
    pub fn needs(&self) -> ActionNeeds {
        use ActionKind::*;
        match self {
            ReviewPlan | Stats | OpenConversation | Iterate | ReviewDoc => ActionNeeds::Open,
            Answer => ActionNeeds::Input(InputKind::Answer),
            Message | MessageStage { .. } => ActionNeeds::Input(InputKind::Message),
            Override => ActionNeeds::Input(InputKind::Reason),
            Resume => ActionNeeds::Input(InputKind::Resume),
            Promote => ActionNeeds::Input(InputKind::Promote),
            Approve
            | Reject
            | Submit
            | ApproveHold { .. }
            | RejectHold { .. }
            | Pause
            | Unpause
            | Cancel
            | Accept
            | Discard
            | Refresh
            | Retry
            | CancelTask => ActionNeeds::Confirm,
        }
    }

    /// Drawn in the failure colour and confirmed with `y` only.
    pub fn destructive(&self) -> bool {
        matches!(
            self,
            ActionKind::Reject
                | ActionKind::RejectHold { .. }
                | ActionKind::Cancel
                | ActionKind::Discard
                | ActionKind::CancelTask
        )
    }
}
