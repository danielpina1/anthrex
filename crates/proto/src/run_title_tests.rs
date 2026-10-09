//! Protocol 18: `RunInfo.title`, the run's model-given short title, round-trips, is
//! left out while empty, and a protocol-17 snapshot without it still decodes.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::*;

/// M8a's round-trip fixtures, loaded as this module's own private copy.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::a_run_info;

fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON: {json}");
}

#[test]
fn a_run_title_round_trips() {
    let run = RunInfo {
        title: "Password reset flow".into(),
        ..a_run_info()
    };
    both_ways(&run);
    let json = serde_json::to_value(&run).unwrap();
    assert_eq!(json["title"], "Password reset flow");
    both_ways(&DaemonMsg::Run(RunReply::Snapshot(RunsSnapshot {
        revision: 3,
        runs: vec![run],
        now: 7,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
        queued_goals: Vec::new(),
    })));
}

#[test]
fn an_empty_title_is_left_out() {
    let run = a_run_info();
    assert_eq!(run.title, "");
    let json = serde_json::to_value(&run).unwrap();
    assert!(json.get("title").is_none(), "{json}");
    both_ways(&run);
}

#[test]
fn a_protocol_17_run_info_without_a_title_decodes() {
    for (name, text) in [
        ("m95_run_info.json", include_str!("m95_run_info.json")),
        ("m93_run_info.json", include_str!("m93_run_info.json")),
    ] {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        assert!(value.get("title").is_none(), "{name} predates the title");
        let json: RunInfo = serde_json::from_str(text).unwrap();
        let packed: RunInfo = rmp_serde::from_slice(&rmp_serde::to_vec_named(&value).unwrap())
            .unwrap_or_else(|e| panic!("{name} decodes from MessagePack: {e}"));
        for run in [&json, &packed] {
            assert_eq!(run.title, "", "{name}");
        }
    }
}
