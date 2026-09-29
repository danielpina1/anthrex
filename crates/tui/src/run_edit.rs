//! Milestone 8c decision 33: the plan gate's task edit form, pure (`AGENTS.md` hard
//! rule 5). It edits a task's route (runtime, model, strength, effort), size, test mode
//! and its reason, and brief, and sends one `PlanEdit::AmendTask` carrying only what
//! changed. The route fields hold the plan's own `RouteSpec` values (`TaskInfo.route_spec`),
//! where `None` is "policy"; the resolved `TaskInfo.route` is only shown beside them.
//! Opening, submitting and the replies are `app/runs.rs`; rendering is `ui/run_edit.rs`.

use crate::dialog::{TextInput, apply_text_key};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Effort, PlanEdit, Route, RouteSpec, Runtime, Size, Strength, TaskInfo, TestMode};

/// Where a newline in the one-line brief is drawn, and restored from on submit.
pub const NEWLINE_MARK: char = '↵';

/// The most characters a text field takes, by typing or pasting: a 1 MB paste stops
/// here rather than growing a request the daemon would carry into its plan.
pub const TEXT_MAX_CHARS: usize = 16_384;

/// M8a decision 10, mirrored so the form says so before the engine does.
pub const REASON_REQUIRED: &str = "a reason is required when test mode is check or none";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditField {
    Runtime,
    Model,
    Strength,
    Effort,
    Size,
    TestMode,
    Reason,
    Brief,
}

/// The values the form opened with, as the fields show them, so "changed" means the
/// user changed a field and never that the form re-encoded one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TaskInfoValues {
    route_spec: RouteSpec,
    model: String,
    size: Size,
    test_mode: TestMode,
    reason: String,
    brief: String,
    /// The snapshot's own reason and brief, uncleaned, for `opened_from`.
    raw_reason: Option<String>,
    raw_brief: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskEditForm {
    pub run_id: String,
    pub task_id: String,
    // Route fields hold the plan's RouteSpec values: None is "policy" (decision 33).
    pub runtime: Option<Runtime>,
    pub model: TextInput,
    pub strength: Option<Strength>,
    pub effort: Option<Effort>,
    pub size: Size,
    pub test_mode: TestMode,
    pub reason: TextInput,
    pub brief: TextInput,
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
}

/// Newlines become `newline` (or go, when `None`), a tab a space, and every other
/// control character is dropped, so no field ever holds a byte the terminal would act on.
fn clean(text: &str, newline: Option<char>) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter_map(|c| match c {
            '\n' => newline,
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

fn next_runtime(value: Option<Runtime>, forward: bool) -> Option<Runtime> {
    let order = [None, Some(Runtime::Claude), Some(Runtime::Codex)];
    step(&order, value, forward)
}

fn next_strength(value: Option<Strength>, forward: bool) -> Option<Strength> {
    let order = [
        None,
        Some(Strength::Fast),
        Some(Strength::Standard),
        Some(Strength::Frontier),
    ];
    step(&order, value, forward)
}

fn next_effort(value: Option<Effort>, forward: bool) -> Option<Effort> {
    let order = [
        None,
        Some(Effort::Low),
        Some(Effort::Medium),
        Some(Effort::High),
    ];
    step(&order, value, forward)
}

fn next_test_mode(value: TestMode, forward: bool) -> TestMode {
    step(
        &[TestMode::Tdd, TestMode::Check, TestMode::None],
        value,
        forward,
    )
}

/// `S ↔ M`; an `L` the plan opened with steps into them (`S` forward, `M` back).
fn next_size(value: Size, forward: bool) -> Size {
    match (value, forward) {
        (Size::S, _) => Size::M,
        (Size::M, _) => Size::S,
        (Size::L, true) => Size::S,
        (Size::L, false) => Size::M,
    }
}

/// The neighbour of `value` in `order`, wrapping; a value not in `order` (a `shell`
/// runtime from a hand-written plan) steps to the first.
fn step<T: Copy + PartialEq>(order: &[T], value: T, forward: bool) -> T {
    let len = order.len();
    match order.iter().position(|v| *v == value) {
        Some(at) if forward => order[(at + 1) % len],
        Some(at) => order[(at + len - 1) % len],
        None => order[0],
    }
}

pub fn runtime_word(runtime: Runtime) -> &'static str {
    runtime.label()
}

pub fn strength_word(strength: Strength) -> &'static str {
    match strength {
        Strength::Fast => "fast",
        Strength::Standard => "standard",
        Strength::Frontier => "frontier",
    }
}

