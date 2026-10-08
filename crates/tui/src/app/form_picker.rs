//! Milestone 9.8 decision 39: the model picker as the goal form and the task edit form
//! use it. `⏎` on a form's model row opens it with `role table` first (no choice); the
//! form's effort row cycles the chosen model's catalog efforts, then none (the row's
//! own, or the model's default). The forms hold the picker; the app, which holds the
//! catalogs, opens it and applies what it picks (`app/goal.rs`, `app/run_gate.rs`).
//! Pure.

use super::Effect;
use super::model_picker::{Catalogs, ModelPicker, Picked, PickerEntry, PickerFor};
use crossterm::event::KeyEvent;
use proto::models::ModelRef;
use proto::{ClientMsg, Runtime};

/// What a key in a form's picker did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FormPick {
    Stay,
    /// `esc`: the picker closes, the form unchanged.
    Close,
    /// A model, or `None` for `role table`.
    Chose(Option<ModelRef>),
    /// `r`: a live probe of both CLIs.
    Refresh,
}

/// The picker over a form, on `current` (or on `role table` with none).
pub(crate) fn open(
    target: PickerFor,
    catalogs: &Catalogs,
    current: Option<&ModelRef>,
    role_table: String,
) -> ModelPicker {
    let first = Some(PickerEntry::RoleTable(role_table));
    ModelPicker::new(target, catalogs, current, first)
}

/// A key while a form's picker is open.
pub(crate) fn on_key(picker: &mut ModelPicker, key: KeyEvent) -> FormPick {
    match picker.on_key(key) {
        None => FormPick::Stay,
        Some(Picked::Model(m)) => FormPick::Chose(Some(m)),
        Some(Picked::RoleTable | Picked::NoFallback) => FormPick::Chose(None),
        Some(Picked::Refresh) => FormPick::Refresh,
        Some(Picked::Cancel) => FormPick::Close,
    }
}

/// `r`'s request (decision 37).
pub(crate) fn refresh() -> Effect {
    Effect::Send(ClientMsg::ListModels {
        runtime: None,
        refresh: true,
    })
}

/// The effort after (or before) `now` among `efforts` then none, round; nothing to
/// cycle when the catalog offers none.
pub(crate) fn cycle_effort(efforts: &[String], now: Option<&str>, forward: bool) -> Option<String> {
    if efforts.is_empty() {
        return now.map(str::to_string);
    }
    let mut order: Vec<Option<&str>> = efforts.iter().map(|e| Some(e.as_str())).collect();
    order.push(None);
    let len = order.len();
    let next = match order.iter().position(|o| *o == now) {
        Some(at) if forward => (at + 1) % len,
        Some(at) => (at + len - 1) % len,
        None if forward => 0,
        None => len - 1,
    };
    order[next].map(str::to_string)
}

/// A route's model as a model reference: both its runtime and its model named.
pub(crate) fn route_model(runtime: Option<Runtime>, model: Option<&str>) -> Option<ModelRef> {
    let (runtime, model) = (runtime?, model?);
    let id = (!model.trim().is_empty()).then(|| model.to_string());
    Some(ModelRef { runtime, id })
}
