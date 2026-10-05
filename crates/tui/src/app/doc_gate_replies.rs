//! Milestone 9.6 (DF §6.1, §10): the document gate screen's requests and their replies.
//! Every request is tagged (milestone 9 decision 2): `ShowDoc` for the version, its
//! diff and the round's drafts, whose `Doc` or `Refused` fills the part that asked by its
//! id; and the gate's `DocGate`, whose `Done` closes the screen with a toast for
//! approve, reject, rethink and back (the gate has moved on), or shows on the message
//! line for changes and edit, and whose `Refused` is shown verbatim on the message line
//! (and in the note editor while it waits). On its tick the screen follows its gate to
//! a newer version, and a request no longer awaited stops loading. Pure: every request
//! leaves as an `Effect`.

use super::doc_gate::{DocLoad, DraftLoad, Tone, gate_doc, round_drafts};
use super::replies::{NO_REPLY, NOT_CONNECTED, NOT_SENT, PendingWhat, reply_timeout};
use super::runs::first_line_and_more;
use super::screens::Screen;
use super::{App, Effect, Modal, ToastLevel};
use proto::{DocGateAction, DocGateKind, DocKind, RunReply, RunRequest};

/// What a tagged `DocGate` of ours was, for its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateAsk {
    pub run_id: String,
    /// Approve, reject, rethink and back move the gate on: their `Done` closes the
    /// screen.
    pub closes: bool,
}

impl App {
    /// One tagged `ShowDoc` of `run_id`'s `kind` v`version`, with its findings (the
    /// Review panel's disputed answers, `f`) or its diff.
    pub(super) fn show_doc(
        &mut self,
        run_id: &str,
        kind: DocKind,
        version: u32,
        diff: bool,
    ) -> (u64, Effect) {
        let request = RunRequest::ShowDoc {
            run: run_id.to_owned(),
            kind,
            version: Some(version),
            diff,
            findings: !diff && kind != DocKind::BrainstormDraft,
        };
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        let what = PendingWhat::DocShow {
            run_id: run_id.to_owned(),
        };
        self.replies.insert(id, what, timeout);
        (id, effect)
    }

    /// `d`: the version's diff, asked once.
    pub(super) fn ask_diff(&mut self) -> Vec<Effect> {
        let Some(s) = self.doc_screen_mut() else {
            return vec![];
        };
        if s.diff.is_some() {
            return vec![];
        }
        let (run_id, kind, version) = (s.run_id.clone(), gate_doc(s.kind), s.version);
        let (id, effect) = self.show_doc(&run_id, kind, version, true);
        if let Some(s) = self.doc_screen_mut() {
            s.diff = Some(DocLoad::Loading(id));
        }
        vec![effect]
    }

    /// `g`: each of the round's drafts (`round_drafts`), asked once.
    pub(super) fn ask_drafts(&mut self) -> Vec<Effect> {
        let Some(s) = self.doc_screen_mut() else {
            return vec![];
        };
        if s.drafts.is_some() {
            return vec![];
        }
        let (run_id, version) = (s.run_id.clone(), s.version);
        let listed = self
            .runs
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .map(|r| round_drafts(r, version))
            .unwrap_or_default();
        let mut drafts = Vec::new();
        let mut effects = Vec::new();
        for (label, n) in listed {
            let (id, effect) = self.show_doc(&run_id, DocKind::BrainstormDraft, n, false);
            effects.push(effect);
            drafts.push(DraftLoad {
                label,
                version: n,
                load: DocLoad::Loading(id),
            });
        }
        if let Some(s) = self.doc_screen_mut() {
            s.drafts = Some(drafts);
        }
        effects
    }

    /// A confirmed or saved gate action: one tagged `DocGate`, its reply routed here.
    pub(super) fn send_doc_gate(
        &mut self,
        run_id: String,
        kind: DocGateKind,
        action: DocGateAction,
    ) -> (u64, Effect) {
        let closes = matches!(
            action,
            DocGateAction::Approve { .. }
                | DocGateAction::Reject
                | DocGateAction::Rethink { .. }
                | DocGateAction::Back { .. }
        );
        let ask = GateAsk {
            run_id: run_id.clone(),
            closes,
        };
        let request = RunRequest::DocGate {
            run: run_id,
            kind,
            action,
        };
        let timeout = reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        self.replies.insert(id, PendingWhat::DocGate(ask), timeout);
        (id, effect)
    }

