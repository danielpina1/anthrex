//! The task edit form's role table row (milestone 9.8 decision 39, M9.8.11's fix round
//! and M9.8.15): which model and effort the table runs the task on, as the form draws
//! them, split from `run_edit.rs` for the 600-line rule.

use super::{ROLE_TABLE, TaskEditForm, clean};
use crate::app::form_picker::route_model;
use crate::app::model_picker::{is_row_model, runtime_name};
use proto::Size;
use proto::models::ModelRef;

impl TaskEditForm {
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
        if self.size_moved_row() {
            return None;
        }
        if let Some(row) = &self.row {
            return route_model(Some(row.runtime), Some(&row.model));
        }
        let resolved = &self.resolved;
        let runtime = self.route.runtime.unwrap_or(resolved.runtime);
        (self.original.route_spec.model.is_none() && runtime == resolved.runtime)
            .then(|| route_model(Some(runtime), Some(&resolved.model)))
            .flatten()
    }

    /// `role table (<model> · <effort>)`: what the table runs, while the resolution is
    /// the table's (the effort only when the opened route set none); else `role table`.
    pub fn role_table_text(&self) -> String {
        if self.size_moved_row() {
            return format!("{ROLE_TABLE} (resolved on save)");
        }
        let Some(model) = self.table_model() else {
            return ROLE_TABLE.to_string();
        };
        let label = self.role_label.clone().unwrap_or_else(|| {
            let id = model.id.as_deref().map_or_else(|| "default".into(), clean);
            format!("{} · {id}", runtime_name(model.runtime))
        });
        // M9.8.15: a pinned effort runs over the row's model; the effort row shows it.
        if let Some(row) = self.row.as_ref().filter(|_| self.route.effort.is_none()) {
            let effort = Some(row.effort.as_str()).filter(|e| !e.is_empty());
            let effort = clean(effort.unwrap_or("default"));
            return format!("{ROLE_TABLE} ({label} · {effort})");
        }
        if self.row.is_some() {
            return format!("{ROLE_TABLE} ({label})");
        }
        let effort = &self.resolved.effort;
        match &self.original.route_spec.effort {
            None if effort.is_default() => format!("{ROLE_TABLE} ({label} · default)"),
            None => format!("{ROLE_TABLE} ({label} · {})", clean(effort.as_str())),
            Some(_) => format!("{ROLE_TABLE} ({label})"),
        }
    }

    /// A named model's blank effort: the row's (`<effort> (role table)`) for the row's
    /// own model, which the daemon runs at the row's effort, else `default`.
    pub(super) fn picked_default(&self) -> String {
        match self.row.as_ref().filter(|_| !self.size_moved_row()) {
            Some(row) if self.picked_is_row(row) => {
                let effort = Some(row.effort.as_str()).filter(|e| !e.is_empty());
                format!("{} ({ROLE_TABLE})", clean(effort.unwrap_or("default")))
            }
            _ => "default".to_string(),
        }
    }

    /// Gate fix B2: the route's model is the row's own (by the catalogs' identity).
    fn picked_is_row(&self, row: &proto::Route) -> bool {
        let row_model = route_model(Some(row.runtime), Some(&row.model));
        match (self.current_model(), row_model) {
            (Some(m), Some(r)) => is_row_model(&r, &self.row_aliases, &m),
            (m, r) => m == r,
        }
    }

    /// M9.8.15: a size change that may move the task to another row, which the form does
    /// not hold; it is resolved on save.
    pub(super) fn size_moved_row(&self) -> bool {
        self.original.sized_row && (self.size == Size::S) != (self.original.size == Size::S)
    }
}
