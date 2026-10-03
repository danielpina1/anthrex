//! `[orchestrator.tuning]` (milestone 9.5 decision 3, ruling RH-7): how the refit learns
//! from history, and the adaptive-concurrency settings. Also the model lists
//! (`orchestrator/routes.rs`, decision 9a) and whether `[orchestrator.budget.s]` and
//! `[orchestrator.budget.m]` set a value explicitly (ruling RH-5: an explicit budget
//! beats the refit). Read by one call, [`read_tuning`], from `orchestrator::read`,
//! after the roster; unknown keys are reported by `orchestrator/unknown.rs`.

use proto::ModelEntry;

use super::read_u32_in_range;
use crate::{Problem, not_a_table_problem, read_bool_key, read_u64_in_range};

#[path = "routes.rs"]
mod routes;
use routes::RouteLists;
pub(crate) use routes::report_unknown_routes;

pub(crate) const KNOWN_TUNING_KEYS: &[&str] = &[
    "min_samples",
    "per_run_cap",
    "window",
    "budget_factor_percent",
    "threshold_percentile",
    "min_change_percent",
    "escalate_above_percent",
    "refit_budgets",
    "refit_tokens",
    "path_weights",
    "adaptive_concurrency",
    "recover_after_mins",
    "halve_hold_secs",
    "race_slot_wait_secs",
];

/// `[orchestrator.tuning]`. Ranges are inclusive; an out-of-range value keeps the
/// default with a [`Problem`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tuning {
    /// 5..=1000: the samples a class needs before anything is refitted or proposed.
    pub min_samples: u32,
    /// 1..=1000: the most samples one run's round may supply.
    pub per_run_cap: u32,
    /// 10..=5000: the newest samples kept; never below `min_samples`.
    pub window: u32,
    /// 200..=300: a refit budget is the median spend times this, over 100.
    pub budget_factor_percent: u32,
    /// 50..=99: the percentile of changed lines a threshold proposal takes.
    pub threshold_percentile: u32,
    /// 1..=100: the change, in percent, below which nothing is rewritten or proposed.
    pub min_change_percent: u32,
    /// 1..=100: the escalated share above which a class's route steps up.
    pub escalate_above_percent: u32,
    pub refit_budgets: bool,
    pub refit_tokens: bool,
    pub path_weights: bool,
    pub adaptive_concurrency: bool,
    /// 1..=240: minutes without a rate limit or cap change before a lowered writer cap
    /// steps back up by one.
    pub recover_after_mins: u64,
    /// 0..=3600: after a halving, further rate limits do not halve again for this long.
    pub halve_hold_secs: u64,
    /// 0..=3600: how long a race waits for its second writer slot.
    pub race_slot_wait_secs: u64,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            min_samples: 30,
            per_run_cap: 10,
            window: 200,
            budget_factor_percent: 250,
            threshold_percentile: 90,
            min_change_percent: 20,
            escalate_above_percent: 25,
            refit_budgets: true,
            refit_tokens: false,
            path_weights: true,
            adaptive_concurrency: true,
            recover_after_mins: 10,
            halve_hold_secs: 60,
            race_slot_wait_secs: 120,
        }
    }
}

/// Whether config sets `[orchestrator.budget.s]` and `[orchestrator.budget.m]`
/// explicitly (ruling RH-5): such a class keeps its configured budget, and its refit is
/// only shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ConfiguredBudgets {
    pub s: bool,
    pub m: bool,
}

/// Everything 9.5 reads from `[orchestrator]`: `config::Orchestrator.tuning`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TuningConfig {
    pub table: Tuning,
    pub routes: RouteLists,
    pub configured: ConfiguredBudgets,
}

/// Reads `[orchestrator.tuning]`, `[orchestrator.routes.*]` and the explicit budgets
/// out of `[orchestrator]` (`orchestrator`). `roster` is the merged roster, already
/// read. Never fails.
pub(crate) fn read_tuning(
    orchestrator: &toml::Table,
    roster: &[ModelEntry],
    problems: &mut Vec<Problem>,
) -> TuningConfig {
    let mut table = Tuning::default();
    match orchestrator.get("tuning").map(|v| v.as_table()) {
        Some(Some(t)) => read_table(t, &mut table, problems),
        Some(None) => problems.push(not_a_table_problem("orchestrator.tuning")),
        None => {}
    }
    TuningConfig {
        table,
        routes: routes::read_routes(orchestrator, roster, problems),
        configured: configured_budgets(orchestrator),
    }
}

fn read_table(t: &toml::Table, o: &mut Tuning, problems: &mut Vec<Problem>) {
    let u32_keys: [(&str, std::ops::RangeInclusive<u32>, &mut u32); 7] = [
        ("min_samples", 5..=1000, &mut o.min_samples),
        ("per_run_cap", 1..=1000, &mut o.per_run_cap),
        ("window", 10..=5000, &mut o.window),
        (
            "budget_factor_percent",
            200..=300,
            &mut o.budget_factor_percent,
        ),
        ("threshold_percentile", 50..=99, &mut o.threshold_percentile),
        ("min_change_percent", 1..=100, &mut o.min_change_percent),
        (
            "escalate_above_percent",
            1..=100,
            &mut o.escalate_above_percent,
        ),
    ];
    for (key, range, field) in u32_keys {
        let full = format!("orchestrator.tuning.{key}");
        read_u32_in_range(t, key, &full, &range, field, problems);
    }
    let bool_keys: [(&str, &mut bool); 4] = [
        ("refit_budgets", &mut o.refit_budgets),
        ("refit_tokens", &mut o.refit_tokens),
        ("path_weights", &mut o.path_weights),
        ("adaptive_concurrency", &mut o.adaptive_concurrency),
    ];
    for (key, field) in bool_keys {
        read_bool_key(
            t,
            key,
            &format!("orchestrator.tuning.{key}"),
            field,
            problems,
        );
    }
    let u64_keys: [(&str, std::ops::RangeInclusive<u64>, &mut u64); 3] = [
        ("recover_after_mins", 1..=240, &mut o.recover_after_mins),
        ("halve_hold_secs", 0..=3600, &mut o.halve_hold_secs),
        ("race_slot_wait_secs", 0..=3600, &mut o.race_slot_wait_secs),
    ];
    for (key, range, field) in u64_keys {
        let full = format!("orchestrator.tuning.{key}");
        read_u64_in_range(t, key, &full, &range, field, problems);
    }
    // The window keeps the newest samples; one smaller than `min_samples` could never
    // let a class qualify.
    if o.window < o.min_samples {
        o.window = o.min_samples;
        problems.push(Problem {
            key: "orchestrator.tuning.window".to_string(),
            message: format!("must be at least min_samples ({})", o.min_samples),
            default: o.min_samples.to_string(),
        });
    }
}

/// Ruling RH-5: a class's budget is explicit when its table sets at least one value.
/// An empty `[orchestrator.budget.s]`, as the Settings screen leaves behind when it
/// deletes a value set back to its default, is not.
fn configured_budgets(orchestrator: &toml::Table) -> ConfiguredBudgets {
    let explicit = |rung: &str| {
        orchestrator
            .get("budget")
            .and_then(|b| b.get(rung))
            .and_then(|r| r.as_table())
            .is_some_and(|r| {
                ["tool_calls", "minutes", "tokens"]
                    .iter()
                    .any(|k| r.contains_key(*k))
            })
    };
    ConfiguredBudgets {
        s: explicit("s"),
        m: explicit("m"),
    }
}

#[cfg(test)]
#[path = "tuning_tests.rs"]
mod tests;
