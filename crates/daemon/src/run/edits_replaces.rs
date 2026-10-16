//! Milestone 9.17 decisions 23 and 24: `add_task`'s `replaces`. Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime`.
//!
//! The rules read the run as the batch's earlier edits left it, so a `cancel_task`
//! before the `add_task` makes a working task replaceable, and one after it does not.
//! Every error joins the batch's list with rule `replaces`, which refuses the batch.

use std::collections::BTreeSet;

use proto::TaskState;

use super::{Batch, state_label};
use crate::run::model::Run;
use crate::run::orch::EditSource;
use crate::run::orch::plan_frame::clean;
use crate::run::plan::PlanError;

/// Rule (c)'s bound, the MCP schema's (`orch::tools_bounds`), checked again here for
/// the user's `run edit --file`.
pub(crate) const REPLACES_MAX: usize = 12;

impl Batch {
    /// Decision 23's rules over `ids`, the tasks the just-added `new_id` replaces, then
    /// decision 24's effect: a `blocked` one is cancelled (`cancelled: replaced by
    /// <new>`), a `cancelled` one gains `replaced by <new>`, and the new task stores
    /// the list. A replaced task is never changed otherwise.
    pub(super) fn replaces(&mut self, new_id: &str, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let errors = self.errors.len();
        if ids.len() > REPLACES_MAX {
            let n = ids.len();
            return self.refuse_replaces(new_id, format!("at most {REPLACES_MAX} tasks, not {n}"));
        }
        let mut seen = BTreeSet::new();
        let mut named = Vec::new();
        for id in ids {
            let shown = clean(id);
            if !seen.insert(id) {
                self.refuse_replaces(new_id, format!("{shown} is named twice"));
                continue;
            }
            // Rule (a).
            let Some(i) = self.run.tasks.iter().position(|t| t.id() == id) else {
                self.refuse_replaces(new_id, format!("no task {shown} in this run"));
                continue;
            };
            // Rule (d): a sub-planner names only its own epic's tasks.
            if let EditSource::Planner { epic } = &self.source
                && self.run.tasks[i].spec.epic.as_ref() != Some(epic)
            {
                let text = format!("{shown} is not in epic {epic}");
                self.refuse_replaces(new_id, text);
                continue;
            }
            // Rule (b), and pre-flight ruling C3: never a block only the user may cancel.
            match self.run.tasks[i].state {
                TaskState::Cancelled => {}
                TaskState::Blocked => {
                    if let Some(mut error) = self.user_only_refusal(i, id) {
                        error.rule = "replaces".to_string();
                        self.errors.push(error);
                        continue;
                    }
                }
                _ => {
                    let state = state_label(&self.run.tasks[i]);
                    let text = format!(
                        "{shown} is {state}; only a cancelled or blocked task can be replaced"
                    );
                    self.refuse_replaces(new_id, text);
                    continue;
                }
            }
            named.push(i);
        }
        if self.errors.len() > errors {
            return;
        }
        let Some(new) = self.run.tasks.iter().rposition(|t| t.id() == new_id) else {
            return;
        };
        self.run.tasks[new].replaces = ids.to_vec();
        // Rule (e).
        if let Some(text) = cycle(&self.run, new_id) {
            return (self.errors).push(PlanError::new(Some(new_id), "", "replaces", text));
        }
        for i in named {
            if self.run.tasks[i].state == TaskState::Blocked {
                self.cancel_logged(i, &format!("cancelled: replaced by {new_id}"));
            } else {
                self.log(i, format!("replaced by {new_id}"));
            }
        }
    }

    /// One rule's refusal: `replaces: <why>`, attributed to the new task.
    fn refuse_replaces(&mut self, new_id: &str, why: String) {
        let text = format!("replaces: {why}");
        (self.errors).push(PlanError::new(Some(new_id), "", "replaces", text));
    }
}

/// Rule (e): the cycle through `from` in the replaced-by relation, as its text
/// `replaces: <a> → <b> → … → <a> is a cycle`. A new task replaces only existing ones,
/// so this cannot fire today; it guards a later edit kind and costs one walk.
pub(super) fn cycle(run: &Run, from: &str) -> Option<String> {
    fn walk<'a>(run: &'a Run, at: &'a str, from: &str, path: &mut Vec<&'a str>) -> bool {
        let Some(task) = run.tasks.iter().find(|t| t.id() == at) else {
            return false;
        };
        for next in &task.replaces {
            if next == from {
                return true;
            }
            if path.contains(&next.as_str()) {
                continue;
            }
            path.push(next);
            if walk(run, next, from, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = vec![from];
    walk(run, from, from, &mut path).then(|| {
        path.push(from);
        format!("replaces: {} is a cycle", path.join(" → "))
    })
}

#[cfg(test)]
#[path = "edits_tests_replaces.rs"]
mod tests;
