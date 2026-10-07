//! The watch-only guards for headless run sessions (decision 49, milestone 8c decision
//! 26): which window input may reach, and the refusal `C-b x`, `C-b X` and `C-b R`
//! toast. Moved out of `app/lifecycle.rs` and `app/windows.rs` (`AGENTS.md` hard rule 8).

use super::App;
use proto::{AgentRole, WindowKind};

/// The toast when a bare Esc is held back from a working orchestrator.
pub(crate) const ESC_HELD: &str =
    "Esc is off in the orchestrator while it works; press Ctrl-C to interrupt it";

impl App {
    /// A stray Esc must not interrupt a live run's orchestrator mid-turn: a bare Esc
    /// (`bytes` is exactly `ESC`) for the focused window is held back, with a toast,
    /// while that window is the `Working` orchestrator of a run this client shows as not
    /// terminal. Ctrl-C is untouched, and Esc passes whenever the orchestrator is not
    /// working, since Claude Code needs it to close its menus and permission dialogs.
    pub(super) fn holds_esc(&mut self, bytes: &[u8]) -> bool {
        if bytes != [0x1b] {
            return false;
        }
        let held = self.focused_window().is_some_and(|w| {
            w.status == proto::Status::Working
                && w.run.as_ref().is_some_and(|run| {
                    run.role == AgentRole::Orchestrator
                        && (self.runs.runs.iter())
                            .any(|r| r.run_id == run.run_id && !r.state.is_terminal())
                })
        });
        if held {
            self.toast(ESC_HELD);
        }
        held
    }

    /// Decision 49: a headless run session's window has no terminal. No `Subscribe`,
    /// `Input` or mouse report is ever sent for it; its pane points at `C-b m`.
    pub(crate) fn is_headless(&self, id: u32) -> bool {
        self.windows
            .iter()
            .any(|w| w.id == id && w.kind == WindowKind::Headless)
    }

    /// The focused window, when it is an ordinary PTY window that input may reach.
    pub(crate) fn focused_pty(&self) -> Option<u32> {
        self.focused.filter(|&id| !self.is_headless(id))
    }

    /// Milestone 8c decision 26: the daemon's own refusal text when the focused window is
    /// headless, so `C-b x`, `C-b X` and `C-b R` toast it and open nothing.
    fn headless_control_refusal(&self) -> Option<String> {
        let w = self.focused_window()?;
        (w.kind == WindowKind::Headless)
            .then(|| daemon::manager::control_refusal(w.id, w.run.as_ref()))
    }

    /// `true`, with the refusal toasted, when a control command must stop here.
    pub(super) fn refuse_headless_control(&mut self) -> bool {
        let Some(refusal) = self.headless_control_refusal() else {
            return false;
        };
        self.toast(refusal);
        true
    }

    /// `refuse_headless_control`, and milestone 9 decision 11's second case for `C-b x`
    /// and `C-b X` only: the focused window is the orchestrator of a run this client
    /// shows as not terminal. `C-b R` stays allowed. A placeholder: none (9.5 d. 44).
    pub(super) fn refuse_kill_or_remove(&mut self) -> bool {
        if self.focused_window().is_some_and(|w| w.placeholder) {
            return false;
        }
        if self.refuse_headless_control() {
            return true;
        }
        let refusal = self.focused_window().and_then(|w| {
            let run = w
                .run
                .as_ref()
                .filter(|r| r.role == AgentRole::Orchestrator)?;
            let shown = self.runs.runs.iter().find(|r| r.run_id == run.run_id)?;
            (!shown.state.is_terminal())
                .then(|| daemon::manager::orchestrator_refusal(w.id, &run.run_id))
        });
        let Some(refusal) = refusal else {
            return false;
        };
        self.toast(refusal);
        true
    }
}
