//! The `[orchestrator]` table (milestone 8a): run limits, budgets, timings, the
//! headless-session policy for Claude and Codex workers, the model roster (decision 23)
//! and the profile defaults (decision 56). Split out of `lib.rs`, following M6.5's
//! `conversation.rs` and `git.rs`, to keep every file under the 600-line rule; the
//! roster gets its own file, `orchestrator/roster.rs`, since `[[orchestrator.models]]`
//! parsing and the built-in roster are a responsibility of their own.
//!
//! [`read`] takes the *whole* top-level table, not just `[orchestrator]`, because it
//! must read the dotted key `review.small` (spec §9), and the `toml` crate folds
//! `[orchestrator]\nreview.small = "off"` and `[orchestrator.review]\nsmall = "off"`
//! into the exact same nested table once parsed -- the two forms are indistinguishable
//! after parsing, so one code path (`read_review`, below) handles both (decision 35,
//! Ruling Q2 of the brief refresh).
//!
//! Parsing rules follow decision 5, as [`crate::parse`] does for every other table:
//! each key is validated on its own, an invalid value is a [`Problem`] and the field
//! keeps its default, and an unknown key is `unknown key, ignored`.

use std::str::FromStr;

use super::*;

mod profile;
mod roster;

use profile::{read_profile, report_unknown_profile};
pub use roster::default_roster;
use roster::read_models;

/// The `[orchestrator]` table's built-in list for `worker_allowed_tools`.
const WORKER_ALLOWED_TOOLS_DEFAULT: &[&str] = &[
    "Bash",
    "Edit",
    "Write",
    "Read",
    "Glob",
    "Grep",
    "Agent",
    "TodoWrite",
];
const WORKER_PERMISSION_MODES: &[&str] =
    &["default", "acceptEdits", "dontAsk", "bypassPermissions"];
const CODEX_SANDBOX_MODES: &[&str] = &["read-only", "workspace-write", "danger-full-access"];
const KNOWN_BUDGET_RUNG_KEYS: &[&str] = &["tool_calls", "minutes", "tokens"];
const KNOWN_CLAUDE_KEYS: &[&str] = &["auth", "api_key_helper"];
const KNOWN_REVIEW_KEYS: &[&str] = &["small"];

/// The `[orchestrator]` table. Every field of the Interfaces block in
/// `docs/milestones/M8a-orchestration-engine-core.md`.
#[derive(Debug, Clone, PartialEq)]
pub struct Orchestrator {
    pub max_writers: u8,
    pub max_readers: u8,
    pub max_bounces: u8,
    pub max_tasks: u32,
    pub max_windows: u32,
    pub default_runtime: proto::Runtime,
    pub review_small: bool,
    pub budget_s: proto::Budget,
    pub budget_m: proto::Budget,
    pub budget_l: proto::Budget,
    pub stall_after_secs: u64,
    pub rate_limit_retry_secs: u64,
    pub denials_before_block: u32,
    pub git_timeout_secs: u64,
    pub worker_permission_mode: String,
    pub worker_allowed_tools: Vec<String>,
    pub worker_codex_sandbox: String,
    pub worker_sandbox: bool,
    /// M8a final fix batch F1c round 2: allow checks, proofs and `setup` to run
    /// unconfined where the platform cannot confine them (as `run start
    /// --unconfined-checks`). The user's own config only: a plan cannot set it.
    pub unconfined_checks: bool,
    /// M8a final fix batch F1c round 3 (N3): `[orchestrator.cache_dirs]`, a table
    /// keyed by repository root of the directories a confined check, proof or `setup`
    /// in that repository may also write. The user's own config only.
    pub cache_dirs: std::collections::BTreeMap<String, Vec<String>>,
    /// M8a final fix batch F1d (R4): `[orchestrator.confined_network]`, keyed by
    /// repository root: whether confined checks there have the network. The user's own
    /// config only.
    pub confined_network: std::collections::BTreeMap<String, bool>,
    /// F1d round 2 (S1): `[orchestrator.confined_unix_sockets]` and
    /// `[orchestrator.confined_localhost_ports]`, keyed by repository root.
    pub confined_unix_sockets: std::collections::BTreeMap<String, Vec<String>>,
    pub confined_localhost_ports: std::collections::BTreeMap<String, Vec<u16>>,
    pub claude: ClaudeHeadless,
    pub builtin_models: bool,
    pub models: Vec<proto::ModelEntry>,
    pub profile: proto::ProfileSpec,
}

/// `[orchestrator.claude] auth`, decision 50: whether a headless Claude session reads
/// the user's subscription login (the default) or `--bare`s in on an API key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClaudeAuth {
    #[default]
    Login,
    ApiKey,
}

/// `[orchestrator.claude]`, decision 50.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClaudeHeadless {
    pub auth: ClaudeAuth,
    pub api_key_helper: Option<String>,
}

