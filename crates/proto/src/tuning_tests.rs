//! Milestone 9.5 task 2: the racer and test-writer roles, race lanes, the tuning types,
//! `McpReady` and the protocol-16 fields, and that what protocol 15 wrote still loads.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::tuning::*;
use crate::*;

/// M8a's round-trip fixtures, loaded as this module's own private copy.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::{a_review, a_route, a_run_info, a_task_info, a_token_usage, an_agent_round};

/// Milestone 8b's builders, for its history records.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "adapt_tests_fixtures.rs"]
mod adapt_fixtures;
use adapt_fixtures::a_stats;

fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON");
}

#[test]
fn agent_roles_serialize_as_words() {
    assert_eq!(
        serde_json::to_value(AgentRole::Racer).unwrap(),
        json!("racer")
    );
    assert_eq!(
        serde_json::to_value(AgentRole::TestWriter).unwrap(),
        json!("test_writer")
    );
    assert_eq!(
        serde_json::from_value::<AgentRole>(json!("test_writer")).unwrap(),
        AgentRole::TestWriter
    );
    assert!(serde_json::from_value::<AgentRole>(json!("testwriter")).is_err());
    both_ways(&AgentRole::Racer);
    both_ways(&AgentRole::TestWriter);
}

#[test]
fn race_lane_is_a_letter() {
    assert_eq!(serde_json::to_value(RaceLane::A).unwrap(), json!("a"));
    assert_eq!(serde_json::to_value(RaceLane::B).unwrap(), json!("b"));
    assert_eq!(
        serde_json::from_value::<RaceLane>(json!("b")).unwrap(),
        RaceLane::B
    );
    assert!(serde_json::from_value::<RaceLane>(json!("A")).is_err());
    assert_eq!((RaceLane::A.label(), RaceLane::B.label()), ("a", "b"));
    assert_eq!(RaceLane::A.other(), RaceLane::B);
    assert_eq!(RaceLane::B.other(), RaceLane::A);
    assert!(RaceLane::A < RaceLane::B);
}

/// A plan task as protocol 15 wrote it: no `race`, no `pair`.
const P15_PLAN: &str = r#"
goal = "Add password reset"

[[task]]
id = "t1"
title = "Reset token model"
size = "M"
owns = ["crates/auth/src/token.rs"]
brief = "..."
acceptance = ["..."]
"#;

