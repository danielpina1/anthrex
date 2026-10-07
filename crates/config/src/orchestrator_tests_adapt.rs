//! M8b.3: `[orchestrator] fast_path` and the `[orchestrator.deciders]`,
//! `[orchestrator.scouts]`, `[orchestrator.onboarding]` and `[orchestrator.metering]`
//! tables (M8b decision 3, Interfaces "`config`").

use super::*;
use proto::{DeciderMode, Effort, Runtime, Strength};

#[test]
fn adapt_defaults_when_absent() {
    let (config, problems) = parse("");
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    assert!(o.fast_path);
    assert_eq!(
        o.deciders,
        Deciders {
            mode: DeciderMode::Claude,
            timeout_secs: 90,
            strength: Strength::Fast,
            effort: Effort::Low,
            slot_wait_secs: 30,
        }
    );
    assert_eq!(
        o.scouts,
        Scouts {
            runtime: None,
            strength: Strength::Fast,
            effort: Effort::Low,
            timeout_secs: 900,
            max_tool_calls: 120,
        }
    );
    assert_eq!(
        o.onboarding,
        Onboarding {
            auto: true,
            verify_timeout_secs: 1800,
        }
    );
    assert_eq!(
        o.metering,
        Metering {
            otlp: true,
            otlp_port: 0,
        }
    );
    // An empty `[orchestrator]` table gives the same defaults.
    let (config, problems) = parse("[orchestrator]\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.orchestrator, Orchestrator::default());
}

#[test]
fn adapt_keys_are_read() {
    let (config, problems) = parse(
        r#"
[orchestrator]
fast_path = false

[orchestrator.deciders]
mode = "codex"
timeout_secs = 120
strength = "standard"
effort = "medium"
slot_wait_secs = 0

[orchestrator.scouts]
runtime = "codex"
strength = "frontier"
effort = "high"
timeout_secs = 60
max_tool_calls = 1000

[orchestrator.onboarding]
auto = false
verify_timeout_secs = 14400

[orchestrator.metering]
otlp = false
otlp_port = 4318
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let o = &config.orchestrator;
    assert!(!o.fast_path);
    assert_eq!(
        o.deciders,
        Deciders {
            mode: DeciderMode::Codex,
            timeout_secs: 120,
            strength: Strength::Standard,
            effort: Effort::Medium,
            slot_wait_secs: 0,
        }
    );
    assert_eq!(
        o.scouts,
        Scouts {
            runtime: Some(Runtime::Codex),
            strength: Strength::Frontier,
            effort: Effort::High,
            timeout_secs: 60,
            max_tool_calls: 1000,
        }
    );
    assert_eq!(
        o.onboarding,
        Onboarding {
            auto: false,
            verify_timeout_secs: 14400,
        }
    );
    assert_eq!(
        o.metering,
        Metering {
            otlp: false,
            otlp_port: 4318,
        }
    );
}

/// One `[orchestrator.<table>] <key> = <value>` in a config of its own: exactly one
/// problem, printed exactly, and the returned config.
fn one_problem(table: &str, key: &str, value: &str) -> (Config, String) {
    let (config, problems) = parse(&format!("[orchestrator.{table}]\n{key} = {value}\n"));
    assert_eq!(problems.len(), 1, "{table}.{key} = {value}: {problems:?}");
    (config, problems[0].to_string())
}

#[test]
fn adapt_out_of_range_values_fall_back() {
    let defaults = Orchestrator::default();

    for value in ["4", "601"] {
        let (config, message) = one_problem("deciders", "timeout_secs", value);
        assert_eq!(
            message,
            "orchestrator.deciders.timeout_secs: must be between 5 and 600 (using 90)"
        );
        assert_eq!(config.orchestrator.deciders, defaults.deciders);
    }

    let (config, message) = one_problem("deciders", "slot_wait_secs", "601");
    assert_eq!(
        message,
        "orchestrator.deciders.slot_wait_secs: must be between 0 and 600 (using 30)"
    );
    assert_eq!(config.orchestrator.deciders, defaults.deciders);

    let (config, message) = one_problem("scouts", "timeout_secs", "59");
    assert_eq!(
        message,
        "orchestrator.scouts.timeout_secs: must be between 60 and 7200 (using 900)"
    );
    assert_eq!(config.orchestrator.scouts, defaults.scouts);

    let (config, message) = one_problem("scouts", "max_tool_calls", "9");
    assert_eq!(
        message,
        "orchestrator.scouts.max_tool_calls: must be between 10 and 1000 (using 120)"
    );
    assert_eq!(config.orchestrator.scouts, defaults.scouts);

    let (config, message) = one_problem("onboarding", "verify_timeout_secs", "9");
    assert_eq!(
        message,
        "orchestrator.onboarding.verify_timeout_secs: must be between 10 and 14400 (using 1800)"
    );
    assert_eq!(config.orchestrator.onboarding, defaults.onboarding);

    let (config, message) = one_problem("metering", "otlp_port", "70000");
    assert_eq!(
        message,
        "orchestrator.metering.otlp_port: must be between 0 and 65535 (using 0)"
    );
    assert_eq!(config.orchestrator.metering, defaults.metering);
}

#[test]
fn decider_mode_values() {
    for (text, mode) in [
        ("claude", DeciderMode::Claude),
        ("codex", DeciderMode::Codex),
        ("off", DeciderMode::Off),
    ] {
        let (config, problems) = parse(&format!("[orchestrator.deciders]\nmode = \"{text}\"\n"));
        assert!(problems.is_empty(), "{text}: {problems:?}");
        assert_eq!(config.orchestrator.deciders.mode, mode);
    }
    let (config, message) = one_problem("deciders", "mode", "\"gpt\"");
    assert_eq!(
        message,
        "orchestrator.deciders.mode: must be claude, codex or off (using claude)"
    );
    assert_eq!(config.orchestrator.deciders.mode, DeciderMode::Claude);
}

#[test]
fn scouts_runtime_shell_is_a_problem() {
    let (config, message) = one_problem("scouts", "runtime", "\"shell\"");
    assert_eq!(
        message,
        "orchestrator.scouts.runtime: must be claude or codex \
         (using orchestrator.default_runtime)"
    );
    assert_eq!(config.orchestrator.scouts.runtime, None);
    let (config, problems) = parse("[orchestrator.scouts]\nruntime = \"claude\"\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.orchestrator.scouts.runtime, Some(Runtime::Claude));
}

#[test]
fn unknown_adapt_keys_are_reported() {
    let (_, problems) = parse(
        r#"
[orchestrator.deciders]
model = "x"

[orchestrator.onboarding]
cache_dirs = ["/tmp"]
"#,
    );
    let mut printed: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    printed.sort();
    assert_eq!(
        printed,
        vec![
            "orchestrator.deciders.model: unknown key, ignored (using nothing)".to_string(),
            "orchestrator.onboarding.cache_dirs: unknown key, ignored (using nothing)".to_string(),
        ]
    );
    // Every key this milestone adds is known.
    let (_, problems) = parse(
        r#"
[orchestrator]
fast_path = true
[orchestrator.deciders]
mode = "off"
timeout_secs = 90
strength = "fast"
effort = "low"
slot_wait_secs = 30
[orchestrator.scouts]
runtime = "claude"
strength = "fast"
effort = "low"
timeout_secs = 900
max_tool_calls = 120
[orchestrator.onboarding]
auto = true
verify_timeout_secs = 1800
[orchestrator.metering]
otlp = true
otlp_port = 0
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
}

#[test]
fn adapt_wrong_types_are_problems() {
    let (config, problems) = parse(
        r#"
[orchestrator]
fast_path = "no"
deciders = 3
[orchestrator.scouts]
strength = "huge"
effort = "max"
[orchestrator.onboarding]
auto = "yes"
"#,
    );
    let mut printed: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    printed.sort();
    assert_eq!(
        printed,
        vec![
            "orchestrator.deciders: expected a table (using table of defaults)".to_string(),
            "orchestrator.fast_path: expected a boolean (using true)".to_string(),
            "orchestrator.onboarding.auto: expected a boolean (using true)".to_string(),
            "orchestrator.scouts.effort: must be low, medium or high (using low)".to_string(),
            "orchestrator.scouts.strength: must be fast, standard or frontier (using fast)"
                .to_string(),
        ]
    );
    // Milestone 9.8 decision 14: the scout keys are present, so they still feed the
    // research row (at their defaults, today's research route) and give a note each.
    let o = config.orchestrator;
    let research = crate::models::builtin_choice(crate::Role::Research);
    assert_eq!(o.roles.rows, [(crate::Role::Research, research)].into());
    assert_eq!(o.roles_notes.len(), 2, "{:?}", o.roles_notes);
    let unmigrated = Orchestrator {
        roles: Default::default(),
        roles_notes: Vec::new(),
        ..o
    };
    assert_eq!(unmigrated, Orchestrator::default());
}

#[test]
fn fast_path_false_is_read() {
    let (config, problems) = parse("[orchestrator]\nfast_path = false\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!config.orchestrator.fast_path);
}

#[test]
fn the_dotted_and_table_forms_read_alike() {
    let (dotted, p1) = parse("[orchestrator]\ndeciders.mode = \"off\"\n");
    let (table, p2) = parse("[orchestrator.deciders]\nmode = \"off\"\n");
    assert!(p1.is_empty() && p2.is_empty(), "{p1:?} {p2:?}");
    assert_eq!(dotted.orchestrator.deciders.mode, DeciderMode::Off);
    assert_eq!(dotted.orchestrator, table.orchestrator);
}