pub fn effort_word(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
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
        EditField::Runtime => "runtime",
        EditField::Model => "model",
        EditField::Strength => "strength",
        EditField::Effort => "effort",
        EditField::Size => "size",
        EditField::TestMode => "test mode",
        EditField::Reason => "reason",
        EditField::Brief => "brief",
    }
}

fn choice(word: &str) -> String {
    format!("‹ {word} ›")
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
        // Invisible characters are dropped here, so an edited field is sent without them.
        let model = clean(spec.model.as_deref().unwrap_or(""), None);
        let reason = clean(task.test_mode_reason.as_deref().unwrap_or(""), None);
        let brief = clean(&task.brief, Some(NEWLINE_MARK));
        Self {
            run_id: run_id.to_string(),
            task_id: task.id.clone(),
            runtime: spec.runtime,
            model: TextInput::new(&model),
            strength: spec.strength,
            effort: spec.effort,
            size: task.size,
            test_mode: task.test_mode,
            reason: TextInput::new(&reason),
            brief: TextInput::new(&brief),
            focus: EditField::Runtime,
            error: None,
            submitting: false,
            request_id: None,
            resolved: task.route.clone(),
            original: TaskInfoValues {
                route_spec: spec,
                model,
                size: task.size,
                test_mode: task.test_mode,
                reason,
                brief,
                raw_reason: task.test_mode_reason.clone(),
                raw_brief: task.brief.clone(),
            },
        }
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
    }

    /// Every field in order; `Reason` only while the test mode is not `tdd`.
    pub fn visible_fields(&self) -> Vec<EditField> {
        let mut fields = vec![
            EditField::Runtime,
            EditField::Model,
            EditField::Strength,
            EditField::Effort,
            EditField::Size,
            EditField::TestMode,
        ];
        if self.test_mode != TestMode::Tdd {
            fields.push(EditField::Reason);
        }
        fields.push(EditField::Brief);
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
            EditField::Model => Some(&mut self.model),
            EditField::Reason => Some(&mut self.reason),
            EditField::Brief => Some(&mut self.brief),
            _ => None,
        }
    }

    fn cycle(&mut self, forward: bool) {
        match self.focus {
            EditField::Runtime => {
                self.runtime = next_runtime(self.runtime, forward);
                // Decision 33: a model names one runtime's model.
                self.model.clear();
            }
            EditField::Strength => self.strength = next_strength(self.strength, forward),
            EditField::Effort => self.effort = next_effort(self.effort, forward),
            EditField::Size => self.size = next_size(self.size, forward),
            EditField::TestMode => self.test_mode = next_test_mode(self.test_mode, forward),
            EditField::Model | EditField::Reason | EditField::Brief => {}
        }
    }

    /// Interfaces "The task edit form" keys. While submitting, only `Esc` and `Ctrl-C`
    /// do anything, so a second `Enter` sends nothing.
    pub fn on_key(&mut self, key: KeyEvent) -> EditOutcome {
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            return EditOutcome::Cancel;
        }
        if self.submitting {
            return EditOutcome::Stay;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.move_focus(1),
            KeyCode::BackTab | KeyCode::Up => self.move_focus(-1),
            KeyCode::Enter => return self.submit(),
            _ => match self.focus {
                EditField::Model | EditField::Reason | EditField::Brief => self.on_text_key(key),
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
        let brief = self.focus == EditField::Brief;
        let Some(input) = self.focused_text_mut() else {
            return;
        };
        if is_ctrl(&key, 'j') {
            if brief {
                insert_bounded(input, &NEWLINE_MARK.to_string());
            }
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

    /// A paste into the focused text field: newlines become `↵` in the brief and are
    /// dropped elsewhere; control characters are dropped; the field stays bounded.
    pub fn on_paste(&mut self, text: &str) {
        if self.submitting {
            return;
        }
        let newline = (self.focus == EditField::Brief).then_some(NEWLINE_MARK);
        let text = clean(text, newline);
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
        let model_changed = self.model.text() != original.model;
        let mut route = original.route_spec.clone();
        let runtime_changed = self.runtime != route.runtime;
        let strength_changed = self.strength != route.strength;
        let effort_changed = self.effort != route.effort;
        if runtime_changed {
            route.runtime = self.runtime;
        }
        if model_changed {
            let model = self.model.text().trim();
            route.model = (!model.is_empty()).then(|| model.to_string());
        }
        if strength_changed {
            route.strength = self.strength;
        }
        if effort_changed {
            route.effort = self.effort;
        }
        let route_changed = runtime_changed || model_changed || strength_changed || effort_changed;

        let mode_changed = self.test_mode != original.test_mode;
        let reason_changed =
            self.test_mode != TestMode::Tdd && self.reason.text() != original.reason;
        let reason = self.reason.text().trim();
        if self.test_mode != TestMode::Tdd && (mode_changed || reason_changed) && reason.is_empty()
        {
            return Err((EditField::Reason, REASON_REQUIRED.to_string()));
        }
        let brief_changed = self.brief.text() != original.brief;

        let edit = PlanEdit::AmendTask {
            task_id: self.task_id.clone(),
            brief: brief_changed.then(|| self.brief.text().replace(NEWLINE_MARK, "\n")),
            acceptance: None,
            route: route_changed.then_some(route),
            test_mode: mode_changed.then_some(self.test_mode),
            test_mode_reason: (self.test_mode != TestMode::Tdd && (mode_changed || reason_changed))
                .then(|| reason.to_string()),
            priority: None,
            size: (self.size != original.size).then_some(self.size),
            deps: None,
            stage: None,
        };
        let changed = route_changed
            || mode_changed
            || reason_changed
            || brief_changed
            || self.size != original.size;
        Ok(if changed { vec![edit] } else { vec![] })
    }

    /// A row's value as drawn: the text, and the resolved value shown muted after it
    /// when the field is `policy` (decision 33) — only while the runtime is the task's
    /// current one, since a resolution names one runtime's values (review M4).
    pub fn value_parts(&self, field: EditField) -> (String, Option<String>) {
        let current = self.runtime.unwrap_or(self.resolved.runtime) == self.resolved.runtime;
        let muted = |resolved: String| Some(resolved).filter(|r| current && !r.is_empty());
        let policy = |resolved: &str| (choice("policy"), muted(resolved.to_string()));
        match field {
            EditField::Runtime => match self.runtime {
                Some(runtime) => (choice(runtime_word(runtime)), None),
                None => policy(runtime_word(self.resolved.runtime)),
            },
            EditField::Model if self.model.text().is_empty() => {
                let resolved = clean(&self.resolved.model, None);
                ("policy".to_string(), muted(resolved))
            }
            EditField::Model => (self.model.text().to_string(), None),
            EditField::Strength => match self.strength {
                Some(strength) => (choice(strength_word(strength)), None),
                None => policy(strength_word(self.resolved.strength)),
            },
            EditField::Effort => match self.effort {
                Some(effort) => (choice(effort_word(effort)), None),
                None => policy(effort_word(self.resolved.effort)),
            },
            EditField::Size => (choice(size_word(self.size)), None),
            EditField::TestMode => (choice(test_mode_word(self.test_mode)), None),
            EditField::Reason => (self.reason.text().to_string(), None),
            EditField::Brief => (self.brief.text().to_string(), None),
        }
    }
}

#[cfg(test)]
#[path = "run_edit_tests.rs"]
pub(crate) mod tests;
