//! Milestone 9.6 decisions 34 and 35 (DF §6.1): the document gate screen's state and
//! keys. It opens on a run's brainstorm or spec gate (an Alerts row's Enter, the action
//! menu's `review document`); a design run's plan gate opens today's plan review
//! instead. The screen shows the gate's version (`ShowDoc`), and on `d`, `f` and `g` its
//! diff, its findings and the brainstorm's drafts; the daemon's replies are kept here
//! and drawn from here (`ui/doc_gate.rs`). Each gate key sends its `DocGate` only after
//! its confirm page (`a`, `x`, `r`, `b`) or its editor's save (`c`, `e`), and the
//! daemon's refusal is shown verbatim on the screen's message line. While the
//! orchestrator revises, only `x` and `q` act. Requests and replies are
//! `app/doc_gate_replies.rs`'s, the note editor's glue `app/doc_gate_note.rs`'s. Pure:
//! every request leaves as an `Effect` (`AGENTS.md` hard rule 5).

use super::screens::Screen;
use super::{App, Effect, Modal, PendingAction};
use crate::actions_request::short_id;
use crate::doc_note::{DocNoteForm, NoteFor, report_questions};
use crate::safe_text::one_line;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DocGateAction, DocGateInfo, DocGateKind, DocView, RunInfo, RunState};

#[path = "doc_gate_info.rs"]
mod info;
#[path = "plan_doc.rs"]
pub(crate) mod plan_doc;
pub use info::{
    alert_text, confirm_text, doc_gate_of, gate_doc, kind_title, round_drafts, round_of,
    waiting_gate,
};
pub use plan_doc::PlanDoc;

/// The other screens' refusal of `C-b a`, `C-b m` and `C-b t`, with this one's name.
pub const LEAVE_DOC_FIRST: &str = "leave the document first (esc)";
/// PgUp/PgDn move this many lines, as on the stats screen.
const PAGE: usize = 10;

/// The daemon's refusal while the orchestrator revises `kind` v`version` (brief
/// "Messages"), said before a request it would refuse.
pub(crate) fn revising_text(kind: DocGateKind, version: u32) -> String {
    format!(
        "the orchestrator is revising {} v{}; wait for it",
        kind.label(),
        version + 1
    )
}

/// One requested document, as the daemon answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocLoad {
    /// Waiting on the tagged `ShowDoc` with this id.
    Loading(u64),
    Ready(Box<DocView>),
    /// The daemon's refusal, verbatim, or why no reply will come.
    Failed(String),
}

impl DocLoad {
    pub fn ready(&self) -> Option<&DocView> {
        match self {
            DocLoad::Ready(doc) => Some(doc),
            _ => None,
        }
    }
}

/// What the screen's body shows: the document, or one of `d`, `f` and `g`'s views.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocPane {
    Document,
    Diff,
    Findings,
    Drafts,
}

/// One brainstormer's draft of the round, for `g`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftLoad {
    pub label: String,
    pub version: u32,
    pub load: DocLoad,
}

/// How the message line draws its text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// The daemon took the request.
    Done,
    /// The daemon refused it (its text verbatim), or a request failed.
    Refused,
    /// The screen's own word: a key that does not act now.
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocGateScreen {
    pub run_id: String,
    pub kind: DocGateKind,
    /// The gate version shown; the screen follows the gate to a newer one.
    pub version: u32,
    /// The version's text and its findings.
    pub doc: DocLoad,
    /// `d`'s diff against the previous version, asked once.
    pub diff: Option<DocLoad>,
    /// `g`'s drafts, asked once (the brainstorm gate only).
    pub drafts: Option<Vec<DraftLoad>>,
    pub pane: DocPane,
    /// The first line shown of the pane.
    pub scroll: usize,
    /// The message line.
    pub message: Option<(Tone, String)>,
}

impl App {
    pub(super) fn doc_screen(&self) -> Option<&DocGateScreen> {
        match &self.screen {
            Some(Screen::DocGate(s)) => Some(s),
            _ => None,
        }
    }

    pub(super) fn doc_screen_mut(&mut self) -> Option<&mut DocGateScreen> {
        match &mut self.screen {
            Some(Screen::DocGate(s)) => Some(s),
            _ => None,
        }
    }

    fn run_named(&self, run_id: &str) -> Option<&RunInfo> {
        self.runs.runs.iter().find(|r| r.run_id == run_id)
    }

