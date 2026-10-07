//! Milestone 9.0.6 decision 15: the action menu's input forms — answer, message,
//! override, resume and promote. Each form owns its fields, takes its keys through
//! `on_key` and yields the `ActionInput` the request is built from; Enter goes to the
//! confirmation page (decision 14) and Esc back. Pure (`AGENTS.md` hard rule 5): the one
//! request a form sends, the answer's `TaskDetail`, is an `Effect`.

use super::{ActionFlow, ActionStep, ConfirmPage};
use crate::actions_request::{ActionInput, ActionTarget};
use crate::app::replies::{NOT_CONNECTED, PendingWhat, REPLY_TIMEOUT};
use crate::app::screens::SettingsCache;
use crate::app::{App, Effect, Modal};
use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crate::theme::runtime_tag;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    ActionInfo, BaseMovedInfo, BlockReason, InputKind, MessageKind, OrchestratorChoice, RunInfo,
    RunReply, RunRequest,
};

/// What a form's key did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormOutcome {
    Stay,
    /// Enter with a valid input: on to the confirmation page.
    Page,
    /// Esc: back to the menu.
    Back,
}

/// The answer form's brief rows: one tagged `TaskDetail` fills them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Brief {
    Loading,
    /// The brief's first three lines.
    Ready(Vec<String>),
    /// The refusal's first line.
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnswerForm {
    pub info: ActionInfo,
    pub task_id: String,
    /// `<id> <title>`.
    pub task: String,
    pub question: String,
    pub brief: Brief,
    /// The brief's last `TaskDetail`: its reply fills a brief not yet `Ready`, even late.
    pub brief_id: Option<u64>,
    pub text: TextArea,
    /// Enter was pressed on a blank answer: `type an answer first` shows.
    pub blank: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageForm {
    pub info: ActionInfo,
    /// Who it goes to: the task id or `stage <n>`.
    pub to: String,
    pub kind: MessageKind,
    pub text: TextArea,
    /// Enter was pressed on a blank message: `type a message first` shows.
    pub blank: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideForm {
    pub info: ActionInfo,
    pub reason: TextArea,
    /// Enter was pressed on a blank reason: `type a reason first` shows.
    pub blank: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeForm {
    pub info: ActionInfo,
    pub rebaseline: bool,
    pub moved: Option<BaseMovedInfo>,
}

/// One entry of the promote picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteOption {
    pub label: String,
    pub choice: Option<OrchestratorChoice>,
}

impl PromoteOption {
    pub fn configured() -> Self {
        Self {
            label: "configured".into(),
            choice: None,
        }
    }

    /// `<tag> <model>` for an enabled roster entry (an empty model is `default`).
    pub fn roster(runtime: proto::Runtime, model: &str) -> Self {
        let shown = if model.is_empty() { "default" } else { model };
        Self {
            label: format!("{} {shown}", runtime_tag(runtime)),
            choice: Some(OrchestratorChoice {
                runtime,
                model: (!model.is_empty()).then(|| model.to_string()),
                effort: None,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromoteForm {
    pub info: ActionInfo,
    pub options: Vec<PromoteOption>,
    pub at: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionForm {
    Answer(AnswerForm),
    Message(MessageForm),
    Override(OverrideForm),
    Resume(ResumeForm),
    Promote(PromoteForm),
}

/// The word a message kind is shown and sent as.
pub fn kind_word(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::Info => "info",
        MessageKind::Change => "change",
        MessageKind::StopAndWait => "stop_and_wait",
    }
}

fn cycle_kind(kind: MessageKind, back: bool) -> MessageKind {
    const ALL: [MessageKind; 3] = [
        MessageKind::Info,
        MessageKind::Change,
        MessageKind::StopAndWait,
    ];
    let at = ALL.iter().position(|k| *k == kind).unwrap_or(0);
    ALL[(at + if back { ALL.len() - 1 } else { 1 }) % ALL.len()]
}

fn is_ctrl_j(key: &KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'j'))
}

/// The text a form sends: trimmed, control characters already dropped by the area.
fn sent(area: &TextArea) -> String {
    area.text().trim().to_string()
}

impl ActionForm {
    pub fn info(&self) -> &ActionInfo {
        match self {
            ActionForm::Answer(f) => &f.info,
            ActionForm::Message(f) => &f.info,
            ActionForm::Override(f) => &f.info,
            ActionForm::Resume(f) => &f.info,
            ActionForm::Promote(f) => &f.info,
        }
    }

    pub fn set_info(&mut self, info: ActionInfo) {
        match self {
            ActionForm::Answer(f) => f.info = info,
            ActionForm::Message(f) => f.info = info,
            ActionForm::Override(f) => f.info = info,
            ActionForm::Resume(f) => f.info = info,
            ActionForm::Promote(f) => f.info = info,
        }
    }

    #[cfg(test)]
    pub fn on_key(&mut self, key: KeyEvent) -> FormOutcome {
        self.on_key_in(key, 0)
    }

    /// The form's own keys; Enter, Esc and what the form does not use are decided here.
    /// Up and Down move a row of its text area drawn `width` wide (decision 35).
    pub fn on_key_in(&mut self, key: KeyEvent, width: u16) -> FormOutcome {
        match key.code {
            KeyCode::Esc => return FormOutcome::Back,
            KeyCode::Enter => {
                // A blank text says why on the form (final review I2), as a blank
                // reason always did.
                let ready = self.input().is_ok();
                match self {
                    ActionForm::Answer(f) => f.blank = !ready,
                    ActionForm::Message(f) => f.blank = !ready,
                    ActionForm::Override(f) => f.blank = !ready,
                    ActionForm::Resume(_) | ActionForm::Promote(_) => {}
                }
                return if ready {
                    FormOutcome::Page
                } else {
                    FormOutcome::Stay
                };
            }
            _ => {}
        }
        let back = key.code == KeyCode::BackTab
            || (key.code == KeyCode::Tab && key.modifiers.contains(KeyModifiers::SHIFT));
        let forward = key.code == KeyCode::Tab && !back;
        match self {
            ActionForm::Answer(f) => {
                if f.text.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Message(f) => {
                if forward || back {
                    f.kind = cycle_kind(f.kind, back);
                } else if f.text.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Override(f) => {
                // One line: a newline has no place in a reason.
                if !is_ctrl_j(&key) && f.reason.on_key_in(key, width) {
                    f.blank = false;
                }
            }
            ActionForm::Resume(f) => {
                if matches!(
                    key.code,
                    KeyCode::Tab
                        | KeyCode::BackTab
                        | KeyCode::Left
                        | KeyCode::Right
                        | KeyCode::Char(' ')
                ) {
                    f.rebaseline = !f.rebaseline;
                }
            }
            ActionForm::Promote(f) => {
                let n = f.options.len().max(1);
                match key.code {
                    KeyCode::Right | KeyCode::Tab if !back => f.at = (f.at + 1) % n,
                    KeyCode::Left | KeyCode::BackTab | KeyCode::Tab => f.at = (f.at + n - 1) % n,
                    _ => {}
                }
            }
        }
        FormOutcome::Stay
    }

    pub fn on_paste(&mut self, text: &str) {
        match self {
            ActionForm::Answer(f) => {
                f.text.on_paste(text);
                f.blank = false;
            }
            ActionForm::Message(f) => {
                f.text.on_paste(text);
                f.blank = false;
            }
            ActionForm::Override(f) => {
                // A reason is one line: a paste's breaks become spaces.
                f.reason.on_paste(&text.replace(['\r', '\n'], " "));
                f.blank = false;
            }
            ActionForm::Resume(_) | ActionForm::Promote(_) => {}
        }
    }

    /// What the request is built from, or why the form is not ready.
    pub fn input(&self) -> Result<ActionInput, &'static str> {
        match self {
            ActionForm::Answer(f) => match sent(&f.text) {
                t if t.is_empty() => Err("type an answer first"),
                text => Ok(ActionInput::Answer(text)),
            },
            ActionForm::Message(f) => match sent(&f.text) {
                t if t.is_empty() => Err("type a message first"),
                text => Ok(ActionInput::Message { kind: f.kind, text }),
            },
            ActionForm::Override(f) => match sent(&f.reason) {
                t if t.is_empty() => Err("type a reason first"),
                reason => Ok(ActionInput::Reason(reason)),
            },
            ActionForm::Resume(f) => Ok(ActionInput::Resume {
                rebaseline: f.rebaseline,
            }),
            ActionForm::Promote(f) => Ok(ActionInput::Promote(
                f.options.get(f.at).and_then(|o| o.choice.clone()),
            )),
        }
    }

    /// The confirmation page's rows for what was typed or picked.
    pub fn details(&self, ascii: bool) -> Vec<(String, String)> {
        let ellipsis = if ascii { "..." } else { "…" };
        let preview =
            |area: &TextArea| crate::ui::kit::cut(&one_line(area.text().trim()), 160, ellipsis);
        let row = |l: &str, v: String| (l.to_string(), v);
        match self {
            ActionForm::Answer(f) => {
                vec![row("task", f.task.clone()), row("answer", preview(&f.text))]
            }
            ActionForm::Message(f) => vec![
                row("to", f.to.clone()),
                row("kind", kind_word(f.kind).to_string()),
                row("message", preview(&f.text)),
            ],
            ActionForm::Override(f) => vec![row("reason", preview(&f.reason))],
            ActionForm::Resume(f) => vec![row(
                "rebaseline",
                if f.rebaseline { "on" } else { "off" }.to_string(),
            )],
            ActionForm::Promote(f) => vec![row(
                "orchestrator",
                f.options
                    .get(f.at)
                    .map(|o| o.label.clone())
                    .unwrap_or_default(),
            )],
        }
    }
}

/// The picker's entries: `configured`, then every enabled roster entry of the settings
/// cache (decision 24); without a cache only `configured` is offered.
fn promote_options(cache: Option<&SettingsCache>) -> Vec<PromoteOption> {
    let roster = cache.into_iter().flat_map(|c| c.models());
    std::iter::once(PromoteOption::configured())
        .chain(roster.map(|m| PromoteOption::roster(m.runtime, &m.model)))
        .collect()
}

/// The form for `info` on `target` of `run`, or `None` when the node cannot give it.
fn build_form(
    run: &RunInfo,
    target: &ActionTarget,
    info: &ActionInfo,
    kind: InputKind,
    cache: Option<&SettingsCache>,
) -> Option<ActionForm> {
    let info = info.clone();
    Some(match (kind, target) {
        (InputKind::Answer, ActionTarget::Task(id)) => {
            let task = run.tasks.iter().find(|t| t.id == *id)?;
            let question = match &task.block {
                Some(b) if b.reason == BlockReason::Question => b.text.clone(),
                _ => String::new(),
            };
            ActionForm::Answer(AnswerForm {
                info,
                task_id: id.clone(),
                task: format!("{id} {}", task.title),
                question,
                brief: Brief::Loading,
                brief_id: None,
                text: TextArea::new(),
                blank: false,
            })
        }
        (InputKind::Message, ActionTarget::Task(id)) => ActionForm::Message(MessageForm {
            info,
            to: id.clone(),
            kind: MessageKind::Info,
            text: TextArea::new(),
            blank: false,
        }),
        (InputKind::Message, ActionTarget::Stage(n)) => ActionForm::Message(MessageForm {
            info,
            to: format!("stage {n}"),
            kind: MessageKind::Info,
            text: TextArea::new(),
            blank: false,
        }),
        (InputKind::Reason, ActionTarget::Task(_)) => ActionForm::Override(OverrideForm {
            info,
            reason: TextArea::new(),
            blank: false,
        }),
        (InputKind::Resume, ActionTarget::Run) => ActionForm::Resume(ResumeForm {
            info,
            // Decision 15 (preflight F31): on only when the base moved.
            rebaseline: run.base_moved.is_some(),
            moved: run.base_moved.clone(),
        }),
        (InputKind::Promote, ActionTarget::Run) => ActionForm::Promote(PromoteForm {
            info,
            options: promote_options(cache),
            at: 0,
        }),
        _ => return None,
    })
}

impl App {
    /// A menu entry that needs input: opens its form on `flow`. The answer form asks
    /// for its task's brief with one tagged `TaskDetail` (decision 15).
    pub(in crate::app) fn open_form(
        &mut self,
        flow: &mut ActionFlow,
        info: ActionInfo,
        kind: InputKind,
    ) -> Vec<Effect> {
        let cache = self.settings_cache.as_ref();
        let form = self
            .runs
            .runs
            .iter()
            .find(|r| r.run_id == flow.run_id)
            .and_then(|run| build_form(run, &flow.target, &info, kind, cache));
        let Some(mut form) = form else {
            return vec![];
        };
        let mut effects = vec![];
        if let ActionForm::Answer(f) = &mut form {
            if self.connected() {
                let (id, effect) = self.brief_request(&flow.run_id, &f.task_id);
                f.brief_id = Some(id);
                effects.push(effect);
            } else {
                // Decision 37: say why at once; `form_brief_reconnected` asks again.
                f.brief = Brief::Failed(NOT_CONNECTED.into());
            }
        }
        flow.step = ActionStep::Form(Box::new(form));
        effects
    }

    /// The answer form's one tagged `TaskDetail` for `task_id` of `run_id`, recorded.
    fn brief_request(&mut self, run_id: &str, task_id: &str) -> (u64, Effect) {
        let request = RunRequest::TaskDetail {
            run_id: run_id.into(),
            task_id: task_id.into(),
        };
        let (id, effect) = self.tagged_request(request);
        let what = PendingWhat::FormBrief {
            run_id: run_id.into(),
            task_id: task_id.into(),
        };
        self.replies.insert(id, what, REPLY_TIMEOUT);
        (id, effect)
    }

    /// The open answer form, whether it is being edited or its page shows.
    fn open_answer_mut(&mut self) -> Option<&mut AnswerForm> {
        let Some(Modal::Action(flow)) = self.modal.as_mut() else {
            return None;
        };
        let form = match &mut flow.step {
            ActionStep::Form(form) => &mut **form,
            ActionStep::Confirm(ConfirmPage {
                form: Some(form), ..
            }) => &mut **form,
            _ => return None,
        };
        match form {
            ActionForm::Answer(f) => Some(f),
            _ => None,
        }
    }

    /// No reply will come for the brief request `id`, of `task_id` of `run_id` (final
    /// review I2): an open form still loading it says `why`. `true` when the form was
    /// open on that task; only the form's own request fails it (9.0.7 decision 37), so
    /// an earlier opening's expiry leaves a newer request loading.
    pub(in crate::app) fn fail_form_brief(
        &mut self,
        id: u64,
        run_id: &str,
        task_id: &str,
        why: &str,
    ) -> bool {
        let Some(f) = self.open_form_mut(run_id, task_id) else {
            return false;
        };
        if f.brief == Brief::Loading && f.brief_id == Some(id) {
            f.brief = Brief::Failed(why.into());
        }
        true
    }

    /// A lost link took a loading brief's reply with it.
    pub(in crate::app) fn form_brief_link_lost(&mut self) {
        if let Some(f) = self.open_answer_mut().filter(|f| f.brief == Brief::Loading) {
            f.brief = Brief::Failed(NOT_CONNECTED.into());
        }
    }

    /// A new connection: an open answer form whose brief did not come asks again, once.
    pub(in crate::app) fn form_brief_reconnected(&mut self) -> Vec<Effect> {
        let run_id = match &self.modal {
            Some(Modal::Action(flow)) => flow.run_id.clone(),
            _ => return vec![],
        };
        let Some(task_id) = self
            .open_answer_mut()
            .filter(|f| !matches!(f.brief, Brief::Ready(_)))
            .map(|f| f.task_id.clone())
        else {
            return vec![];
        };
        let (id, effect) = self.brief_request(&run_id, &task_id);
        if let Some(f) = self.open_answer_mut() {
            f.brief = Brief::Loading;
            f.brief_id = Some(id);
        }
        vec![effect]
    }

    /// `Modal::Action`'s form step: Esc goes back to the menu, Enter to the page.
    pub(super) fn on_form_key(
        &mut self,
        flow: &mut ActionFlow,
        mut form: Box<ActionForm>,
        key: KeyEvent,
    ) {
        match form.on_key_in(
            key,
            crate::ui::action_forms::area_width(self.body_area.width),
        ) {
            FormOutcome::Stay => flow.step = ActionStep::Form(form),
            FormOutcome::Back => flow.step = ActionStep::Menu,
            FormOutcome::Page => {
                let info = form.info().clone();
                let mut page = self.page_of(flow, info);
                page.details
                    .extend(form.details(self.settings.badges.ascii));
                page.form = Some(form);
                flow.step = ActionStep::Confirm(page);
            }
        }
    }

    /// The form of the open menu, whether it is being edited or its page shows.
    fn open_form_mut(&mut self, run_id: &str, task_id: &str) -> Option<&mut AnswerForm> {
        match &self.modal {
            Some(Modal::Action(flow)) if flow.run_id == run_id => {}
            _ => return None,
        }
        self.open_answer_mut().filter(|f| f.task_id == task_id)
    }

    /// Decision 15's `TaskDetail` reply (or its refusal) to the answer form, by
    /// `request_id`. `Some` when it was the form's; a closed form just drops it.
    pub(crate) fn route_form_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let (request_id, brief) = match reply {
            RunReply::TaskDetail { detail, request_id } => {
                let lines: Vec<String> = detail
                    .brief
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .take(3)
                    .map(str::to_owned)
                    .collect();
                (*request_id, Brief::Ready(lines))
            }
            RunReply::Refused {
                message,
                request_id,
                ..
            } => {
                let first = message.lines().map(str::trim).find(|l| !l.is_empty());
                (
                    *request_id,
                    Brief::Failed(first.unwrap_or("refused").to_string()),
                )
            }
            _ => return None,
        };
        let id = request_id?;
        if !matches!(self.replies.peek(id), Some(PendingWhat::FormBrief { .. })) {
            // Decision 16: a late reply to the open form's last request still fills a
            // brief that is not ready.
            let f = self
                .open_answer_mut()
                .filter(|f| f.brief_id == Some(id) && !matches!(f.brief, Brief::Ready(_)))?;
            f.brief = brief;
            return Some(vec![]);
        }
        let Some(PendingWhat::FormBrief { run_id, task_id }) =
            self.replies.take(Some(id)).map(|p| p.what)
        else {
            return None;
        };
        if let Some(form) = self.open_form_mut(&run_id, &task_id) {
            form.brief = brief;
        }
        Some(vec![])
    }
}
