//! M9.3: the orchestrator agent, planner, wake, message and note settings (milestone 9
//! Interfaces "`config`").

use super::*;
use crate::orchestrator::agent::{AgentConfig, AgentSettings, PlannerConfig};
use proto::{Effort, Runtime, Strength};

fn printed(problems: &[Problem]) -> Vec<String> {
    let mut printed: Vec<String> = problems.iter().map(|p| p.to_string()).collect();
    printed.sort();
    printed
}

#[test]
fn defaults_when_absent() {
    let expected = AgentSettings {
        planner_task_cap: 12,
        max_scouts: 12,
        wake_orchestrator: true,
        wake_quiet_secs: 5,
        message_max_per_turn: 3,
        note_max_per_task: 10,
        agent: AgentConfig {
            runtime: None,
            model: String::new(),
            effort: Effort::HIGH,
        },
        planners: PlannerConfig {
            runtime: None,
            strength: Strength::Frontier,
            effort: Effort::HIGH,
            max_tool_calls: 200,
            timeout_secs: 2400,
            max_rejections: 5,
        },
    };
    for text in [
        "",
        "[orchestrator]\n",
        "[orchestrator.agent]\n[orchestrator.planners]\n",
    ] {
        let (config, problems) = parse(text);
        assert!(problems.is_empty(), "{text:?}: {problems:?}");
        assert_eq!(config.orchestrator.agent, expected, "{text:?}");
    }
    assert_eq!(Orchestrator::default().agent, expected);
}

/// One numeric key: where it is written, its bounds, its default and how to read it.
struct Numeric {
    table: &'static str,
    key: &'static str,
    lo: i64,
    hi: i64,
    default: u64,
    get: fn(&AgentSettings) -> u64,
}

