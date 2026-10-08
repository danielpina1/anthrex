//! Milestone 8c decision 33: the plan gate's task edit form, pure (`AGENTS.md` hard
//! rule 5). It edits a task's route (its model and effort), size, test mode and its
//! reason, and brief, and sends one `PlanEdit::AmendTask` carrying only what changed.
//! The route is the plan's own `RouteSpec` (`TaskInfo.route_spec`), where `None` is the
//! role table's; the resolved `TaskInfo.route` is only shown beside it. Milestone 9.8
//! decision 39: the model is chosen in the model picker (`role table` first, which
//! clears the route), the effort cycles the chosen model's catalog efforts. The brief is a multi-line `TextArea` (milestone 9.0.7
//! decision 35): Ctrl-J and a pasted newline are real `\n`s, sent as they are.
//! Opening, submitting and the replies are `app/runs.rs`; rendering is `ui/run_edit.rs`.

use crate::app::form_picker::{cycle_effort, route_model};
use crate::app::model_picker::{ModelPicker, runtime_name};
use crate::dialog::{TextInput, apply_text_key};
use crate::text_area::TextArea;
use crate::theme::Palette;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use cycle::{next_size, next_test_mode};
use proto::models::ModelRef;
use proto::{Effort, PlanEdit, Route, RouteSpec, RunInfo, Size, TaskInfo, TaskState, TestMode};

/// The most characters a text field takes, by typing or pasting: a 1 MB paste stops
/// here rather than growing a request the daemon would carry into its plan.
pub const TEXT_MAX_CHARS: usize = 16_384;

/// The most characters the brief holds: one million (the final fix wave's ruling).
/// The daemon checks a brief only for being blank (`daemon/src/run/validate.rs`); its
/// one bound is the frame (`proto::MAX_FRAME`). A brief this long, at most 4 bytes a
/// character, and the edit's other fields (each at most `TEXT_MAX_CHARS`) fit one frame
/// (`the_largest_edit_fits_one_frame`), and the form's per-key draw and insert stay
/// well inside the 100 ms tick (18–23 ms a pass in release at this size, 75–95 ms at
/// the old 4.1 M). A longer brief opens refused, never cut and sent (decision 35).
pub const BRIEF_MAX_CHARS: usize = 1_000_000;

/// A brief past [`BRIEF_MAX_CHARS`] opens cut, says so, and an edit of it is refused,
/// so a cut brief is never sent back.
pub const BRIEF_TOO_LONG: &str = "the brief is too long to edit here; it is not sent";

/// M8a decision 10, mirrored so the form says so before the engine does.
pub const REASON_REQUIRED: &str = "a reason is required when test mode is check or none";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    /// Milestone 9.8 decision 39: `⏎` opens the model picker.
    Model,
    /// The chosen model's catalog efforts, then the role table's.
    Effort,
    Size,
    TestMode,
    Reason,
    Brief,
    /// Milestone 9.1 decision 55: the task's stage, while it has not started.
    Stage,
}

/// The values the form opened with, as the fields show them, so "changed" means the
/// user changed a field and never that the form re-encoded one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskInfoValues {
    route_spec: RouteSpec,
    size: Size,
    test_mode: TestMode,
    reason: String,
    brief: String,
    /// The snapshot's own reason and brief, uncleaned, for `opened_from`.
    raw_reason: Option<String>,
    raw_brief: String,
    stage: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditForm {
    pub run_id: String,
    pub task_id: String,
    /// The plan's route spec as edited: `None` fields are the role table's.
    pub route: RouteSpec,
    /// The picked model as the picker labelled it (`Codex · gpt-6 luna`).
    pub model_label: Option<String>,
    /// The route model's catalog efforts, refreshed by the app before each key.
    pub efforts: Vec<String>,
    /// The table's model as the catalogs label it (`Claude · Sonnet 5`), set by the app;
    /// `None` reads the resolved id.
    pub role_label: Option<String>,
    /// The model picker, while it is open over the form.
    pub picker: Option<ModelPicker>,
    pub size: Size,
    pub test_mode: TestMode,
    pub reason: TextInput,
    pub brief: TextArea,
    /// Milestone 9.1 decision 55: the stage, and the highest it may cycle to; `None`
    /// hides the field (a task that started, or a form opened without its run).
    pub stage: u16,
    stage_max: Option<u16>,
    pub focus: EditField,
    pub error: Option<String>,
    pub submitting: bool,
    /// Milestone 9 decision 2: the id of the tagged `Edit` this form waits on.
    pub request_id: Option<u64>,
    /// `TaskInfo.route`, shown muted beside a `policy` value.
    resolved: Route,
    original: TaskInfoValues,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    Stay,
    Cancel,
    Submit(Vec<PlanEdit>),
    /// Enter with nothing changed: the caller closes the form with `nothing changed`.
    Unchanged,
    /// `⏎` on the model row: the app opens the picker.
    Pick,
}