    /// Decision 34: `review document` on `run_id`: the gate screen on its brainstorm or
    /// spec gate, loading the version; at a design run's plan gate, today's plan review.
    /// A run with no document waiting is told so.
    pub(crate) fn open_doc_gate(&mut self, run_id: &str) -> Vec<Effect> {
        let gate = self.run_named(run_id).and_then(|r| r.doc_gate.clone());
        let waiting = self
            .run_named(run_id)
            .is_some_and(|r| r.state == RunState::AwaitingApproval);
        match gate {
            Some(g) if waiting && g.kind == DocGateKind::Plan => self.review_from_run_view(run_id),
            Some(g) if waiting => {
                let mut screen = DocGateScreen {
                    run_id: run_id.to_owned(),
                    kind: g.kind,
                    version: g.version,
                    doc: DocLoad::Failed(String::new()),
                    diff: None,
                    drafts: None,
                    pane: DocPane::Document,
                    scroll: 0,
                    message: None,
                };
                let (id, effect) = self.show_doc(run_id, gate_doc(g.kind), g.version, false);
                screen.doc = DocLoad::Loading(id);
                self.set_screen(Some(Screen::DocGate(Box::new(screen))));
                vec![effect]
            }
            _ => {
                let id = short_id(&one_line(run_id)).to_owned();
                self.toast(format!("run {id} has no document waiting for review"));
                vec![]
            }
        }
    }

    /// While the orchestrator revises the screen's gate: the next version and the
    /// gate's `revising` note.
    pub(crate) fn doc_revising(&self) -> Option<&DocGateInfo> {
        let s = self.doc_screen()?;
        let gate = self.run_named(&s.run_id)?.doc_gate.as_ref()?;
        (gate.kind == s.kind && gate.revising.is_some()).then_some(gate)
    }

    /// Ruling T17-2: `the <kind> gate is closed` once the gate the screen shows has
    /// closed (approved or rejected elsewhere, the run gone) or the run waits at another
    /// gate. Every key but `q` and Esc then does nothing.
    pub(crate) fn doc_gate_closed(&self) -> Option<String> {
        let s = self.doc_screen()?;
        let gate = self.run_named(&s.run_id).and_then(|r| r.doc_gate.as_ref());
        let open = gate.is_some_and(|g| g.kind == s.kind);
        (!open).then(|| format!("the {} gate is closed", s.kind.label()))
    }

    /// Task M9.6.18: the run view's and the plan review's gate keys at a design gate
    /// (`on_gate_key`'s first try after a hold). At a brainstorm or spec gate `a` opens
    /// the gate screen, since a plain approve is refused there (ruling T1-O1), and `x`
    /// asks on the screen's reject page; `e` and `d` have no task to act on. At the plan
    /// gate `a` asks to approve the version shown (ruling T17-1); `x`, `e` and `d` keep
    /// 9.5's plan gate. While the orchestrator revises, the plan's `a` says so. `None`:
    /// not a design gate, so 9.5's keys apply.
    pub(super) fn on_design_gate_key(&mut self, run_id: &str, key: char) -> Option<Vec<Effect>> {
        let run = self.run_named(run_id)?;
        let gate = (run.doc_gate.as_ref()).filter(|_| run.state == RunState::AwaitingApproval)?;
        let (kind, version) = (gate.kind, gate.version);
        let action = match (kind, key) {
            (DocGateKind::Plan, 'a') if gate.revising.is_some() => {
                self.toast(revising_text(kind, version));
                return Some(vec![]);
            }
            (DocGateKind::Plan, 'a') => DocGateAction::Approve {
                version: Some(version),
            },
            (DocGateKind::Plan, _) => return None,
            (_, 'a') => return Some(self.open_doc_gate(run_id)),
            (_, 'x') => DocGateAction::Reject,
            (_, _) => {
                self.toast(format!(
                    "the {} gate has no tasks; a reviews it",
                    kind.label()
                ));
                return Some(vec![]);
            }
        };
        self.modal = Some(Modal::Confirm {
            message: confirm_text(run_id, round_of(self, run_id), kind, version, &action),
            action: PendingAction::DocGate {
                run_id: run_id.to_owned(),
                kind,
                action,
            },
        });
        Some(vec![])
    }

