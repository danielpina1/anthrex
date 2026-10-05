//! Milestone 9.6's design flow (DF): brainstorm, spec and plan, each approved by the
//! user. This module holds its pure core; every file here but `pack.rs` is pure (no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `SystemTime`, brief decision 1).
//!
//! Task M9.6.3 adds [`mode_for`], DF §1's table: whether a run uses the flow, decided
//! once at the start and frozen as `Run.design_mode`.
//!
//! Task M9.6.4 adds the pure checks and renderers: the spec's requirements, the
//! documents' templates, the plan's coverage and brief shape, `plan.md`, and the change
//! summary and line diff a gate shows.
//!
//! Task M9.6.5 adds [`state`]: the design state on `RunOrch.design` and the index of the
//! documents, whose files the driver writes (`Effect::WriteDoc`, `driver/design_io.rs`).
//!
//! Task M9.6.9 adds [`report`]: the merged brainstorm report's appendix of drafts, and
//! what its gate reads from it.
//!
//! Task M9.6.12 adds [`commit`]: what the documents commit carries, and its paths.

pub mod changes;
pub mod commit;
pub mod coverage;
pub mod epic;
pub mod pack;
pub mod plan_md;
pub mod report;
pub mod requirements;
pub mod spend;
pub mod state;
pub mod template;
pub mod versions;

use proto::{DesignMode, RunPath, TaskKind, TriageInfo};

/// Where a run's goal came from, as DF §1's table reads it.
#[derive(Debug, Clone, Copy)]
pub enum GoalOrigin<'a> {
    /// A goal, with what triage decided. `None` is a continued goal (`start_goal`,
    /// `continue_from`), which is never triaged and reads as a planned code goal
    /// (decision 3's M9.6.1 fact, decision 30).
    Goal(Option<&'a TriageInfo>),
    /// `run start --plan`: the plan already exists.
    PlanFile,
    /// `run promote` of a fast-path run: work has started.
    Promotion,
}

/// DF §1 and decision 3: the design mode of a run started from `origin`.
///
/// The table puts the flow on for a goal on the plan or large path whose every kind is
/// code or docs, and off for anything else. For a goal it puts on, `requested` (`run
/// start --design`, the goal dialog's row) wins, then `config.default`. For a goal it
/// puts off, the mode is `Off`, and a request for `Full` is refused with the exact text
/// `the design flow runs only for planned code or docs goals; this goal is <what>`.
pub fn mode_for(
    origin: GoalOrigin<'_>,
    requested: Option<DesignMode>,
    config: &config::DesignConfig,
) -> Result<DesignMode, String> {
    let off = match origin {
        GoalOrigin::Goal(None) => None,
        GoalOrigin::Goal(Some(triage)) => off_reason(triage),
        GoalOrigin::PlanFile => Some("a plan file"),
        // Only a fast-path run is promoted.
        GoalOrigin::Promotion => Some("fast"),
    };
    match (off, requested) {
        (Some(what), Some(DesignMode::Full)) => Err(format!(
            "the design flow runs only for planned code or docs goals; this goal is {what}"
        )),
        (Some(_), _) => Ok(DesignMode::Off),
        (None, Some(mode)) => Ok(mode),
        (None, None) => Ok(config.default),
    }
}

/// Why DF §1's table puts a triaged goal off, in the refusal's words: the fast path
/// first (the table's earlier row), then the first kind that is neither code nor docs.
fn off_reason(triage: &TriageInfo) -> Option<&'static str> {
    if triage.path == RunPath::Fast {
        return Some("fast");
    }
    triage.kinds.iter().find_map(|kind| match kind {
        TaskKind::Code | TaskKind::Docs => None,
        TaskKind::Research => Some("research"),
        TaskKind::Review => Some("review"),
    })
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
