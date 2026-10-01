//! Milestone 9.0.7 decision 1: which frame has the keys. Every frame-drawing renderer
//! asks `App::key_region` for its border colour, so exactly one frame wears the
//! accent (§5.1 principle 2). Pure.

use super::App;

/// The region that receives keystrokes, first match wins, in the keymap's own
/// precedence (`Keymap::handle`) with the modal first (`App::on_key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyRegion {
    /// A modal: a dialog, a confirm, the action menu or the help.
    Dialog,
    /// The Profile, Settings or stats screen.
    Screen,
    /// The plan review.
    Review,
    /// The alerts.
    Alerts,
    /// The conversation view.
    Conversation,
    /// The project overview, or the run view inside it.
    Overview,
    /// The sidebar tree.
    Sidebar,
    /// The terminal pane.
    Pane,
}

impl App {
    /// A hidden sidebar holds no keys (decision 1): `C-b s` hiding it leaves the
    /// sidebar tree and the Alerts box, so the keys reach the pane that is drawn and
    /// its frame is the one accented. The overview keeps its keys: its frame is the
    /// main pane, which stays.
    pub(super) fn release_a_hidden_sidebar(&mut self) {
        if self.sidebar_visible {
            return;
        }
        if self.tree_input.is_some() && !self.overview {
            self.exit_tree();
        }
        if self.alerts_focus.is_some() {
            self.leave_alerts();
        }
    }

    /// Decision 1's table.
    pub fn key_region(&self) -> KeyRegion {
        if self.modal.is_some() {
            KeyRegion::Dialog
        } else if self.screen.is_some() {
            KeyRegion::Screen
        } else if self.plan_review.is_some() {
            KeyRegion::Review
        } else if self.alerts_focus.is_some() {
            KeyRegion::Alerts
        } else if self.conversation.is_open() {
            KeyRegion::Conversation
        } else if self.tree_input.is_some() && self.overview {
            KeyRegion::Overview
        } else if self.tree_input.is_some() {
            KeyRegion::Sidebar
        } else {
            KeyRegion::Pane
        }
    }
}
