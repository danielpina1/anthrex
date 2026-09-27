//! The watch-only guards for headless run sessions (decision 49, milestone 8c decision
//! 26): which window input may reach, and the refusal `C-b x`, `C-b X` and `C-b R`
//! toast. Moved out of `app/lifecycle.rs` and `app/windows.rs` (`AGENTS.md` hard rule 8).

use super::App;
use proto::WindowKind;

impl App {
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
}
