//! The `[runtimes]` table: each runtime's `command` (with `~/` expanded) and codex's
//! `bypass_hook_trust`. Split out of `lib.rs` before milestone 9.1 to keep it under the
//! 600-line rule; a pure move.

use super::*;

/// A leading `~/` is expanded against `$HOME` when the config is read
/// (decision 6). A bare name, or a value that is not `~/...`, is left as-is.
fn expand_home(command: &str) -> String {
    if let Some(rest) = command.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return format!("{home}/{rest}");
    }
    command.to_string()
}

fn read_runtime_command(
    table: &toml::Table,
    full_key: &str,
    field: &mut String,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("command") else {
        return;
    };
    match value.as_str() {
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: "expected a string".to_string(),
            default: field.clone(),
        }),
        Some("") => problems.push(Problem {
            key: full_key.to_string(),
            message: "must not be empty".to_string(),
            default: field.clone(),
        }),
        Some(s) => *field = expand_home(s),
    }
}

pub(super) fn read_runtimes(table: &toml::Table, config: &mut Config, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("runtimes") else {
        return;
    };
    let Some(runtimes) = value.as_table() else {
        problems.push(not_a_table_problem("runtimes"));
        return;
    };

    if let Some(value) = runtimes.get("claude") {
        match value.as_table() {
            Some(claude) => read_runtime_command(
                claude,
                "runtimes.claude.command",
                &mut config.runtimes.claude.command,
                problems,
            ),
            None => problems.push(not_a_table_problem("runtimes.claude")),
        }
    }

    if let Some(value) = runtimes.get("codex") {
        match value.as_table() {
            Some(codex) => {
                read_runtime_command(
                    codex,
                    "runtimes.codex.command",
                    &mut config.runtimes.codex.command,
                    problems,
                );
                read_bool_key(
                    codex,
                    "bypass_hook_trust",
                    "runtimes.codex.bypass_hook_trust",
                    &mut config.runtimes.codex_bypass_hook_trust,
                    problems,
                );
            }
            None => problems.push(not_a_table_problem("runtimes.codex")),
        }
    }
}

pub(super) fn report_unknown_runtimes(value: &toml::Value, problems: &mut Vec<Problem>) {
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, sub) in table {
        match key.as_str() {
            "claude" => {
                report_unknown_nested(sub, "runtimes.claude", KNOWN_RUNTIME_COMMAND_KEYS, problems)
            }
            "codex" => {
                report_unknown_nested(sub, "runtimes.codex", KNOWN_RUNTIME_CODEX_KEYS, problems)
            }
            other => problems.push(unknown_key_problem(&format!("runtimes.{other}"))),
        }
    }
}