/// A one-line field's text: newlines and every other control character dropped, a tab
/// a space, so no field ever holds a byte the terminal would act on. Invisible format
/// characters are dropped by the renderer (`safe_text::one_line`), so an untouched
/// field is the plan's own text.
fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

/// What a route field the plan leaves to the role table reads.
pub const ROLE_TABLE: &str = "role table";

pub fn effort_word(effort: Effort) -> String {
    effort.to_string()
}

pub fn size_word(size: Size) -> &'static str {
    match size {
        Size::S => "S",
        Size::M => "M",
        Size::L => "L",
    }
}

pub fn test_mode_word(mode: TestMode) -> &'static str {
    match mode {
        TestMode::Tdd => "tdd",
        TestMode::Check => "check",
        TestMode::None => "none",
    }
}

pub fn field_label(field: EditField) -> &'static str {
    match field {
        EditField::Model => "model",
        EditField::Effort => "effort",
        EditField::Size => "size",
        EditField::TestMode => "test mode",
        EditField::Reason => "reason",
        EditField::Brief => "brief",
        EditField::Stage => "stage",
    }
}

/// Whether a brief as opened reached [`BRIEF_MAX_CHARS`], so may have been cut.
fn is_cut(brief: &str) -> bool {
    brief.chars().count() >= BRIEF_MAX_CHARS
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

/// Inserts `text` into `input`, cut so the field never passes [`TEXT_MAX_CHARS`].
fn insert_bounded(input: &mut TextInput, text: &str) {
    let room = TEXT_MAX_CHARS.saturating_sub(input.text().chars().count());
    if room == 0 || text.is_empty() {
        return;
    }
    let cut: String = text.chars().take(room).collect();
    input.insert(&cut);
}

impl TaskEditForm {
    pub fn new(run_id: &str, task: &TaskInfo) -> Self {
        let spec = task.route_spec.clone();
        // Control characters are dropped here, so an edited field is sent without them.
        let reason = clean(task.test_mode_reason.as_deref().unwrap_or(""));
        // `TextArea` keeps newlines and drops control and invisible format characters.
        let brief = TextArea::with_cap(&task.brief, BRIEF_MAX_CHARS);
        let brief_cut = is_cut(brief.text());
        Self {
            run_id: run_id.to_string(),
            task_id: task.id.clone(),
            route: spec.clone(),
            model_label: None,
            efforts: Vec::new(),
            role_label: None,
            picker: None,
            size: task.size,
            test_mode: task.test_mode,
            reason: TextInput::new(&reason),
            brief: brief.clone(),
            stage: task.stage,
            stage_max: None,
            focus: EditField::Model,
            error: brief_cut.then(|| BRIEF_TOO_LONG.to_string()),
            submitting: false,
            request_id: None,
            resolved: task.route.clone(),
            original: TaskInfoValues {
                route_spec: spec,
                size: task.size,
                test_mode: task.test_mode,
                reason,
                brief: brief.text().to_string(),
                raw_reason: task.test_mode_reason.clone(),
                raw_brief: task.brief.clone(),
                stage: task.stage,
            },
        }
    }

    /// The form on `task` of `run`: as [`TaskEditForm::new`], with the stage field
    /// while the task has not started, cycling from 1 to the run's highest stage plus
    /// one (decision 55). The daemon's validation answers an invalid move inline.
    pub fn in_run(run: &RunInfo, task: &TaskInfo) -> Self {
        let mut form = Self::new(&run.run_id, task);
        // The daemon's `not_started` (a message pause has started), and no round yet.
        let not_started = matches!(
            task.state,
            TaskState::Pending | TaskState::Queued | TaskState::Blocked
        ) && !crate::tree::is_paused(task)
            && task.rounds.is_empty();
        if not_started {
            let highest = (run.tasks.iter().map(|t| t.stage))
                .chain(u16::try_from(run.stages.len()))
                .max()
                .unwrap_or(1)
                .max(task.stage)
                .max(1);
            form.stage_max = Some(highest.saturating_add(1));
        }
        form
    }

    /// Review M6: `task` still holds every value the form opened from. An amend sends
    /// the whole route, so one changed elsewhere would be reverted.
    pub fn opened_from(&self, task: &TaskInfo) -> bool {
        let original = &self.original;
        task.route_spec == original.route_spec
            && task.size == original.size
            && task.test_mode == original.test_mode
            && task.test_mode_reason == original.raw_reason
            && task.brief == original.raw_brief
            && task.stage == original.stage
    }

    /// Every field in order; `Reason` only while the test mode is not `tdd`.
    pub fn visible_fields(&self) -> Vec<EditField> {
        let mut fields = vec![
            EditField::Model,
            EditField::Effort,
            EditField::Size,
            EditField::TestMode,
        ];
        if self.test_mode != TestMode::Tdd {
            fields.push(EditField::Reason);
        }
        fields.push(EditField::Brief);
        if self.stage_max.is_some() {
            fields.push(EditField::Stage);
        }
        fields
    }

    fn move_focus(&mut self, delta: isize) {
        let fields = self.visible_fields();
        let at = fields.iter().position(|f| *f == self.focus).unwrap_or(0);
        let next = (at as isize + delta).rem_euclid(fields.len() as isize);
        self.focus = fields[next as usize];
    }

    fn focused_text_mut(&mut self) -> Option<&mut TextInput> {
        match self.focus {
            EditField::Reason => Some(&mut self.reason),
            _ => None,
        }
    }

    /// The route's model, when the plan (or the picker) names both its runtime and its
    /// model.
    pub fn current_model(&self) -> Option<ModelRef> {
        route_model(self.route.runtime, self.route.model.as_deref())
    }

    /// The model whose catalog efforts the effort row cycles: the route's, else (the
    /// route names no model) the one the task resolves to, on its runtime.
    pub fn effort_model(&self) -> Option<ModelRef> {
        self.current_model().or_else(|| self.table_model())
    }

    /// The model the table runs the task on, while the task's resolution is the
    /// table's: the route the form opened with named no model (review I3).
    pub fn table_model(&self) -> Option<ModelRef> {
        let resolved = &self.resolved;
        let runtime = self.route.runtime.unwrap_or(resolved.runtime);
        (self.original.route_spec.model.is_none() && runtime == resolved.runtime)
            .then(|| route_model(Some(runtime), Some(&resolved.model)))
            .flatten()
    }

    /// `role table (<model> · <effort>)`: what the table runs, while the resolution is
    /// the table's (the effort only when the opened route set none); else `role table`.
    pub fn role_table_text(&self) -> String {
        let Some(model) = self.table_model() else {
            return ROLE_TABLE.to_string();
        };
        let label = self.role_label.clone().unwrap_or_else(|| {
            let id = model.id.as_deref().map_or_else(|| "default".into(), clean);
            format!("{} · {id}", runtime_name(model.runtime))
        });
        let effort = &self.resolved.effort;
        match &self.original.route_spec.effort {
            None if effort.is_default() => format!("{ROLE_TABLE} ({label} · default)"),
            None => format!("{ROLE_TABLE} ({label} · {})", clean(effort.as_str())),
            Some(_) => format!("{ROLE_TABLE} ({label})"),
        }
    }

    /// Decision 39: the picker chose `model` (labelled `label`), or the role table
    /// (`None`), which clears the whole route. A new model's effort is the role
    /// table's until cycled.
    pub fn choose(&mut self, model: Option<ModelRef>, label: String) {
        // Review I1: the same model again keeps the route, its effort with it.
        if model.is_some() && model == self.current_model() {
            self.model_label = Some(label).filter(|l| !l.is_empty());
            return;
        }
        self.route = match model {
            Some(m) => RouteSpec {
                runtime: Some(m.runtime),
                model: Some(m.route_model().to_string()),
                strength: None,
                effort: None,
            },
            None => RouteSpec::default(),
        };
        self.model_label = Some(label).filter(|l| !l.is_empty());
        self.efforts.clear();
    }

    fn cycle(&mut self, forward: bool) {
        match self.focus {
            EditField::Effort => {
                let now = self.route.effort.as_ref().map(|e| e.as_str());
                let next = cycle_effort(&self.efforts, now, forward);
                self.route.effort = next.map(Effort::new);
            }
            EditField::Size => self.size = next_size(self.size, forward),
            EditField::TestMode => self.test_mode = next_test_mode(self.test_mode, forward),
            EditField::Stage => {
                let max = self.stage_max.unwrap_or(self.stage).max(1);
                self.stage = match (self.stage.clamp(1, max), forward) {
                    (at, true) if at >= max => 1,
                    (at, true) => at + 1,
                    (1, false) => max,
                    (at, false) => at - 1,
                };
            }
            EditField::Model | EditField::Reason | EditField::Brief => {}
        }
    }

    /// [`TaskEditForm::on_key_in`] with the brief unwrapped: Up and Down in it move a
    /// logical line.
    pub fn on_key(&mut self, key: KeyEvent) -> EditOutcome {
        self.on_key_in(key, 0)
    }

    /// Interfaces "The task edit form" keys. While submitting, only `Esc` and `Ctrl-C`
    /// do anything, so a second `Enter` sends nothing. `brief_width` is the width the
    /// brief's text area is drawn at (`ui::run_edit::brief_width`, 0: unwrapped): in the
    /// brief, Up and Down move a drawn row, and leave the field from its first or last
    /// (decision 35).
    pub fn on_key_in(&mut self, key: KeyEvent, brief_width: u16) -> EditOutcome {
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            return EditOutcome::Cancel;
        }
        if self.submitting {
            return EditOutcome::Stay;
        }
        let in_brief = self.focus == EditField::Brief;
        match key.code {
            KeyCode::Up | KeyCode::Down if in_brief && self.brief.on_key_in(key, brief_width) => {}
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::Enter if self.focus == EditField::Model => return EditOutcome::Pick,
            KeyCode::Enter => return self.submit(),
            _ if in_brief => {
                self.brief.on_key_in(key, brief_width);
            }
            _ => match self.focus {
                EditField::Reason => self.on_text_key(key),
                _ => match key.code {
                    KeyCode::Right | KeyCode::Char(' ') => self.cycle(true),
                    KeyCode::Left => self.cycle(false),
                    _ => {}
                },
            },
        }
        EditOutcome::Stay
    }

    fn on_text_key(&mut self, key: KeyEvent) {
        let Some(input) = self.focused_text_mut() else {
            return;
        };
        if is_ctrl(&key, 'j') {
            return;
        }
        let full = input.text().chars().count() >= TEXT_MAX_CHARS;
        let typing =
            matches!(key.code, KeyCode::Char(_)) && !key.modifiers.contains(KeyModifiers::CONTROL);
        if !(full && typing) {
            apply_text_key(input, key);
        }
    }

    fn submit(&mut self) -> EditOutcome {
        match self.edits() {
            Ok(edits) if edits.is_empty() => EditOutcome::Unchanged,
            Ok(edits) => {
                self.submitting = true;
                self.error = None;
                EditOutcome::Submit(edits)
            }
            Err((field, message)) => {
                self.focus = field;
                self.error = Some(message);
                EditOutcome::Stay
            }
        }
    }

    /// A paste into the focused text field: newlines kept in the brief and dropped
    /// elsewhere; control characters are dropped; the field stays bounded.
    pub fn on_paste(&mut self, text: &str) {
        if self.submitting {
            return;
        }
        if self.focus == EditField::Brief {
            self.brief.on_paste(text);
            return;
        }
        let text = clean(text);
        if let Some(input) = self.focused_text_mut() {
            insert_bounded(input, &text);
        }
    }

    /// Decision 33: the one `AmendTask` naming only what changed, or no edit at all
    /// when nothing did. The route is `route_spec` with only the changed fields
    /// replaced, because `AmendTask.route` replaces the whole spec route. A mode other
    /// than `tdd` needs a non-blank reason whenever the mode or its reason is sent.
    pub fn edits(&self) -> Result<Vec<PlanEdit>, (EditField, String)> {
        let original = &self.original;
        let route_changed = self.route != original.route_spec;
        // Review minor 4: a route naming no model carries only its effort (the daemon
        // refuses a runtime or a strength alone).
        let route = match &self.route.model {
            Some(_) => self.route.clone(),
            None => RouteSpec {
                effort: self.route.effort.clone(),
                ..RouteSpec::default()
            },
        };

        let mode_changed = self.test_mode != original.test_mode;
        let reason_changed =
            self.test_mode != TestMode::Tdd && self.reason.text() != original.reason;
        let reason = self.reason.text().trim();
        if self.test_mode != TestMode::Tdd && (mode_changed || reason_changed) && reason.is_empty()
        {
            return Err((EditField::Reason, REASON_REQUIRED.to_string()));
        }
        let brief_changed = self.brief.text() != original.brief;
        // The brief opened cut at the cap: an edit of it is refused.
        if brief_changed && is_cut(&original.brief) {
            return Err((EditField::Brief, BRIEF_TOO_LONG.to_string()));
        }

        let edit = PlanEdit::AmendTask {
            task_id: self.task_id.clone(),
            brief: brief_changed.then(|| self.brief.text().to_string()),
            acceptance: None,
            route: route_changed.then_some(route),
            test_mode: mode_changed.then_some(self.test_mode),
            test_mode_reason: (self.test_mode != TestMode::Tdd && (mode_changed || reason_changed))
                .then(|| reason.to_string()),
            priority: None,
            size: (self.size != original.size).then_some(self.size),
            deps: None,
            stage: (self.stage != original.stage).then_some(self.stage),
            race: None,
            pair: None,
        };
        let changed = route_changed
            || mode_changed
            || reason_changed
            || brief_changed
            || self.size != original.size
            || self.stage != original.stage;
        Ok(if changed { vec![edit] } else { vec![] })
    }

    /// [`TaskEditForm::value_parts_in`] in unicode, for tests.
    #[cfg(test)]
    pub fn value_parts(&self, field: EditField) -> (String, Option<String>) {
        self.value_parts_in(field, Palette::PLAIN)
    }

    /// A row's value as drawn: the text, and the resolved value shown muted after it
    /// when the field is the role table's (decision 33) — only while the route is the
    /// one the form opened with, since the resolution is of that route (review M4). A
    /// choice is `theme::choice`'s (`kit::choice_in`'s), `< value >` in ASCII (decision
    /// 35).
    pub fn value_parts_in(&self, field: EditField, p: Palette) -> (String, Option<String>) {
        let fold = |text: &str| crate::theme::fold(text, p.ascii);
        let choice = |word: &str| crate::theme::choice(&fold(word), p);
        let opened = self.route == self.original.route_spec;
        let muted = |resolved: String| Some(fold(&resolved)).filter(|r| opened && !r.is_empty());
        let resolved = &self.resolved;
        match field {
            EditField::Model => match (self.current_model(), self.route.runtime) {
                (Some(m), _) => {
                    let id = m.id.as_deref().map_or_else(|| "default".into(), clean);
                    let label = (self.model_label.clone())
                        .unwrap_or_else(|| format!("{} · {id}", runtime_name(m.runtime)));
                    (choice(&label), None)
                }
                // A plan that names only the runtime: the table's model on it.
                (None, Some(runtime)) => (
                    choice(&format!("{} · {ROLE_TABLE}", runtime_name(runtime))),
                    muted(clean(&resolved.model)),
                ),
                (None, None) => (choice(&self.role_table_text()), None),
            },
            EditField::Effort => match (&self.route.effort, &self.route.model) {
                (Some(effort), _) => (choice(&effort_word(effort.clone())), None),
                // Fix round 1 (controller ruling I2): a named model with no effort runs
                // at its default.
                (None, Some(_)) => (choice("default"), None),
                (None, None) => (
                    choice(ROLE_TABLE),
                    muted(effort_word(resolved.effort.clone())),
                ),
            },
            EditField::Size => (choice(size_word(self.size)), None),
            EditField::TestMode => (choice(test_mode_word(self.test_mode)), None),
            EditField::Reason => (self.reason.text().to_string(), None),
            EditField::Brief => (self.brief.text().to_string(), None),
            EditField::Stage => (choice(&self.stage.to_string()), None),
        }
    }
}

#[path = "run_edit_cycle.rs"]
mod cycle;

#[cfg(test)]
#[path = "run_edit_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
#[path = "run_edit_tests_models.rs"]
mod tests_models;
