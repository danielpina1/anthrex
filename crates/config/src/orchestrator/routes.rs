//! `[orchestrator.routes.<name>]`, the user's model lists (milestone 9.5 decision 9a):
//! one ordered list of candidates and a `pick` per task class (`s`, `m`, `hub`) and per
//! role (`review`, `scout`, `decider`, `planner`, `orchestrator`, and milestone 9.6's
//! `brainstorm`). Milestone 9.8 (task M9.8.13): no run uses them; they are read only so
//! `models::migrate` can turn them into role-table rows, and `tuning::read_tuning` reads
//! them for their problems alone. Unknown tables and keys are reported by
//! [`report_unknown_routes`], which `orchestrator/unknown.rs` calls for `routes`.
//!
//! A candidate outside the merged roster, or with a bad `runtime`, `effort` or an empty
//! `model`, is dropped with a [`Problem`] and the rest of the list is kept. An absent
//! table, or one whose list is empty after validation, leaves that class or role as
//! it was before 9.5.

use proto::{Effort, ModelEntry, Runtime};

use crate::{Problem, not_a_table_problem, report_unknown_nested, unknown_key_problem};

/// The tables `[orchestrator.routes]` may hold, in [`RouteLists`]' field order.
const KNOWN_ROUTE_TABLES: &[&str] = &[
    "s",
    "m",
    "hub",
    "review",
    "scout",
    "decider",
    "planner",
    "orchestrator",
    "brainstorm",
];
const KNOWN_ROUTE_KEYS: &[&str] = &["candidates", "pick"];
const KNOWN_CANDIDATE_KEYS: &[&str] = &["runtime", "model", "effort"];

/// One entry of a model list. `effort` `None` takes the class's or role's default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub runtime: Runtime,
    pub model: String,
    pub effort: Option<Effort>,
}

/// How a list hands out its candidates: the first unskipped one, or round-robin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Pick {
    #[default]
    First,
    Spread,
}

/// One `[orchestrator.routes.<name>]` table, validated. Empty means "no list".
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RouteList {
    pub candidates: Vec<Candidate>,
    pub pick: Pick,
}

/// Every `[orchestrator.routes.<name>]` table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct RouteLists {
    pub s: RouteList,
    pub m: RouteList,
    pub hub: RouteList,
    pub review: RouteList,
    pub scout: RouteList,
    pub decider: RouteList,
    pub planner: RouteList,
    pub orchestrator: RouteList,
    /// Milestone 9.6 decision 10: the brainstormers' list (its first two entries on
    /// different runtimes, else its first two).
    pub brainstorm: RouteList,
}

/// 9.5 decision 9a: `[orchestrator.routes.<name>]` tables, their keys, and each
/// candidate's keys, keyed by the candidate's index.
pub(crate) fn report_unknown_routes(value: &toml::Value, problems: &mut Vec<Problem>) {
    for (name, sub) in value.as_table().into_iter().flatten() {
        let prefix = format!("orchestrator.routes.{name}");
        if !KNOWN_ROUTE_TABLES.contains(&name.as_str()) {
            problems.push(unknown_key_problem(&prefix));
            continue;
        }
        report_unknown_nested(sub, &prefix, KNOWN_ROUTE_KEYS, problems);
        let entries = sub.get("candidates").and_then(|c| c.as_array());
        for (i, entry) in entries.into_iter().flatten().enumerate() {
            let key = format!("{prefix}.candidates[{i}]");
            report_unknown_nested(entry, &key, KNOWN_CANDIDATE_KEYS, problems);
        }
    }
}

/// Reads `[orchestrator.routes]` out of `[orchestrator]` (`orchestrator`), checking
/// every candidate against the merged `roster`. Never fails. Also read by
/// `models::migrate` (M9.8 preflight ruling F4).
pub(crate) fn read_routes(
    orchestrator: &toml::Table,
    roster: &[ModelEntry],
    problems: &mut Vec<Problem>,
) -> RouteLists {
    let mut lists = RouteLists::default();
    let Some(value) = orchestrator.get("routes") else {
        return lists;
    };
    let Some(routes) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.routes"));
        return lists;
    };
    let RouteLists {
        s,
        m,
        hub,
        review,
        scout,
        decider,
        planner,
        orchestrator,
        brainstorm,
    } = &mut lists;
    let fields = [
        s,
        m,
        hub,
        review,
        scout,
        decider,
        planner,
        orchestrator,
        brainstorm,
    ];
    for (name, list) in KNOWN_ROUTE_TABLES.iter().zip(fields) {
        let Some(value) = routes.get(*name) else {
            continue;
        };
        let prefix = format!("orchestrator.routes.{name}");
        match value.as_table() {
            Some(t) => *list = read_list(t, &prefix, roster, problems),
            None => problems.push(not_a_table_problem(&prefix)),
        }
    }
    lists
}

fn read_list(
    t: &toml::Table,
    prefix: &str,
    roster: &[ModelEntry],
    problems: &mut Vec<Problem>,
) -> RouteList {
    let mut list = RouteList::default();
    match t.get("candidates").map(|v| v.as_array()) {
        Some(Some(entries)) => {
            for (i, entry) in entries.iter().enumerate() {
                let key = format!("{prefix}.candidates[{i}]");
                match read_candidate(entry, &key, roster) {
                    Ok(c) => list.candidates.push(c),
                    Err((key, message)) => problems.push(Problem {
                        key,
                        message,
                        default: crate::ENTRY_SKIPPED.to_string(),
                    }),
                }
            }
        }
        Some(None) => problems.push(Problem {
            key: format!("{prefix}.candidates"),
            message: "expected an array of tables".to_string(),
            default: "no list".to_string(),
        }),
        None => {}
    }
    match t.get("pick").map(|v| v.as_str()) {
        None | Some(Some("first")) => {}
        Some(Some("spread")) => list.pick = Pick::Spread,
        Some(_) => problems.push(Problem {
            key: format!("{prefix}.pick"),
            message: "must be first or spread".to_string(),
            default: "first".to_string(),
        }),
    }
    list
}

/// One candidate, or the key and message of the [`Problem`] that drops it.
fn read_candidate(
    entry: &toml::Value,
    key: &str,
    roster: &[ModelEntry],
) -> Result<Candidate, (String, String)> {
    let fail = |field: &str, message: &str| Err((format!("{key}{field}"), message.to_string()));
    let Some(t) = entry.as_table() else {
        return fail("", "expected a table");
    };
    let runtime = match t.get("runtime").and_then(|v| v.as_str()) {
        Some("claude") => Runtime::Claude,
        Some("codex") => Runtime::Codex,
        _ => return fail(".runtime", "must be claude or codex"),
    };
    let model = match t.get("model").and_then(|v| v.as_str()) {
        Some("") => return fail(".model", "must not be empty"),
        Some(m) => m.to_string(),
        None if t.contains_key("model") => return fail(".model", "must be a string"),
        None => return fail(".model", "is required"),
    };
    let effort = match t.get("effort").map(|v| v.as_str()) {
        None => None,
        Some(Some("low")) => Some(Effort::LOW),
        Some(Some("medium")) => Some(Effort::MEDIUM),
        Some(Some("high")) => Some(Effort::HIGH),
        Some(_) => return fail(".effort", "must be low, medium or high"),
    };
    if !roster
        .iter()
        .any(|e| e.runtime == runtime && e.model == model)
    {
        return fail("", &format!("{runtime}/{model} is not in the roster"));
    }
    Ok(Candidate {
        runtime,
        model,
        effort,
    })
}
