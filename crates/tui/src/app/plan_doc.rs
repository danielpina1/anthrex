//! Milestone 9.6 decisions 34 and 35 (DF §6.1): the plan review's Plan doc tab. At a
//! design run's plan gate, Tab switches the review between its task list and the
//! engine's `plan.md` (one tagged `ShowDoc` of the gate's version, kept while that
//! version stands), whose coverage table marks a requirement no task covers
//! (`ui/plan_doc.rs`). `c` (changes) and `b` (back) take their note in the gate screen's
//! editor (`crate::doc_note`) and send the gate screen's `DocGate`: changes on the
//! editor's Ctrl-S, back after its confirm page (`app/doc_gate_note.rs`). The plan is
//! never edited as text here (ruling T11-3, m3): its tasks keep the review's `e` and
//! `d`. While the tab shows, the scroll keys move the document; every other key is the
//! review's. The tab follows the gate to a newer version on the tick, and a load no
//! longer awaited says why. A run without the flow has no tab. Pure: every request
//! leaves as an `Effect` (`AGENTS.md` hard rule 5).

use super::{DocLoad, revising_text};
use crate::app::plan_review::ReviewTarget;
use crate::app::replies::{NO_REPLY, NOT_CONNECTED};
use crate::app::{App, Effect, Modal};
use crate::doc_note::{DocNoteForm, NoteFor};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DocGateInfo, DocGateKind, DocKind, RunState};

/// PgUp/PgDn move this many lines, as on the gate screen.
const PAGE: usize = 10;

/// The Plan doc tab of a design run's plan review.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanDoc {
    /// The gate version the tab loaded.
    pub version: u32,
    pub load: DocLoad,
    /// The first line shown.
    pub scroll: usize,
    /// The tab shows, not the task list.
    pub shown: bool,
}

/// Review m2: `e` and `d` on the Plan doc tab, which hides the tasks they act on.
pub(crate) const TASKS_TAB_TEXT: &str = "e and d act on the tasks: tab shows them";

impl App {
    /// The reviewed run's id and its design plan gate, while the review is that gate's.
    pub(crate) fn plan_doc_gate(&self) -> Option<(&str, &DocGateInfo)> {
        let review = (self.plan_review.as_ref()).filter(|r| r.target == ReviewTarget::Gate)?;
        let run = self.runs.runs.iter().find(|r| r.run_id == review.run_id)?;
        let gate = (run.doc_gate.as_ref())
            .filter(|g| g.kind == DocGateKind::Plan && run.state == RunState::AwaitingApproval)?;
        Some((run.run_id.as_str(), gate))
    }

    /// The tab, while it shows.
    pub(crate) fn plan_doc(&self) -> Option<&PlanDoc> {
        let doc = self.plan_review.as_ref()?.doc.as_ref()?;
        doc.shown.then_some(doc)
    }

    /// The plan review's keys at a design run's plan gate, tried before its own: Tab,
    /// `c` and `b`, and while the tab shows the scroll keys and `e`/`d`'s refusal.
    /// `None`: not one of them, or no design plan gate, so the review's own keys apply.
    pub(crate) fn on_plan_doc_key(&mut self, key: KeyEvent) -> Option<Vec<Effect>> {
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        let (run_id, gate) = self.plan_doc_gate()?;
        let (run_id, version, revising) =
            (run_id.to_owned(), gate.version, gate.revising.is_some());
        match key.code {
            KeyCode::Tab => Some(self.toggle_plan_doc(&run_id, version)),
            KeyCode::Char(c @ ('c' | 'b')) => {
                if revising {
                    self.toast(revising_text(DocGateKind::Plan, version));
                    return Some(vec![]);
                }
                let purpose = if c == 'c' {
                    NoteFor::Changes
                } else {
                    NoteFor::Back
                };
                let form = DocNoteForm::new(run_id, (DocGateKind::Plan, version), purpose, "");
                self.modal = Some(Modal::DocNote(Box::new(form)));
                Some(vec![])
            }
            // Review m2: the tab hides the tasks `e` and `d` would act on.
            KeyCode::Char('e' | 'd') if self.plan_doc().is_some() => {
                self.toast(TASKS_TAB_TEXT);
                Some(vec![])
            }
            code if self.plan_doc().is_some() => self.scroll_plan_doc(code),
            _ => None,
        }
    }

