//! Milestone 9.15 decision 23: the lessons blocks of the worker and reviewer prompts.
//! Pure (design decision 2): each block is a function of the run alone, its lessons
//! scored against the run's freeze time (ruling C8), so a task's worker and reviewer
//! always see the same ids and a replay renders the same text.
//!
//! With lessons off, none frozen, or none that scores, every block is empty and every
//! prompt is byte-identical to what it was before 9.15 (Review focus 3). Test writers
//! get no block (ruling C28): only `worker_prompt` and `reviewer_prompt` call these.

use super::model::{Run, Task};
use crate::lessons::select::{Picked, pick, render};

/// The worker block's heading (decision 23, exact).
pub const WORKER_HEADING: &str = "Notes from earlier runs in this repository. They are advisory; your task and its acceptance criteria above take precedence.";
/// The reviewer block's heading (decision 23, exact).
pub const REVIEWER_HEADING: &str =
    "Known pitfalls in this repository (advisory; check the diff against them):";

/// The lessons `task`'s prompts show, or none when the run's lessons are off or absent.
fn picked(run: &Run, task: &Task) -> Picked {
    match run.lessons.as_ref().filter(|l| l.limits.enabled) {
        Some(lessons) => pick(
            &lessons.items,
            &task.spec.owns,
            lessons.frozen_at,
            &lessons.limits,
        ),
        None => Picked::default(),
    }
}

/// The ids `task`'s prompts show, in order (decision 24's `lessons_shown`).
pub fn shown_ids(run: &Run, task: &Task) -> Vec<String> {
    picked(run, task).ids
}

/// The worker prompt's block, after the notes and before the brief; empty when none.
pub fn worker_block(run: &Run, task: &Task) -> String {
    render(WORKER_HEADING, &picked(run, task))
}

/// The reviewer prompt's block, the same items under [`REVIEWER_HEADING`], after the
/// messages and before the requirements; empty when none.
pub fn reviewer_block(run: &Run, task: &Task) -> String {
    render(REVIEWER_HEADING, &picked(run, task))
}

#[cfg(test)]
#[path = "contract_lessons_tests.rs"]
mod tests;