#[test]
fn plan_task_race_and_pair_default_false() {
    let plan: Plan = toml::from_str(P15_PLAN).expect("a protocol-15 plan parses");
    let task = &plan.tasks[0];
    assert!(!task.race && !task.pair);

    let racing: Plan =
        toml::from_str(&P15_PLAN.replace("size = \"M\"", "size = \"M\"\nrace = true"))
            .expect("race = true parses");
    assert!(racing.tasks[0].race && !racing.tasks[0].pair);
    let paired: Plan =
        toml::from_str(&P15_PLAN.replace("size = \"M\"", "size = \"M\"\npair = true"))
            .expect("pair = true parses");
    assert!(paired.tasks[0].pair && !paired.tasks[0].race);
    both_ways(&racing.tasks[0]);

    // The JSON of an `edit_plan` add or a `run.json` spec, from before 9.5.
    let mut json = serde_json::to_value(task).unwrap();
    let map = json.as_object_mut().unwrap();
    map.remove("race");
    map.remove("pair");
    let old: PlanTask = serde_json::from_value(json).unwrap();
    assert_eq!(&old, task);

    // An amend without them sets neither.
    let PlanEdit::AmendTask { race, pair, .. } =
        serde_json::from_str(r#"{"op":"amend_task","task_id":"t3","priority":5}"#).unwrap()
    else {
        panic!("an amend");
    };
    assert_eq!((race, pair), (None, None));
    let PlanEdit::AmendTask { race, pair, .. } =
        serde_json::from_str(r#"{"op":"amend_task","task_id":"t3","race":true,"pair":false}"#)
            .unwrap()
    else {
        panic!("an amend");
    };
    assert_eq!((race, pair), (Some(true), Some(false)));
}

/// Decision 10's tuning file with every table.
const TUNING_TOML: &str = r#"
v = 1

[budgets.s]
tool_calls = 55
minutes = 18
samples = 34
at = 1790500000

[budgets.m]
tool_calls = 160
minutes = 70
tokens = 900000
samples = 31
at = 1790500000

[budgets.hub]
tool_calls = 300
minutes = 120
samples = 30
at = 1790500000

[weights]
s_secs = 550
m_secs = 1650
hub_secs = 1650
derived = ["M", "hub"]
at = 1790500000

[thresholds]
s_lines = 35
m_lines = 100

[routes.s]
strength = "standard"
effort = "medium"

[dismissed]
"route-m" = "frontier/high"
"#;

#[test]
fn tuning_file_round_trips_and_rejects_unknown_keys() {
    let file: TuningFile = toml::from_str(TUNING_TOML).expect("decision 10's file parses");
    assert_eq!(file.v, TUNING_VERSION);
    assert_eq!(
        file.budgets["s"],
        ClassBudget {
            tool_calls: 55,
            minutes: 18,
            tokens: None,
            samples: 34,
            at: 1_790_500_000,
        }
    );
    assert_eq!(file.budgets["m"].tokens, Some(900_000));
    assert_eq!(file.budgets.len(), 3);
    let weights = file.weights.clone().expect("weights");
    assert_eq!(
        (weights.s_secs, weights.m_secs, weights.hub_secs),
        (550, 1650, 1650)
    );
    assert_eq!(weights.derived, ["M", "hub"]);
    assert_eq!(
        file.thresholds,
        Some(SizeThresholds {
            s_lines: 35,
            m_lines: 100,
        })
    );
    assert_eq!(
        file.routes["s"],
        ClassRoute {
            strength: Strength::Standard,
            effort: Effort::Medium,
        }
    );
    assert_eq!(file.dismissed["route-m"], "frontier/high");

    let text = toml::to_string(&file).unwrap();
    assert_eq!(toml::from_str::<TuningFile>(&text).unwrap(), file, "{text}");
    both_ways(&file);

    // An absent table is its default; an empty file writes only `v`.
    let empty: TuningFile = toml::from_str("v = 1").unwrap();
    assert_eq!(empty, TuningFile::default());
    assert_eq!(toml::to_string(&empty).unwrap().trim(), "v = 1");

    // Unknown keys are refused at every level, by name, even beside every known key
    // (so the refusal is not a missing field's).
    for (bad, key) in [
        (
            TUNING_TOML.replace("tool_calls = 55", "tool_calls = 55\ncalls = 1"),
            "calls",
        ),
        (TUNING_TOML.replace("v = 1", "v = 1\nextra = 1"), "extra"),
        (format!("{TUNING_TOML}\n[extra]\nx = 1"), "extra"),
        (
            TUNING_TOML.replace("s_lines = 35", "s_lines = 35\nl_lines = 300"),
            "l_lines",
        ),
        (
            TUNING_TOML.replace("hub_secs = 1650", "hub_secs = 1650\nl_secs = 1"),
            "l_secs",
        ),
        (
            TUNING_TOML.replace("effort = \"medium\"", "effort = \"medium\"\nmodel = \"x\""),
            "model",
        ),
    ] {
        let error = toml::from_str::<TuningFile>(&bad)
            .expect_err(&bad)
            .to_string();
        assert!(
            error.contains(&format!("unknown field `{key}`")),
            "{bad}\n{error}"
        );
    }
}

#[test]
fn size_thresholds_default_to_20_and_100() {
    assert_eq!(
        SizeThresholds::default(),
        SizeThresholds {
            s_lines: 20,
            m_lines: 100,
        }
    );
}

#[test]
fn old_history_lines_still_parse() {
    let mut tasks = 0;
    for text in [
        include_str!("m9_history_v2.jsonl"),
        include_str!("m93_history.jsonl"),
    ] {
        for line in text.lines() {
            let line: HistoryLine = serde_json::from_str(line).expect("an older line decodes");
            if let HistoryLine::Task(record) = line {
                tasks += 1;
                assert_eq!(record.pattern, None);
                assert_eq!(record.race_winner, None);
                assert!(!record.race_adopted);
                assert_eq!(record.writer_failures, 0);
                assert_eq!(record.round, 0, "0 reads as round 1");
            }
        }
    }
    assert!(tasks >= 2, "both fixtures carry a task line");
    let captured: Vec<HistoryLine> = include_str!("m93_history.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        captured.iter().any(|l| matches!(l, HistoryLine::Round(_))),
        "the 9.3 fixture has a round line"
    );

    // Old stats decode without `tuning`.
    let mut old = serde_json::to_value(a_stats()).unwrap();
    old.as_object_mut().unwrap().remove("tuning");
    let old: HistoryStats = serde_json::from_value(old).unwrap();
    assert_eq!(old.tuning, None);
}

#[path = "tuning_tests_wire.rs"]
mod wire;

#[path = "tuning_tests_kept.rs"]
mod kept;