    /// `route_reply`'s document arm: a reply to a `ShowDoc` or a `DocGate` of ours, by
    /// id; `None` for any other.
    pub(super) fn route_doc_gate_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let id = reply.request_id()?;
        match self.replies.peek(id)? {
            PendingWhat::DocShow { .. } => {
                self.replies.take(Some(id));
                let load = match reply {
                    RunReply::Doc { doc, .. } => DocLoad::Ready(doc.clone()),
                    RunReply::Refused { message, .. } => DocLoad::Failed(message.clone()),
                    _ => DocLoad::Failed(format!("{reply:?}")),
                };
                self.fill_doc(id, load);
                Some(vec![])
            }
            PendingWhat::DocGate(ask) => {
                let ask = ask.clone();
                self.replies.take(Some(id));
                self.gate_replied(id, &ask, reply);
                Some(vec![])
            }
            _ => None,
        }
    }

    /// The part of the open screen waiting on `id` takes `load`.
    fn fill_doc(&mut self, id: u64, load: DocLoad) {
        let Some(s) = self.doc_screen_mut() else {
            return;
        };
        let waits = |l: &DocLoad| *l == DocLoad::Loading(id);
        if waits(&s.doc) {
            s.doc = load;
        } else if s.diff.as_ref().is_some_and(waits) {
            s.diff = Some(load);
        } else if let Some(d) = (s.drafts.iter_mut().flatten()).find(|d| waits(&d.load)) {
            d.load = load;
        }
    }

    fn gate_replied(&mut self, id: u64, ask: &GateAsk, reply: &RunReply) {
        let note = matches!(&self.modal, Some(Modal::DocNote(f)) if f.request_id == Some(id));
        let screen = matches!(&self.screen, Some(Screen::DocGate(s)) if s.run_id == ask.run_id);
        match reply {
            RunReply::Refused { message, .. } => {
                if note && let Some(Modal::DocNote(form)) = &mut self.modal {
                    let text = first_line_and_more(message).unwrap_or_else(|| message.clone());
                    form.error = Some(text);
                    form.submitting = false;
                    form.request_id = None;
                }
                if screen {
                    self.doc_message(Tone::Refused, message.clone());
                } else if !note {
                    let text = first_line_and_more(message).unwrap_or_else(|| message.clone());
                    self.toast_at(ToastLevel::Error, text);
                }
            }
            RunReply::Done { message, .. } => {
                if note {
                    self.modal = None;
                }
                if screen && ask.closes {
                    self.set_screen(None);
                    self.toast(super::runs::capped(message));
                } else if screen {
                    self.doc_message(Tone::Done, message.clone());
                } else {
                    self.toast(super::runs::capped(message));
                }
            }
            _ => {}
        }
    }

    /// The message line says `text`.
    pub(super) fn doc_message(&mut self, tone: Tone, text: impl Into<String>) {
        if let Some(s) = self.doc_screen_mut() {
            s.message = Some((tone, text.into()));
        }
    }

    /// `screens_tick`: the screen follows its gate to a newer version (the orchestrator's
    /// revision, the user's edit), loading it afresh; and a request no longer awaited
    /// (expired, the link gone) stops loading and says why.
    pub(super) fn doc_gate_tick(&mut self) -> Vec<Effect> {
        let Some(s) = self.doc_screen() else {
            return vec![];
        };
        let (run_id, kind, version) = (s.run_id.clone(), s.kind, s.version);
        let newer = self
            .runs
            .runs
            .iter()
            .find(|r| r.run_id == run_id)
            .and_then(|r| r.doc_gate.as_ref())
            .filter(|g| g.kind == kind && g.version != version)
            .map(|g| g.version);
        let mut effects = Vec::new();
        if let Some(newer) = newer {
            let (id, effect) = self.show_doc(&run_id, gate_doc(kind), newer, false);
            effects.push(effect);
            if let Some(s) = self.doc_screen_mut() {
                s.version = newer;
                s.doc = DocLoad::Loading(id);
                s.diff = None;
                s.drafts = None;
                s.scroll = 0;
            }
        }
        let why = if self.connected() {
            NO_REPLY
        } else {
            NOT_CONNECTED
        };
        self.doc_give_up(None, why);
        effects
    }

    /// `expire_replies`' share (review m6): the part loading on the expired `ShowDoc`
    /// `id` says `why`; `true` when one did, so no toast repeats it.
    pub(super) fn doc_show_failed(&mut self, id: u64, why: &str) -> bool {
        let Some(s) = self.doc_screen() else {
            return false;
        };
        let waits = std::iter::once(&s.doc)
            .chain(s.diff.as_ref())
            .chain(s.drafts.iter().flatten().map(|d| &d.load))
            .any(|l| *l == DocLoad::Loading(id));
        if waits {
            self.doc_give_up(Some(id), why);
        }
        waits
    }

    /// Every part loading on a request no longer awaited (on `id`, when given) fails
    /// with `why`.
    fn doc_give_up(&mut self, id: Option<u64>, why: &str) {
        let replies = self.replies.clone();
        let Some(s) = self.doc_screen_mut() else {
            return;
        };
        let stale = |l: &DocLoad| match l {
            DocLoad::Loading(on) => !replies.contains(*on) && id.is_none_or(|id| id == *on),
            _ => false,
        };
        let parts = std::iter::once(&mut s.doc)
            .chain(s.diff.as_mut())
            .chain(s.drafts.iter_mut().flatten().map(|d| &mut d.load));
        for part in parts {
            if stale(part) {
                *part = DocLoad::Failed(why.to_owned());
            }
        }
    }

    /// `on_send_failed`: a `ShowDoc` or `DocGate` of ours never left. The part loading
    /// on it, the note editor waiting on it and the message line say so; `true` when
    /// the usual not-sent toast is the only feedback.
    pub(super) fn doc_gate_not_sent(&mut self, id: u64) -> bool {
        let why = if self.connected() {
            NOT_SENT
        } else {
            NOT_CONNECTED
        };
        let mut owned = false;
        if let Some(Modal::DocNote(form)) = &mut self.modal
            && form.request_id == Some(id)
        {
            form.submitting = false;
            form.request_id = None;
            form.error = Some(crate::doc_note::NOT_SENT.into());
            owned = true;
        }
        if self.doc_screen().is_some() {
            self.doc_give_up(Some(id), why);
            if !owned {
                self.doc_message(Tone::Refused, why.to_owned());
            }
            owned = true;
        }
        !owned
    }
}
