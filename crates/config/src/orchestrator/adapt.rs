//! Milestone 8b's settings (decision 3): `[orchestrator] fast_path` and the
//! `[orchestrator.deciders]`, `[orchestrator.scouts]`, `[orchestrator.onboarding]` and
//! `[orchestrator.metering]` tables. Read by one call, [`read_adapt`], from
//! `orchestrator::read`; their unknown keys are reported by `orchestrator/unknown.rs`.
//!
//! As with `review.small`, the `toml` crate folds `[orchestrator]\ndeciders.mode = "off"`
//! and `[orchestrator.deciders]\nmode = "off"` into the same nested table, so one path
//! reads both. None of these tables names a path or a confinement setting.

use proto::{DeciderMode, Effort, Runtime, Strength};

use super::Orchestrator;
use crate::{Problem, not_a_table_problem, read_bool_key, read_u64_in_range};

pub(super) const KNOWN_DECIDERS_KEYS: &[&str] = &[
    "mode",
    "timeout_secs",
    "strength",
    "effort",
    "slot_wait_secs",
];
pub(super) const KNOWN_SCOUTS_KEYS: &[&str] = &[
    "runtime",
    "strength",
    "effort",
    "timeout_secs",
    "max_tool_calls",
];
pub(super) const KNOWN_ONBOARDING_KEYS: &[&str] = &["auto", "verify_timeout_secs"];
pub(super) const KNOWN_METERING_KEYS: &[&str] = &["otlp", "otlp_port"];

/// `[orchestrator.deciders]`: the one-shot headless calls (triage, size cross-check,
/// check summary, blocked reason).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deciders {
    pub mode: DeciderMode,
    pub timeout_secs: u64,
    pub strength: Strength,
    pub effort: Effort,
    pub slot_wait_secs: u64,
}

impl Default for Deciders {
    fn default() -> Self {
        Deciders {
            mode: DeciderMode::Claude,
            timeout_secs: 90,
            strength: Strength::Fast,
            effort: Effort::LOW,
            slot_wait_secs: 30,
        }
    }
}

/// `[orchestrator.scouts]`. `runtime` `None` means `orchestrator.default_runtime`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scouts {
    pub runtime: Option<Runtime>,
    pub strength: Strength,
    pub effort: Effort,
    pub timeout_secs: u64,
    pub max_tool_calls: u32,
}

impl Default for Scouts {
    fn default() -> Self {
        Scouts {
            runtime: None,
            strength: Strength::Fast,
            effort: Effort::LOW,
            timeout_secs: 900,
            max_tool_calls: 120,
        }
    }
}

/// `[orchestrator.onboarding]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Onboarding {
    pub auto: bool,
    pub verify_timeout_secs: u64,
}

impl Default for Onboarding {
    fn default() -> Self {
        Onboarding {
            auto: true,
            verify_timeout_secs: 1800,
        }
    }
}

/// `[orchestrator.metering]`. `otlp_port` 0 means any free port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metering {
    pub otlp: bool,
    pub otlp_port: u16,
}

impl Default for Metering {
    fn default() -> Self {
        Metering {
            otlp: true,
            otlp_port: 0,
        }
    }
}

/// Reads every M8b key of `[orchestrator]` (`table`) into `o`. Never fails: an invalid
/// value is a [`Problem`] and keeps the default.
pub(crate) fn read_adapt(table: &toml::Table, o: &mut Orchestrator, problems: &mut Vec<Problem>) {
    read_bool_key(
        table,
        "fast_path",
        "orchestrator.fast_path",
        &mut o.fast_path,
        problems,
    );
    if let Some(t) = sub_table(table, "deciders", problems) {
        read_deciders(t, &mut o.deciders, problems);
    }
    if let Some(t) = sub_table(table, "scouts", problems) {
        read_scouts(t, &mut o.scouts, problems);
    }
    if let Some(t) = sub_table(table, "onboarding", problems) {
        read_bool_key(
            t,
            "auto",
            "orchestrator.onboarding.auto",
            &mut o.onboarding.auto,
            problems,
        );
        read_u64_in_range(
            t,
            "verify_timeout_secs",
            "orchestrator.onboarding.verify_timeout_secs",
            &(10..=14400),
            &mut o.onboarding.verify_timeout_secs,
            problems,
        );
    }
    if let Some(t) = sub_table(table, "metering", problems) {
        read_bool_key(
            t,
            "otlp",
            "orchestrator.metering.otlp",
            &mut o.metering.otlp,
            problems,
        );
        read_otlp_port(t, &mut o.metering.otlp_port, problems);
    }
}

