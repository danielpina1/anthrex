//! The merge of milestone 9.0.6 (protocol 13: `actions`) with milestone 9.2 (protocol
//! 14: `delivery`, `pr`): a snapshot from either side decodes with the other side's
//! fields empty and writes none of them, and a snapshot carrying both round-trips with
//! every key where Interfaces puts it.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::*;

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

fn an_action(kind: ActionKind) -> ActionInfo {
    ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        label: "l".into(),
        effect: "e".into(),
        refused_why: None,
        kind,
    }
}

fn a_delivery() -> DeliveryInfo {
    DeliveryInfo {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
        repo: "fake/app".into(),
        watching: true,
        delivering: false,
        poll_secs: 60,
        skipped_stages: vec![2],
        alerts: Vec::new(),
    }
}

fn a_stage_pr() -> StagePrInfo {
    StagePrInfo {
        number: 142,
        url: "https://github.com/fake/app/pull/142".into(),
        state: PrState::Open,
        base: "main".into(),
        opened_at: 1_700_000_100,
        head: "aaaa1111".into(),
        ci: CiState::Pending,
        checks: Vec::new(),
        threads: ThreadCounts::default(),
        fix_tasks: Vec::new(),
        paused: false,
        human_review_secs: 600,
        merged_at: None,
        merge_commit: None,
    }
}

/// A 9.0.6 snapshot: the 9.1 fixture with actions on the run, a task and a stage.
fn a_protocol_13_run_info() -> RunInfo {
    let mut info: RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    info.actions = vec![an_action(ActionKind::Accept)];
    info.tasks[0].actions = vec![an_action(ActionKind::Answer)];
    info.stages[0].actions = vec![an_action(ActionKind::MessageStage { stage: 1 })];
    info
}

#[test]
fn a_protocol_13_snapshot_decodes_with_no_delivery_and_writes_none() {
    let written = serde_json::to_value(a_protocol_13_run_info()).unwrap();
    let back: RunInfo = serde_json::from_value(written.clone()).unwrap();
    assert_eq!(back.delivery, None);
    assert!(back.stages.iter().all(|s| s.pr.is_none()));
    assert_eq!(back.actions.len(), 1);
    assert_eq!(back.stages[0].actions.len(), 1);
    // Re-written by 9.2's types, it is exactly what 9.0.6 wrote.
    let again = serde_json::to_value(&back).unwrap();
    assert_eq!(again, written);
    assert!(again.get("delivery").is_none());
    assert!(again["stages"][0].get("pr").is_none());
    let packed = rmp_serde::to_vec_named(&back).unwrap();
    assert!(!packed.windows(8).any(|w| w == b"delivery"));
}

#[test]
fn a_9_2_snapshot_written_before_the_merge_decodes_with_no_actions() {
    let mut info: RunInfo = serde_json::from_str(include_str!("m9_1_run_info.json")).unwrap();
    info.delivery = Some(a_delivery());
    info.stages[0].pr = Some(a_stage_pr());
    let written = serde_json::to_value(&info).unwrap();
    assert!(written.get("actions").is_none());
    assert!(written["stages"][0].get("actions").is_none());
    let back: RunInfo = serde_json::from_value(written).unwrap();
    assert!(back.actions.is_empty() && back.stages.iter().all(|s| s.actions.is_empty()));
    assert_eq!(back, info);
}

#[test]
fn a_snapshot_with_both_sides_fields_round_trips_with_its_keys_pinned() {
    let mut info = a_protocol_13_run_info();
    info.delivery = Some(a_delivery());
    info.stages[0].pr = Some(a_stage_pr());
    both_ways(&info);

    let value = serde_json::to_value(&info).unwrap();
    assert_eq!(
        value["actions"],
        serde_json::json!([{
            "kind": {"kind": "accept"},
            "label": "l",
            "effect": "e",
            "needs": "confirm",
            "destructive": false,
            "refused_why": null,
        }])
    );
    assert_eq!(
        value["delivery"],
        serde_json::json!({
            "mode": "pr",
            "remote": "origin",
            "repo": "fake/app",
            "watching": true,
            "delivering": false,
            "poll_secs": 60,
            "skipped_stages": [2],
        })
    );
    let stage = &value["stages"][0];
    assert_eq!(
        stage["actions"][0]["kind"],
        serde_json::json!({"kind": "message_stage", "stage": 1})
    );
    assert_eq!(stage["pr"]["number"], 142);
    assert_eq!(stage["pr"]["state"], "open");
    assert_eq!(stage["pr"]["ci"], "pending");
    assert_eq!(
        value["tasks"][0]["actions"][0]["kind"],
        serde_json::json!({"kind": "answer"})
    );
}
