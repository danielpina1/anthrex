//! `[orchestrator.design]` and `[orchestrator.design.budget.*]` (milestone 9.6, DF §1
//! and §2.2): when a goal runs the design flow, where its documents are committed, how
//! many questions the orchestrator may ask, each orchestrator phase's wall-clock
//! budget, and the brainstormers' and document reviewers' budgets. Read by one call,
//! [`read`], from `orchestrator::read`; unknown keys are reported by
//! [`report_unknown_design`], which `orchestrator/unknown.rs` calls for `design`.
//!
//! As everywhere in `[orchestrator]`, an invalid value is a [`Problem`] and keeps its
//! default. The `[orchestrator.routes.brainstorm]` list is read with the other route
//! lists (`orchestrator/routes.rs`).

use proto::{Budget, DesignMode};

use super::budget::read_budget;
use super::read_u32_in_range;
use crate::{Problem, not_a_table_problem, read_bool_key, report_unknown_nested};

const KNOWN_DESIGN_KEYS: &[&str] = &[
    "default",
    "docs_dir",
    "commit_brainstorm",
    "max_questions",
    "phase_minutes",
    "budget",
];
const KNOWN_DESIGN_BUDGET_KEYS: &[&str] = &["brainstormer", "doc_reviewer"];
const KNOWN_BUDGET_KEYS: &[&str] = &["tool_calls", "minutes", "tokens"];

/// `max_questions`: zero asks nothing.
const MAX_QUESTIONS_RANGE: std::ops::RangeInclusive<u32> = 0..=20;
/// `phase_minutes`: from five minutes to a day.
const PHASE_MINUTES_RANGE: std::ops::RangeInclusive<u32> = 5..=1440;

/// `[orchestrator.design]`. `config::Orchestrator.design`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignConfig {
    /// The mode for a goal DF §1's table puts on; a goal it puts off is always `Off`.
    pub default: DesignMode,
    /// Where the approved spec and plan are committed, relative to the repository.
    /// Empty: never commit; the documents stay in the data folder only.
    pub docs_dir: String,
    pub commit_brainstorm: bool,
    pub max_questions: u32,
    /// Each orchestrator phase's wall-clock budget (DF §2.2).
    pub phase_minutes: u32,
    pub budget: DesignBudget,
}

/// `[orchestrator.design.budget]`: each design agent's budget. `minutes` is its wall
/// clock and `tool_calls` its calls, as a task's budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignBudget {
    pub brainstormer: Budget,
    pub doc_reviewer: Budget,
}

impl Default for DesignConfig {
    fn default() -> Self {
        DesignConfig {
            default: DesignMode::Full,
            docs_dir: "docs/anthrex".to_string(),
            commit_brainstorm: false,
            max_questions: 5,
            phase_minutes: 60,
            budget: DesignBudget::default(),
        }
    }
}

impl Default for DesignBudget {
    fn default() -> Self {
        let budget = |tool_calls, minutes| Budget {
            tool_calls,
            minutes,
            tokens: None,
        };
        DesignBudget {
            brainstormer: budget(40, 15),
            doc_reviewer: budget(20, 10),
        }
    }
}

/// Reads `[orchestrator.design]` out of `[orchestrator]` (`orchestrator`). Never fails.
pub(super) fn read(orchestrator: &toml::Table, problems: &mut Vec<Problem>) -> DesignConfig {
    let mut d = DesignConfig::default();
    let Some(value) = orchestrator.get("design") else {
        return d;
    };
    let Some(t) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.design"));
        return d;
    };
    read_default(t, &mut d, problems);
    read_docs_dir(t, &mut d, problems);
    let key = |k: &str| format!("orchestrator.design.{k}");
    read_bool_key(
        t,
        "commit_brainstorm",
        &key("commit_brainstorm"),
        &mut d.commit_brainstorm,
        problems,
    );
    let ranged = [
        ("max_questions", MAX_QUESTIONS_RANGE, &mut d.max_questions),
        ("phase_minutes", PHASE_MINUTES_RANGE, &mut d.phase_minutes),
    ];
    for (k, range, field) in ranged {
        read_u32_in_range(t, k, &key(k), &range, field, problems);
    }
    match t.get("budget").map(|v| v.as_table()) {
        Some(Some(budget)) => {
            let prefix = "orchestrator.design.budget";
            for (k, field) in [
                ("brainstormer", &mut d.budget.brainstormer),
                ("doc_reviewer", &mut d.budget.doc_reviewer),
            ] {
                read_budget(budget, k, &format!("{prefix}.{k}"), field, problems);
            }
        }
        Some(None) => problems.push(not_a_table_problem("orchestrator.design.budget")),
        None => {}
    }
    d
}

fn read_default(t: &toml::Table, d: &mut DesignConfig, problems: &mut Vec<Problem>) {
    let Some(value) = t.get("default") else {
        return;
    };
    match value.as_str() {
        Some("full") => d.default = DesignMode::Full,
        Some("off") => d.default = DesignMode::Off,
        _ => problems.push(Problem {
            key: "orchestrator.design.default".to_string(),
            message: "must be \"full\" or \"off\"".to_string(),
            default: "\"full\"".to_string(),
        }),
    }
}

/// `docs_dir`: `""`, or a relative path whose every component is a plain name, so the
/// documents commit (decision 24) writes inside the repository.
fn read_docs_dir(t: &toml::Table, d: &mut DesignConfig, problems: &mut Vec<Problem>) {
    let Some(value) = t.get("docs_dir") else {
        return;
    };
    match value.as_str().filter(|s| inside_the_repository(s)) {
        Some(s) => d.docs_dir = s.to_string(),
        None => problems.push(Problem {
            key: "orchestrator.design.docs_dir".to_string(),
            message: "must be a relative path inside the repository, or \"\"".to_string(),
            default: format!("{:?}", d.docs_dir),
        }),
    }
}

fn inside_the_repository(dir: &str) -> bool {
    dir.is_empty()
        || (!dir.starts_with('/')
            && !dir.contains('\\')
            && dir
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}

/// `[orchestrator.design]`'s unknown keys, its budget table's, and each budget's.
pub(crate) fn report_unknown_design(value: &toml::Value, problems: &mut Vec<Problem>) {
    report_unknown_nested(value, "orchestrator.design", KNOWN_DESIGN_KEYS, problems);
    let Some(budget) = value.get("budget") else {
        return;
    };
    let prefix = "orchestrator.design.budget";
    report_unknown_nested(budget, prefix, KNOWN_DESIGN_BUDGET_KEYS, problems);
    for (name, sub) in budget.as_table().into_iter().flatten() {
        if KNOWN_DESIGN_BUDGET_KEYS.contains(&name.as_str()) {
            let prefix = format!("{prefix}.{name}");
            report_unknown_nested(sub, &prefix, KNOWN_BUDGET_KEYS, problems);
        }
    }
}

#[cfg(test)]
#[path = "design_tests.rs"]
mod tests;