    /// A bare key while the screen is open (`KeyAction::Screen`).
    pub(super) fn on_doc_gate_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(s) = self.doc_screen() else {
            return vec![];
        };
        let (run_id, kind, version, pane) = (s.run_id.clone(), s.kind, s.version, s.pane);
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return vec![];
        }
        if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
            match pane {
                DocPane::Document => self.set_screen(None),
                _ => self.doc_pane(DocPane::Document),
            }
            return vec![];
        }
        if self.doc_gate_closed().is_some() {
            return vec![];
        }
        if let Some(effects) = self.doc_scroll(key.code) {
            return effects;
        }
        let KeyCode::Char(c) = key.code else {
            return vec![];
        };
        if !"acerbxdfg".contains(c) {
            return vec![];
        }
        // DF §6.1: while the orchestrator revises, only `x` and `q` work.
        if c != 'x'
            && let Some(gate) = self.doc_revising()
        {
            let next = gate.version + 1;
            self.doc_message(
                Tone::Note,
                format!("the orchestrator is revising v{next}; only x and q work now"),
            );
            return vec![];
        }
        let confirm = |app: &mut App, action: DocGateAction| {
            app.modal = Some(Modal::Confirm {
                message: confirm_text(&run_id, round_of(app, &run_id), kind, version, &action),
                action: PendingAction::DocGate {
                    run_id: run_id.clone(),
                    kind,
                    action,
                },
            });
            vec![]
        };
        match c {
            // Ruling T17-1: the approve names the version its page shows, once that
            // version's text has loaded (final fix wave FW-54), as `e` waits for it.
            'a' if self.doc_screen().and_then(|s| s.doc.ready()).is_none() => {
                self.doc_message(
                    Tone::Note,
                    "the document is not loaded; a approves it once it is",
                );
                vec![]
            }
            'a' => confirm(
                self,
                DocGateAction::Approve {
                    version: Some(version),
                },
            ),
            'x' => confirm(self, DocGateAction::Reject),
            'c' => {
                let text = match kind {
                    DocGateKind::Brainstorm => self
                        .doc_screen()
                        .and_then(|s| s.doc.ready())
                        .map(|d| report_questions(&d.text))
                        .unwrap_or_default(),
                    _ => String::new(),
                };
                self.open_note(NoteFor::Changes, &text)
            }
            'e' => match self.doc_screen().and_then(|s| s.doc.ready()) {
                Some(doc) => {
                    let text = doc.text.clone();
                    self.open_note(NoteFor::Edit, &text)
                }
                None => {
                    self.doc_message(
                        Tone::Note,
                        "the document is not loaded; e edits it once it is",
                    );
                    vec![]
                }
            },
            'r' if kind != DocGateKind::Brainstorm => {
                self.doc_message(Tone::Note, "rethink is only for the brainstorm gate");
                vec![]
            }
            'r' => self.open_note(NoteFor::Rethink, ""),
            'b' if kind == DocGateKind::Brainstorm => {
                self.doc_message(Tone::Note, "back is only for the spec and plan gates");
                vec![]
            }
            'b' => self.open_note(NoteFor::Back, ""),
            'd' => {
                self.doc_pane(DocPane::Diff);
                self.ask_diff()
            }
            'f' => {
                self.doc_pane(DocPane::Findings);
                vec![]
            }
            'g' if kind != DocGateKind::Brainstorm => {
                self.doc_message(Tone::Note, "the drafts are the brainstorm gate's");
                vec![]
            }
            'g' => {
                self.doc_pane(DocPane::Drafts);
                self.ask_drafts()
            }
            _ => vec![],
        }
    }

    /// `d`, `f` and `g` toggle their view: pressed again, or Esc, the document again.
    fn doc_pane(&mut self, pane: DocPane) {
        if let Some(s) = self.doc_screen_mut() {
            s.pane = if s.pane == pane {
                DocPane::Document
            } else {
                pane
            };
            s.scroll = 0;
        }
    }

    /// `j`/`k`/Down/Up a line, PgDn/PgUp a page, stopping where the view stops
    /// (`ui::doc_gate::max_scroll` over the body the last frame drew). `None` for any
    /// other key.
    fn doc_scroll(&mut self, code: KeyCode) -> Option<Vec<Effect>> {
        let step: isize = match code {
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            KeyCode::PageDown => PAGE as isize,
            KeyCode::PageUp => -(PAGE as isize),
            _ => return None,
        };
        let last = self
            .doc_screen()
            .map(|s| crate::ui::doc_gate::max_scroll(self, s, self.body_area))?;
        let s = self.doc_screen_mut()?;
        let from = s.scroll.min(last);
        s.scroll = from.saturating_add_signed(step).min(last);
        Some(vec![])
    }

    /// The note editor for `purpose`, opened over the screen with `text`.
    fn open_note(&mut self, purpose: NoteFor, text: &str) -> Vec<Effect> {
        let Some(s) = self.doc_screen() else {
            return vec![];
        };
        let form = DocNoteForm::new(s.run_id.clone(), (s.kind, s.version), purpose, text);
        self.modal = Some(Modal::DocNote(Box::new(form)));
        vec![]
    }
}

#[cfg(test)]
#[path = "doc_gate_tests.rs"]
mod tests;
