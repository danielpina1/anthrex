//! Milestone 9.3 decision 32 (KG §2.2, §1.5, §6): the iterate dialog the action menu's
//! `iterate` opens, pure (`AGENTS.md` hard rule 5). It holds one request in the goal
//! dialog's nano-like editor (`TextArea::on_editor_key`, Enter a newline, capped at
//! `proto::GOAL_MAX_CHARS` characters), and no options: Ctrl-S sends
//! `RunRequest::Iterate`, and Esc on a text asks before discarding it, on the goal
//! dialog's confirm page (decision 8). It keeps no draft. Opening, sending and the
//! replies are `app/iterate.rs`; rendering is `ui/run_iterate.rs`.
//!
//! Milestone 9.6 (DF §6.2, §8.1): a design run's dialog has one option, the `design`
//! row (`amend`, `full`, `off`; `amend` by default, as the daemon defaults), reached by
//! Tab from the text, and shows the run's own round mode beside it. A run without the
//! flow has no row and sends no mode, so the daemon's `does not use the design flow` is
//! never provoked.

use crate::run_goal::EditorView;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{RoundDesign, RunRequest};

/// KG §2.2's prompt line (exact).
pub const PROMPT: &str = "what should change or be added?";
/// The inline error of Ctrl-S on a blank request (the goal dialog's `type a goal
/// first`, for a request).
pub const EMPTY_REQUEST: &str = "type a request first";
/// The inline error after the connection refused the request or the link was lost.
pub const NOT_SENT: &str = "the request was not sent; press Ctrl-S to retry";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IterateForm {
    /// The run the request is for, chosen when the dialog opened; never changed.
    pub run_id: String,
    /// The round the request would start: the run's current round plus one, as the
    /// snapshot had it when the dialog opened.
    pub round: u32,
    pub text: TextArea,
    pub error: Option<String>,
    pub submitting: bool,
    /// The id of the tagged `Iterate` this dialog waits on (milestone 9 decision 2).
    pub request_id: Option<u64>,
    /// Decision 8's confirm page, which `Esc` on a text opened.
    pub discarding: bool,
    /// Milestone 9.6 (DF §8.1): a design run's round mode for the request; `None` on a
    /// run without the flow, whose dialog has no `design` row.
    pub design: Option<RoundDesign>,
    /// The run's own current round mode, shown after the row.
    pub current: Option<RoundDesign>,
    /// The `design` row has the keys, not the text.
    pub on_design: bool,
}

// Protocol 19 grew `ClientMsg`/`RunRequest` (the role table, `OrchestratorChoice.effort`); these
// values are built once per key press, so boxing would only add noise.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum IterateOutcome {
    Stay,
    /// The dialog closes (Esc on an empty text, while submitting, or the confirm
    /// page's `y`); nothing is kept.
    Close,
    Submit(RunRequest),
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

impl IterateForm {
    pub fn new(run_id: String, round: u32) -> Self {
        Self {
            run_id,
            round,
            text: TextArea::editor(""),
            error: None,
            submitting: false,
            request_id: None,
            discarding: false,
            design: None,
            current: None,
            on_design: false,
        }
    }

    /// Milestone 9.6: the `design` row, for a design run whose current round runs
    /// `current` (`None` when the snapshot does not say).
    pub fn with_design(mut self, current: Option<RoundDesign>) -> Self {
        self.design = Some(RoundDesign::Amend);
        self.current = current;
        self
    }

    /// The dialog's keys, the text drawn as `view` says (the goal dialog's, decisions 7
    /// and 8). The confirm page takes the next key: a plain `y` closes, any other goes
    /// back to the text. `Esc` and `Ctrl-C` close at once while submitting or on an
    /// empty text, and ask on any other. While submitting nothing else acts. Ctrl-S
    /// sends; every other key is the editor's (a key it hands back does nothing: the
    /// dialog has no options to move to).
    pub fn on_key_in(&mut self, key: KeyEvent, view: EditorView) -> IterateOutcome {
        if self.discarding {
            self.discarding = false;
            let plain = !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            if key.code == KeyCode::Char('y') && plain {
                return IterateOutcome::Close;
            }
            return IterateOutcome::Stay;
        }
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            if self.submitting || self.text.is_empty() {
                return IterateOutcome::Close;
            }
            self.discarding = true;
            return IterateOutcome::Stay;
        }
        if self.submitting {
            return IterateOutcome::Stay;
        }
        if is_ctrl(&key, 's') {
            return self.submit();
        }
        if self.on_design {
            return self.on_design_key(key);
        }
        let handed_back = self.text.on_editor_key(key, view.width, view.rows)
            == crate::text_area::EditorKey::Unhandled;
        if handed_back
            && self.design.is_some()
            && matches!(key.code, KeyCode::Tab | KeyCode::BackTab)
        {
            self.on_design = true;
        }
        IterateOutcome::Stay
    }

    /// The `design` row's keys: ←, → and Space cycle `amend`, `full`, `off`; Tab,
    /// Shift-Tab, Up and Down go back to the text; Enter sends; nothing else acts.
    fn on_design_key(&mut self, key: KeyEvent) -> IterateOutcome {
        let order = [
            Some(RoundDesign::Amend),
            Some(RoundDesign::Full),
            Some(RoundDesign::Off),
        ];
        match key.code {
            KeyCode::Right | KeyCode::Char(' ') => {
                self.design = crate::run_goal::cycle(&order, self.design, true);
            }
            KeyCode::Left => self.design = crate::run_goal::cycle(&order, self.design, false),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                self.on_design = false;
            }
            KeyCode::Enter => return self.submit(),
            _ => {}
        }
        IterateOutcome::Stay
    }

    /// A bracketed paste into the text, through the editor (decision 5), the text drawn
    /// as `view` says. Nothing while submitting, on the confirm page or on the row.
    pub fn on_paste_in(&mut self, text: &str, view: EditorView) {
        if self.submitting || self.discarding || self.on_design {
            return;
        }
        self.text.on_editor_paste(text, view.width, view.rows);
    }

    fn submit(&mut self) -> IterateOutcome {
        match self.request() {
            Some(request) => {
                self.submitting = true;
                self.error = None;
                IterateOutcome::Submit(request)
            }
            None => {
                self.error = Some(EMPTY_REQUEST.into());
                IterateOutcome::Stay
            }
        }
    }

    /// What `anthrex run iterate <run> <text> [--design <mode>]` sends: the request
    /// trimmed, as the goal dialog trims its goal, and the design row's mode; `None`
    /// while it is blank.
    pub fn request(&self) -> Option<RunRequest> {
        let text = self.text.text().trim();
        (!text.is_empty()).then(|| RunRequest::Iterate {
            run: self.run_id.clone(),
            goal: text.to_owned(),
            design: self.design,
        })
    }
}

#[cfg(test)]
#[path = "run_iterate_tests.rs"]
mod tests;