fn read_deciders(t: &toml::Table, d: &mut Deciders, problems: &mut Vec<Problem>) {
    if let Some(v) = t.get("mode") {
        match v.as_str() {
            Some("claude") => d.mode = DeciderMode::Claude,
            Some("codex") => d.mode = DeciderMode::Codex,
            Some("off") => d.mode = DeciderMode::Off,
            _ => problems.push(Problem {
                key: "orchestrator.deciders.mode".to_string(),
                message: "must be claude, codex or off".to_string(),
                default: mode_label(d.mode).to_string(),
            }),
        }
    }
    read_u64_in_range(
        t,
        "timeout_secs",
        "orchestrator.deciders.timeout_secs",
        &(5..=600),
        &mut d.timeout_secs,
        problems,
    );
    read_strength(t, "orchestrator.deciders", &mut d.strength, problems);
    read_effort(t, "orchestrator.deciders", &mut d.effort, problems);
    read_u64_in_range(
        t,
        "slot_wait_secs",
        "orchestrator.deciders.slot_wait_secs",
        &(0..=600),
        &mut d.slot_wait_secs,
        problems,
    );
}

fn read_scouts(t: &toml::Table, s: &mut Scouts, problems: &mut Vec<Problem>) {
    if let Some(v) = t.get("runtime") {
        match v.as_str() {
            Some("claude") => s.runtime = Some(Runtime::Claude),
            Some("codex") => s.runtime = Some(Runtime::Codex),
            _ => problems.push(Problem {
                key: "orchestrator.scouts.runtime".to_string(),
                message: "must be claude or codex".to_string(),
                default: match s.runtime {
                    Some(r) => r.to_string(),
                    None => "orchestrator.default_runtime".to_string(),
                },
            }),
        }
    }
    read_strength(t, "orchestrator.scouts", &mut s.strength, problems);
    read_effort(t, "orchestrator.scouts", &mut s.effort, problems);
    read_u64_in_range(
        t,
        "timeout_secs",
        "orchestrator.scouts.timeout_secs",
        &(60..=7200),
        &mut s.timeout_secs,
        problems,
    );
    if let Some(v) = t.get("max_tool_calls") {
        match v
            .as_integer()
            .and_then(|n| u32::try_from(n).ok())
            .filter(|n| (10..=1000).contains(n))
        {
            Some(n) => s.max_tool_calls = n,
            None => problems.push(Problem {
                key: "orchestrator.scouts.max_tool_calls".to_string(),
                message: "must be between 10 and 1000".to_string(),
                default: s.max_tool_calls.to_string(),
            }),
        }
    }
}

fn read_otlp_port(t: &toml::Table, field: &mut u16, problems: &mut Vec<Problem>) {
    let Some(v) = t.get("otlp_port") else {
        return;
    };
    match v.as_integer().and_then(|n| u16::try_from(n).ok()) {
        Some(n) => *field = n,
        None => problems.push(Problem {
            key: "orchestrator.metering.otlp_port".to_string(),
            message: "must be between 0 and 65535".to_string(),
            default: field.to_string(),
        }),
    }
}

pub(super) fn read_strength(
    t: &toml::Table,
    prefix: &str,
    field: &mut Strength,
    problems: &mut Vec<Problem>,
) {
    let Some(v) = t.get("strength") else {
        return;
    };
    match v.as_str() {
        Some("fast") => *field = Strength::Fast,
        Some("standard") => *field = Strength::Standard,
        Some("frontier") => *field = Strength::Frontier,
        _ => problems.push(Problem {
            key: format!("{prefix}.strength"),
            message: "must be fast, standard or frontier".to_string(),
            default: match field {
                Strength::Fast => "fast",
                Strength::Standard => "standard",
                Strength::Frontier => "frontier",
            }
            .to_string(),
        }),
    }
}

pub(super) fn read_effort(
    t: &toml::Table,
    prefix: &str,
    field: &mut Effort,
    problems: &mut Vec<Problem>,
) {
    let Some(v) = t.get("effort") else {
        return;
    };
    match v.as_str() {
        Some("low") => *field = Effort::LOW,
        Some("medium") => *field = Effort::MEDIUM,
        Some("high") => *field = Effort::HIGH,
        _ => problems.push(Problem {
            key: format!("{prefix}.effort"),
            message: "must be low, medium or high".to_string(),
            default: match field.as_str() {
                "low" => "low",
                "medium" => "medium",
                _ => "high",
            }
            .to_string(),
        }),
    }
}

fn mode_label(mode: DeciderMode) -> &'static str {
    match mode {
        DeciderMode::Claude => "claude",
        DeciderMode::Codex => "codex",
        DeciderMode::Off => "off",
    }
}

/// `[orchestrator.<key>]` as a table; a problem, and `None`, when it is something else.
pub(super) fn sub_table<'a>(
    table: &'a toml::Table,
    key: &str,
    problems: &mut Vec<Problem>,
) -> Option<&'a toml::Table> {
    let value = table.get(key)?;
    let t = value.as_table();
    if t.is_none() {
        problems.push(not_a_table_problem(&format!("orchestrator.{key}")));
    }
    t
}