#[test]
fn each_range_is_enforced_with_the_exact_message() {
    let cases = [
        Numeric {
            table: "orchestrator",
            key: "planner_task_cap",
            lo: 2,
            hi: 50,
            default: 12,
            get: |s| s.planner_task_cap.into(),
        },
        Numeric {
            table: "orchestrator",
            key: "max_scouts",
            lo: 1,
            hi: 50,
            default: 12,
            get: |s| s.max_scouts.into(),
        },
        Numeric {
            table: "orchestrator",
            key: "wake_quiet_secs",
            lo: 1,
            hi: 120,
            default: 5,
            get: |s| s.wake_quiet_secs,
        },
        Numeric {
            table: "orchestrator",
            key: "message_max_per_turn",
            lo: 0,
            hi: 20,
            default: 3,
            get: |s| s.message_max_per_turn.into(),
        },
        Numeric {
            table: "orchestrator",
            key: "note_max_per_task",
            lo: 0,
            hi: 100,
            default: 10,
            get: |s| s.note_max_per_task.into(),
        },
        Numeric {
            table: "orchestrator.planners",
            key: "max_tool_calls",
            lo: 20,
            hi: 2000,
            default: 200,
            get: |s| s.planners.max_tool_calls.into(),
        },
        Numeric {
            table: "orchestrator.planners",
            key: "timeout_secs",
            lo: 120,
            hi: 14400,
            default: 2400,
            get: |s| s.planners.timeout_secs,
        },
        Numeric {
            table: "orchestrator.planners",
            key: "max_rejections",
            lo: 1,
            hi: 20,
            default: 5,
            get: |s| s.planners.max_rejections.into(),
        },
    ];
    for c in cases {
        let text = |v: i64| format!("[{}]\n{} = {v}\n", c.table, c.key);
        for v in [c.lo, c.hi] {
            let (config, problems) = parse(&text(v));
            assert!(problems.is_empty(), "{} = {v}: {problems:?}", c.key);
            assert_eq!((c.get)(&config.orchestrator.agent) as i64, v, "{}", c.key);
        }
        for v in [c.lo - 1, c.hi + 1] {
            let (config, problems) = parse(&text(v));
            assert_eq!(
                printed(&problems),
                vec![format!(
                    "{}.{}: must be between {} and {} (using {})",
                    c.table, c.key, c.lo, c.hi, c.default
                )],
                "{} = {v}",
                c.key
            );
            assert_eq!((c.get)(&config.orchestrator.agent), c.default, "{}", c.key);
        }
    }
    // Zero turns messages and notes off, and is accepted.
    let (config, problems) =
        parse("[orchestrator]\nmessage_max_per_turn = 0\nnote_max_per_task = 0\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.orchestrator.agent.message_max_per_turn, 0);
    assert_eq!(config.orchestrator.agent.note_max_per_task, 0);
}

#[test]
fn effort_and_strength_and_runtime_parse() {
    for (word, effort) in [
        ("low", Effort::LOW),
        ("medium", Effort::MEDIUM),
        ("high", Effort::HIGH),
    ] {
        let (config, problems) = parse(&format!(
            "[orchestrator.agent]\neffort = \"{word}\"\n[orchestrator.planners]\neffort = \"{word}\"\n"
        ));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.orchestrator.agent.agent.effort, effort);
        assert_eq!(config.orchestrator.agent.planners.effort, effort);
    }
    for (word, strength) in [
        ("fast", Strength::Fast),
        ("standard", Strength::Standard),
        ("frontier", Strength::Frontier),
    ] {
        let (config, problems) =
            parse(&format!("[orchestrator.planners]\nstrength = \"{word}\"\n"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.orchestrator.agent.planners.strength, strength);
    }
    for (word, runtime) in [("claude", Runtime::Claude), ("codex", Runtime::Codex)] {
        let (config, problems) = parse(&format!(
            "[orchestrator.agent]\nruntime = \"{word}\"\n[orchestrator.planners]\nruntime = \"{word}\"\n"
        ));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(config.orchestrator.agent.agent.runtime, Some(runtime));
        assert_eq!(config.orchestrator.agent.planners.runtime, Some(runtime));
    }
    let (config, problems) = parse(
        "[orchestrator]\nwake_orchestrator = false\n[orchestrator.agent]\nmodel = \"opus\"\n",
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert!(!config.orchestrator.agent.wake_orchestrator);
    assert_eq!(config.orchestrator.agent.agent.model, "opus");

    // Wrong spellings and types: the message, and the default kept.
    let (config, problems) = parse(
        r#"
[orchestrator]
wake_orchestrator = "yes"
[orchestrator.agent]
runtime = "shell"
model = 3
effort = "max"
[orchestrator.planners]
runtime = "shell"
strength = "huge"
effort = "High"
"#,
    );
    assert_eq!(
        printed(&problems),
        vec![
            "orchestrator.agent.effort: must be low, medium or high (using high)".to_string(),
            "orchestrator.agent.model: expected a string (using unset)".to_string(),
            "orchestrator.agent.runtime: must be claude or codex \
             (using orchestrator.default_runtime)"
                .to_string(),
            "orchestrator.planners.effort: must be low, medium or high (using high)".to_string(),
            "orchestrator.planners.runtime: must be claude or codex \
             (using the orchestrator's runtime)"
                .to_string(),
            "orchestrator.planners.strength: must be fast, standard or frontier (using frontier)"
                .to_string(),
            "orchestrator.wake_orchestrator: expected a boolean (using true)".to_string(),
        ]
    );
    assert_eq!(config.orchestrator.agent, AgentSettings::default());

    // A table given as something else.
    let (config, problems) = parse("[orchestrator]\nagent = 1\nplanners = \"x\"\n");
    assert_eq!(
        printed(&problems),
        vec![
            "orchestrator.agent: expected a table (using table of defaults)".to_string(),
            "orchestrator.planners: expected a table (using table of defaults)".to_string(),
        ]
    );
    assert_eq!(config.orchestrator.agent, AgentSettings::default());
}

#[test]
fn unknown_keys_warn() {
    let (_, problems) = parse(
        r#"
[orchestrator.agent]
strength = "fast"
[orchestrator.planners]
model = "x"
"#,
    );
    assert_eq!(
        printed(&problems),
        vec![
            "orchestrator.agent.strength: unknown key, ignored (using nothing)".to_string(),
            "orchestrator.planners.model: unknown key, ignored (using nothing)".to_string(),
        ]
    );
    // Every key this milestone adds is known.
    let (_, problems) = parse(
        r#"
[orchestrator]
planner_task_cap = 12
max_scouts = 12
wake_orchestrator = true
wake_quiet_secs = 5
message_max_per_turn = 3
note_max_per_task = 10
[orchestrator.agent]
runtime = "claude"
model = ""
effort = "high"
[orchestrator.planners]
runtime = "codex"
strength = "frontier"
effort = "high"
max_tool_calls = 200
timeout_secs = 2400
max_rejections = 5
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
}
