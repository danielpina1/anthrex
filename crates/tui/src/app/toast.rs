//! The status bar's toast and its severity (9.0.6 decision 6), apart from `app/mod.rs`.

use super::App;
use std::time::Instant;

/// A toast's severity (9.0.6 decision 6): info plain, a warning `Attention`, an error `Failed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastLevel {
    Info,
    Warn,
    Error,
}

pub(super) struct Toast {
    pub(super) text: String,
    pub(super) at: Instant,
    pub(super) level: ToastLevel,
}

impl App {
    pub fn toast_text(&self) -> Option<&str> {
        self.toast.as_ref().map(|t| t.text.as_str())
    }

    /// The severity of the toast on screen, if any (decision 6): the status bar draws an
    /// error in `Failed` and a warning in `Attention`.
    pub fn toast_level(&self) -> Option<ToastLevel> {
        self.toast.as_ref().map(|t| t.level)
    }

    /// Shows `text` in the status bar for [`super::TOAST_TTL`], on one line and with no
    /// control or bidi character: a toast often quotes the daemon, or an id an agent chose.
    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast_at(ToastLevel::Info, text);
    }

    /// [`App::toast`] with a severity.
    pub fn toast_at(&mut self, level: ToastLevel, text: impl Into<String>) {
        let text = crate::safe_text::one_line(&text.into());
        self.toast = Some(Toast {
            text,
            at: Instant::now(),
            level,
        });
    }
}