impl Default for Orchestrator {
    fn default() -> Self {
        Orchestrator {
            max_writers: 3,
            max_readers: 3,
            max_bounces: 2,
            max_tasks: 50,
            max_windows: 60,
            default_runtime: proto::Runtime::Claude,
            review_small: true,
            budget_s: proto::Budget {
                tool_calls: 40,
                minutes: 15,
                tokens: None,
            },
            budget_m: proto::Budget {
                tool_calls: 150,
                minutes: 60,
                tokens: None,
            },
            budget_l: proto::Budget {
                tool_calls: 300,
                minutes: 120,
                tokens: None,
            },
            stall_after_secs: 600,
            rate_limit_retry_secs: 300,
            denials_before_block: 3,
            git_timeout_secs: 60,
            worker_permission_mode: "acceptEdits".to_string(),
            worker_allowed_tools: WORKER_ALLOWED_TOOLS_DEFAULT
                .iter()
                .map(|s| s.to_string())
                .collect(),
            worker_codex_sandbox: "workspace-write".to_string(),
            worker_sandbox: true,
            unconfined_checks: false,
            cache_dirs: std::collections::BTreeMap::new(),
            confined_network: std::collections::BTreeMap::new(),
            confined_unix_sockets: std::collections::BTreeMap::new(),
            confined_localhost_ports: std::collections::BTreeMap::new(),
            claude: ClaudeHeadless::default(),
            builtin_models: true,
            models: default_roster(),
            profile: proto::ProfileSpec::default(),
        }
    }
}

/// Parses `[orchestrator]` out of the whole config table. Never fails: an invalid or
/// missing value leaves the corresponding field at its default and pushes a
/// [`Problem`]. Unknown keys are reported separately, by [`report_unknown`], which
/// `report_unknown_keys` (`lib.rs`) calls for the `"orchestrator"` key.
pub(crate) fn read(table: &toml::Table, problems: &mut Vec<Problem>) -> Orchestrator {
    let mut o = Orchestrator::default();
    let Some(value) = table.get("orchestrator") else {
        return o;
    };
    let Some(t) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator"));
        return o;
    };

    read_u8_in_range(
        t,
        "max_writers",
        "orchestrator.max_writers",
        &(1..=8),
        &mut o.max_writers,
        problems,
    );
    read_u8_in_range(
        t,
        "max_readers",
        "orchestrator.max_readers",
        &(1..=8),
        &mut o.max_readers,
        problems,
    );
    read_u8_in_range(
        t,
        "max_bounces",
        "orchestrator.max_bounces",
        &(1..=5),
        &mut o.max_bounces,
        problems,
    );
    read_u32_in_range(
        t,
        "max_tasks",
        "orchestrator.max_tasks",
        &(1..=200),
        &mut o.max_tasks,
        problems,
    );
    read_u32_in_range(
        t,
        "max_windows",
        "orchestrator.max_windows",
        &(1..=500),
        &mut o.max_windows,
        problems,
    );
    read_default_runtime(t, &mut o, problems);
    read_review(t, &mut o, problems);
    read_budgets(t, &mut o, problems);
    read_u64_in_range(
        t,
        "stall_after_secs",
        "orchestrator.stall_after_secs",
        &(5..=7200),
        &mut o.stall_after_secs,
        problems,
    );
    read_u64_in_range(
        t,
        "rate_limit_retry_secs",
        "orchestrator.rate_limit_retry_secs",
        &(5..=3600),
        &mut o.rate_limit_retry_secs,
        problems,
    );
    read_u32_in_range(
        t,
        "denials_before_block",
        "orchestrator.denials_before_block",
        &(1..=50),
        &mut o.denials_before_block,
        problems,
    );
    read_u64_in_range(
        t,
        "git_timeout_secs",
        "orchestrator.git_timeout_secs",
        &(5..=600),
        &mut o.git_timeout_secs,
        problems,
    );
    read_worker_permission_mode(t, &mut o, problems);
    read_worker_allowed_tools(t, &mut o, problems);
    read_worker_codex_sandbox(t, &mut o, problems);
    read_bool_key(
        t,
        "worker_sandbox",
        "orchestrator.worker_sandbox",
        &mut o.worker_sandbox,
        problems,
    );
    read_bool_key(
        t,
        "unconfined_checks",
        "orchestrator.unconfined_checks",
        &mut o.unconfined_checks,
        problems,
    );
    read_claude(t, &mut o, problems);
    profile::read_cache_dirs(t, &mut o, problems);
    profile::read_confined_network(t, &mut o, problems);
    profile::read_confined_unix_sockets(t, &mut o, problems);
    profile::read_confined_localhost_ports(t, &mut o, problems);
    read_bool_key(
        t,
        "builtin_models",
        "orchestrator.builtin_models",
        &mut o.builtin_models,
        problems,
    );
    read_models(t, &mut o, problems);
    read_profile(t, &mut o, problems);

    o
}

fn read_u8_in_range(
    table: &toml::Table,
    local_key: &str,
    full_key: &str,
    range: &std::ops::RangeInclusive<u8>,
    field: &mut u8,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(local_key) else {
        return;
    };
    match value
        .as_integer()
        .and_then(|n| u8::try_from(n).ok())
        .filter(|n| range.contains(n))
    {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: field.to_string(),
        }),
    }
}

