//! Milestone 9.5 decisions 17 and 24: the plan rules of `race` and `pair` (rulings
//! RR-5 to RR-8, review ruling I9), and the refusal of an amend of either on a task that has started. Called from
//! `validate_graph::validate_tasks_with`, after each task's own resolution, so a task's
//! test mode is the resolved one (rule 8.3 applied). Pure (design decision 1).

use std::collections::BTreeSet;

use proto::{TaskKind, TestMode};

use super::edits_state::has_started;
use super::model::Task;
use super::plan::PlanError;

/// The race and pair rules for every task of `touched` that has not started, each exact
/// (decisions 17 and 24). A started task is never judged again: its race and pair were
/// decided at dispatch and cannot change (RR-8), and the engine may since have moved its
/// route where no peer exists (review I1, the same reason as final review A-I1). A
/// task's other problems are reported by the other rules. Milestone 9.8 (task
/// M9.8.13): a race needs no peer model; its second racer is its row's fallback, else the
/// task's own route (decision 28).
pub fn validate(tasks: &[Task], touched: &BTreeSet<String>) -> Vec<PlanError> {
    (tasks.iter())
        .filter(|t| !has_started(t) && touched.contains(t.id()))
        .flat_map(task_rules)
        .collect()
}

fn task_rules(task: &Task) -> Vec<PlanError> {
    let id = task.id();
    let e =
        |field: &str, rule: &str, message: String| PlanError::new(Some(id), field, rule, message);
    let mut errors = Vec::new();
    let spec = &task.spec;
    if spec.race {
        let why = if spec.kind != TaskKind::Code {
            Some("only code tasks can race".to_string())
        } else if task.hub {
            Some("a hub task cannot race, because a hub runs alone".to_string())
        } else {
            None
        };
        errors.extend(why.map(|why| e("race", "4.1.race", format!("{why} (rule 4.1.race)"))));
        if spec.pair {
            let text = "a task cannot both race and pair (rule 4.1)";
            errors.push(e("pair", "4.1", text.to_string()));
        }
    }
    if spec.pair {
        if task.test_mode != TestMode::Tdd {
            let text = "a paired task needs test mode tdd (rule 4.1.pair)";
            errors.push(e("pair", "4.1.pair", text.to_string()));
        }
        if spec
            .test_to_write
            .as_deref()
            .is_none_or(|t| t.trim().is_empty())
        {
            let text = "name the test the test writer writes in test_to_write (rule 4.1.pair)";
            errors.push(e("pair", "4.1.pair", text.to_string()));
        }
    }
    errors
}

/// Ruling RR-8: an amend naming `race` or `pair` on a task that has started is refused,
/// one sentence per field, whatever the value (clearing included: the race and the test
/// writer are decided at dispatch).
pub(crate) fn amend_refusals(
    task: &Task,
    race: Option<bool>,
    pair: Option<bool>,
) -> Vec<PlanError> {
    if !has_started(task) {
        return Vec::new();
    }
    let id = task.id();
    [("race", race), ("pair", pair)]
        .into_iter()
        .filter(|(_, set)| set.is_some())
        .map(|(name, _)| {
            let text = format!("task {id} has started: its {name} cannot change");
            PlanError::new(Some(id), "", "13", text)
        })
        .collect()
}

#[cfg(test)]
#[path = "validate_patterns_tests.rs"]
mod tests;