    /// Tab: the list again, or the document: kept while its version stands and it did
    /// not fail, else asked afresh.
    fn toggle_plan_doc(&mut self, run_id: &str, version: u32) -> Vec<Effect> {
        let doc = self.plan_review.as_mut().and_then(|r| r.doc.as_mut());
        match doc {
            Some(doc) if doc.shown => {
                doc.shown = false;
                return vec![];
            }
            Some(doc) if doc.version == version && !matches!(doc.load, DocLoad::Failed(_)) => {
                doc.shown = true;
                return vec![];
            }
            _ => {}
        }
        let (id, effect) = self.show_doc(run_id, DocKind::Plan, version, false);
        if let Some(review) = self.plan_review.as_mut() {
            review.doc = Some(PlanDoc {
                version,
                load: DocLoad::Loading(id),
                scroll: 0,
                shown: true,
            });
        }
        vec![effect]
    }

    /// `j`/`k`/Down/Up a line, PgDn/PgUp a page, stopping where the document stops
    /// (`ui::plan_review::plan_doc::max_scroll` at the last frame's body). `None` for
    /// any other key.
    fn scroll_plan_doc(&mut self, code: KeyCode) -> Option<Vec<Effect>> {
        let step: isize = match code {
            KeyCode::Char('j') | KeyCode::Down => 1,
            KeyCode::Char('k') | KeyCode::Up => -1,
            KeyCode::PageDown => PAGE as isize,
            KeyCode::PageUp => -(PAGE as isize),
            _ => return None,
        };
        let last = crate::ui::plan_review::plan_doc::max_scroll(self, self.body_area);
        let doc = self.plan_review.as_mut()?.doc.as_mut()?;
        doc.scroll = doc.scroll.min(last).saturating_add_signed(step).min(last);
        Some(vec![])
    }

    /// `screens_tick` with no screen open: the tab follows its gate to a newer version
    /// (asked afresh while it shows, else on the next Tab), and a load no longer awaited
    /// (the link gone) says why.
    pub(crate) fn plan_doc_tick(&mut self) -> Vec<Effect> {
        let newer = self
            .plan_doc_gate()
            .map(|(run_id, gate)| (run_id.to_owned(), gate.version));
        let Some(doc) = self.plan_review.as_ref().and_then(|r| r.doc.as_ref()) else {
            return vec![];
        };
        let (shown, version, loading) = (doc.shown, doc.version, doc.load.clone());
        let mut effects = Vec::new();
        match newer {
            Some((run_id, newer)) if newer != version => {
                if let Some(review) = self.plan_review.as_mut() {
                    review.doc = None;
                }
                if shown {
                    effects = self.toggle_plan_doc(&run_id, newer);
                }
                return effects;
            }
            _ => {}
        }
        if let DocLoad::Loading(id) = loading
            && !self.replies.contains(id)
        {
            let why = if self.connected() {
                NO_REPLY
            } else {
                NOT_CONNECTED
            };
            self.plan_doc_failed(id, why);
        }
        effects
    }

    /// The tab waiting on `ShowDoc` `id` takes `load`; `true` when it did.
    pub(crate) fn fill_plan_doc(&mut self, id: u64, load: &DocLoad) -> bool {
        let doc = self.plan_review.as_mut().and_then(|r| r.doc.as_mut());
        match doc {
            Some(doc) if doc.load == DocLoad::Loading(id) => {
                doc.load = load.clone();
                true
            }
            _ => false,
        }
    }

    /// The tab waiting on `ShowDoc` `id` shows `why` instead; `true` when it did, so no
    /// toast repeats it.
    pub(crate) fn plan_doc_failed(&mut self, id: u64, why: &str) -> bool {
        self.fill_plan_doc(id, &DocLoad::Failed(why.to_owned()))
    }
}

#[cfg(test)]
#[path = "plan_doc_tests.rs"]
pub(crate) mod tests;