fn read_u32_in_range(
    table: &toml::Table,
    local_key: &str,
    full_key: &str,
    range: &std::ops::RangeInclusive<u32>,
    field: &mut u32,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get(local_key) else {
        return;
    };
    match value
        .as_integer()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| range.contains(n))
    {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: full_key.to_string(),
            message: format!("must be between {} and {}", range.start(), range.end()),
            default: field.to_string(),
        }),
    }
}

fn read_default_runtime(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("default_runtime") else {
        return;
    };
    match value
        .as_str()
        .and_then(|s| proto::Runtime::from_str(s).ok())
        .filter(|r| matches!(r, proto::Runtime::Claude | proto::Runtime::Codex))
    {
        Some(r) => o.default_runtime = r,
        None => problems.push(Problem {
            key: "orchestrator.default_runtime".to_string(),
            message: "must be claude or codex".to_string(),
            default: o.default_runtime.to_string(),
        }),
    }
}

/// `review.small`, decision 35: `"on"` (true, the default) or `"off"`. Reads the
/// nested `review` table regardless of whether the source wrote a dotted key or
/// `[orchestrator.review]`; the parsed TOML is identical either way.
fn read_review(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("review") else {
        return;
    };
    let Some(review) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.review"));
        return;
    };
    let Some(small) = review.get("small") else {
        return;
    };
    match small.as_str() {
        Some("on") => o.review_small = true,
        Some("off") => o.review_small = false,
        _ => problems.push(Problem {
            key: "orchestrator.review.small".to_string(),
            message: "must be \"on\" or \"off\"".to_string(),
            default: "\"on\"".to_string(),
        }),
    }
}

fn read_budgets(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
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
            .filter(|n| *n >= 1)
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
            .filter(|n| *n >= 1)
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

fn read_worker_permission_mode(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("worker_permission_mode") else {
        return;
    };
    match value
        .as_str()
        .filter(|s| WORKER_PERMISSION_MODES.contains(s))
    {
        Some(s) => o.worker_permission_mode = s.to_string(),
        None => problems.push(Problem {
            key: "orchestrator.worker_permission_mode".to_string(),
            message: "must be default, acceptEdits, dontAsk or bypassPermissions".to_string(),
            default: o.worker_permission_mode.clone(),
        }),
    }
}

fn read_worker_allowed_tools(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("worker_allowed_tools") else {
        return;
    };
    let Some(array) = value.as_array() else {
        problems.push(Problem {
            key: "orchestrator.worker_allowed_tools".to_string(),
            message: "expected an array of strings".to_string(),
            default: "the built-in list".to_string(),
        });
        return;
    };

    let mut tools = Vec::new();
    for item in array {
        match item.as_str() {
            Some(s) => tools.push(s.to_string()),
            None => {
                problems.push(Problem {
                    key: "orchestrator.worker_allowed_tools".to_string(),
                    message: "expected an array of strings".to_string(),
                    default: "the built-in list".to_string(),
                });
                return;
            }
        }
    }
    if tools.is_empty() {
        problems.push(Problem {
            key: "orchestrator.worker_allowed_tools".to_string(),
            message: "must not be empty".to_string(),
            default: "the built-in list".to_string(),
        });
        return;
    }
    o.worker_allowed_tools = tools;
}

fn read_worker_codex_sandbox(
    table: &toml::Table,
    o: &mut Orchestrator,
    problems: &mut Vec<Problem>,
) {
    let Some(value) = table.get("worker_codex_sandbox") else {
        return;
    };
    match value.as_str().filter(|s| CODEX_SANDBOX_MODES.contains(s)) {
        Some(s) => o.worker_codex_sandbox = s.to_string(),
        None => problems.push(Problem {
            key: "orchestrator.worker_codex_sandbox".to_string(),
            message: "must be read-only, workspace-write or danger-full-access".to_string(),
            default: o.worker_codex_sandbox.clone(),
        }),
    }
}

fn read_claude(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    let Some(value) = table.get("claude") else {
        return;
    };
    let Some(claude) = value.as_table() else {
        problems.push(not_a_table_problem("orchestrator.claude"));
        return;
    };

    if let Some(v) = claude.get("auth") {
        match v.as_str() {
            Some("login") => o.claude.auth = ClaudeAuth::Login,
            Some("api_key") => o.claude.auth = ClaudeAuth::ApiKey,
            _ => problems.push(Problem {
                key: "orchestrator.claude.auth".to_string(),
                message: "must be \"login\" or \"api_key\"".to_string(),
                default: "login".to_string(),
            }),
        }
    }
    if let Some(v) = claude.get("api_key_helper") {
        match v.as_str() {
            Some(s) => o.claude.api_key_helper = Some(s.to_string()),
            None => problems.push(Problem {
                key: "orchestrator.claude.api_key_helper".to_string(),
                message: "expected a string".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
}

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
            | "confined_localhost_ports"
            | "models" => {}
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
            other => {
                report_unknown_nested(sub, &format!("orchestrator.budget.{other}"), &[], problems)
            }
        }
    }
}

#[cfg(test)]
#[path = "orchestrator_tests.rs"]
mod tests;
