//! `[orchestrator]`'s unknown keys, each reported as `unknown key, ignored` (split out
//! of `orchestrator.rs` to keep it under the 600-line rule, F4).

use super::report_unknown_profile;
use crate::{Problem, report_unknown_nested, unknown_key_problem};

const KNOWN_BUDGET_RUNG_KEYS: &[&str] = &["tool_calls", "minutes", "tokens"];
const KNOWN_CLAUDE_KEYS: &[&str] = &["auth", "api_key_helper"];
const KNOWN_REVIEW_KEYS: &[&str] = &["small"];

/// Reports `[orchestrator]`'s unknown keys. Called from `report_unknown_keys`
/// (`lib.rs`) for the `"orchestrator"` key, the way `report_unknown_conversation` is
/// called for `"conversation"`.
pub(crate) fn report_unknown(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        match key.as_str() {
            "max_writers"
            | "max_readers"
            | "max_bounces"
            | "max_tasks"
            | "max_windows"
            | "default_runtime"
            | "stall_after_secs"
            | "rate_limit_retry_secs"
            | "denials_before_block"
            | "git_timeout_secs"
            | "worker_permission_mode"
            | "worker_allowed_tools"
            | "worker_codex_sandbox"
            | "worker_sandbox"
            | "unconfined_checks"
            | "builtin_models"
            | "cache_dirs"
            | "confined_network"
            | "confined_unix_sockets"
            | "confined_localhost_ports" => {}
            "models" => report_unknown_model_keys(sub, problems),
            "review" => {
                report_unknown_nested(sub, "orchestrator.review", KNOWN_REVIEW_KEYS, problems)
            }
            "budget" => report_unknown_budget(sub, problems),
            "claude" => {
                report_unknown_nested(sub, "orchestrator.claude", KNOWN_CLAUDE_KEYS, problems)
            }
            "profile" => report_unknown_profile(sub, problems),
            other => problems.push(unknown_key_problem(&format!("orchestrator.{other}"))),
        }
    }
}

fn report_unknown_budget(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        match key.as_str() {
            "s" | "m" | "l" => report_unknown_nested(
                sub,
                &format!("orchestrator.budget.{key}"),
                KNOWN_BUDGET_RUNG_KEYS,
                problems,
            ),
            // Review E-M5 (F4): the rung itself, a scalar or an empty table too.
            other => problems.push(unknown_key_problem(&format!("orchestrator.budget.{other}"))),
        }
    }
}

/// Review E-M5 (F4): each `[[orchestrator.models]]` entry's keys outside the four it
/// has, keyed by the entry's index. A malformed entry is reported by the roster parse.
fn report_unknown_model_keys(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(entries) = value.as_array() else {
        return;
    };
    for (i, entry) in entries.iter().enumerate() {
        let Some(table) = entry.as_table() else {
            continue;
        };
        for key in table.keys() {
            if !matches!(key.as_str(), "runtime" | "model" | "strength" | "note") {
                problems.push(unknown_key_problem(&format!(
                    "orchestrator.models[{i}].{key}"
                )));
            }
        }
    }
}
