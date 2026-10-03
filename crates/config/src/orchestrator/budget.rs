//! `[orchestrator.budget.s]`, `.m` and `.l` (decision 40's budgets). Split out of
//! `orchestrator.rs` to keep that file under the 600-line rule.

use super::Orchestrator;
use crate::{Problem, not_a_table_problem};

pub(super) fn read_budgets(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("budget") else {
        return;
    };
    let Some(budget) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.budget"));
        return;
    };
    read_one_budget(budget, "s", &mut o.budget_s, problems);
    read_one_budget(budget, "m", &mut o.budget_m, problems);
    read_one_budget(budget, "l", &mut o.budget_l, problems);
}

fn read_one_budget(
    budget: &toml::Table,
    rung: &str,
    field: &mut proto::Budget,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = budget.get(rung) else {
        return;
    };
    let Some(t) = value.as_table() else {
        problems.push(not_a_table_problem(&format!("orchestrator.budget.{rung}")));
        return;
    };

    if let Some(v) = t.get("tool_calls") {
        match v
            .as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n >= proto::settings::BUDGET_MIN)
        {
            Some(n) => field.tool_calls = n,
            None => problems.push(Problem {
                key: format!("orchestrator.budget.{rung}.tool_calls"),
                message: "must be at least 1".to_string(),
                default: field.tool_calls.to_string(),
            }),
        }
    }
    if let Some(v) = t.get("minutes") {
        match v
            .as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| *n >= proto::settings::BUDGET_MIN)
        {
            Some(n) => field.minutes = n,
            None => problems.push(Problem {
                key: format!("orchestrator.budget.{rung}.minutes"),
                message: "must be at least 1".to_string(),
                default: field.minutes.to_string(),
            }),
        }
    }
    if let Some(v) = t.get("tokens") {
        match v
            .as_integer()
            .and_then(|n| u64::try_from(n).ok())
            .filter(|n| *n >= 1)
        {
            Some(n) => field.tokens = Some(n),
            None => problems.push(Problem {
                key: format!("orchestrator.budget.{rung}.tokens"),
                message: "must be at least 1".to_string(),
                default: match field.tokens {
                    Some(n) => n.to_string(),
                    None => "unset".to_string(),
                },
            }),
        }
    }
}
