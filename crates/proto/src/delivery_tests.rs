//! Milestone 9.2 task 2: the delivery types, the new requests, `reply_comment`, the
//! `stage` history line, and that what milestone 9.1 wrote (protocol 12, history
//! version 3) still loads.

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::delivery::*;
use crate::history::StageLine;
use crate::run_wire::{RunReply, request};
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

fn a_stage_pr() -> StagePrInfo {
    StagePrInfo {
        number: 142,
        url: "https://github.com/fake/app/pull/142".into(),
        state: PrState::Open,
        base: "main".into(),
        opened_at: 1_700_000_100,
        head: "aaaa1111".into(),
        ci: CiState::Red,
        checks: vec![
            CheckRunInfo {
                name: "build".into(),
                state: CiState::Green,
                fix_task: None,
            },
            CheckRunInfo {
                name: "test".into(),
                state: CiState::Red,
                fix_task: Some("fix3".into()),
            },
        ],
        threads: ThreadCounts {
            new: 2,
            tasked: 3,
            replied: 4,
            ignored: 5,
        },
        fix_tasks: vec!["fix3 ci working".into()],
        paused: false,
        human_review_secs: 600,
        merged_at: Some(1_700_009_200),
        merge_commit: Some("bbbb2222".into()),
    }
}

fn a_delivery_info() -> DeliveryInfo {
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

fn a_stage_line() -> StageLine {
    StageLine {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/stage/1".into(),
        run_id: "run-a1b2".into(),
        stage: 1,
        pr: 142,
        time_to_open_secs: 90,
        human_review_secs: 600,
        ci_rounds: 5,
        review_rounds: 2,
        sync_tasks: 3,
        outcome: StageOutcome::Merged,
        merge_method: MergeMethod::SquashOrRebase,
        at: 1_700_009_000,
    }
}

/// Every key exactly as Interfaces "proto" spells it, each with a value no other field
/// of its type has, so a swapped or renamed key fails here (review of `628a932`).
#[test]
fn delivery_keys_are_pinned_by_literal_json() {
    use serde_json::json;
    assert_eq!(
        serde_json::to_value(ThreadCounts {
            new: 2,
            tasked: 3,
            replied: 4,
            ignored: 5,
        })
        .unwrap(),
        json!({"new": 2, "tasked": 3, "replied": 4, "ignored": 5})
    );
    assert_eq!(
        serde_json::to_value(a_stage_pr()).unwrap(),
        json!({
            "number": 142,
            "url": "https://github.com/fake/app/pull/142",
            "state": "open",
            "base": "main",
            "opened_at": 1_700_000_100u64,
            "head": "aaaa1111",
            "ci": "red",
            "checks": [
                {"name": "build", "state": "green", "fix_task": null},
                {"name": "test", "state": "red", "fix_task": "fix3"}
            ],
            "threads": {"new": 2, "tasked": 3, "replied": 4, "ignored": 5},
            "fix_tasks": ["fix3 ci working"],
            "paused": false,
            "human_review_secs": 600,
            "merged_at": 1_700_009_200u64,
            "merge_commit": "bbbb2222"
        })
    );
    assert_eq!(
        serde_json::to_value(a_delivery_info()).unwrap(),
        json!({
            "mode": "pr",
            "remote": "origin",
            "repo": "fake/app",
            "watching": true,
            "delivering": false,
            "poll_secs": 60,
            "skipped_stages": [2]
        })
    );
    assert_eq!(
        serde_json::to_value(DeliveryProfile {
            mode: DeliveryMode::Pr,
            remote: "upstream".into(),
        })
        .unwrap(),
        json!({"mode": "pr", "remote": "upstream"})
    );
    // The `stage` line is written to `history.jsonl` for good: its keys are decision
    // 44's `{ v, run_id, stage, pr, time_to_open_secs, human_review_secs, ci_rounds,
    // review_rounds, sync_tasks, outcome, merge_method, at }` plus Interfaces'
    // `record_id` ("<run>/stage/<n>"), under `"type": "stage"`.
    assert_eq!(
        serde_json::to_value(HistoryLine::Stage(a_stage_line())).unwrap(),
        json!({
            "type": "stage",
            "v": 5,
            "record_id": "run-a1b2/stage/1",
            "run_id": "run-a1b2",
            "stage": 1,
            "pr": 142,
            "time_to_open_secs": 90,
            "human_review_secs": 600,
            "ci_rounds": 5,
            "review_rounds": 2,
            "sync_tasks": 3,
            "outcome": "merged",
            "merge_method": "squash_or_rebase",
            "at": 1_700_009_000u64
        })
    );
}

#[test]
fn delivery_types_round_trip() {
    for mode in [DeliveryMode::Local, DeliveryMode::Pr] {
        both_ways(&mode);
    }
    assert_eq!(DeliveryMode::default(), DeliveryMode::Local);
    assert_eq!(serde_json::to_string(&DeliveryMode::Pr).unwrap(), r#""pr""#);
    assert_eq!(
        serde_json::to_string(&DeliveryMode::Local).unwrap(),
        r#""local""#
    );
    both_ways(&DeliveryProfile {
        mode: DeliveryMode::Pr,
        remote: "upstream".into(),
    });
    let profile: DeliveryProfile = serde_json::from_str(r#"{"mode":"pr"}"#).unwrap();
    assert_eq!(profile.remote, "origin", "remote defaults to origin");
    assert!(
        serde_json::from_str::<DeliveryProfile>(r#"{"mode":"pr","sync":"always"}"#).is_err(),
        "deny_unknown_fields"
    );
    for state in [PrState::Open, PrState::Merged, PrState::Closed] {
        both_ways(&state);
    }
    for ci in [
        CiState::None,
        CiState::Pending,
        CiState::Green,
        CiState::Red,
    ] {
        both_ways(&ci);
    }
    for category in [
        CiCategory::Test,
        CiCategory::Build,
        CiCategory::Lint,
        CiCategory::Infra,
        CiCategory::Unknown,
    ] {
        both_ways(&category);
    }
    assert_eq!(
        serde_json::to_string(&CiCategory::Infra).unwrap(),
        r#""infra""#
    );
    both_ways(&ThreadCounts::default());
    both_ways(&a_stage_pr());
    both_ways(&StagePrInfo {
        state: PrState::Merged,
        merged_at: Some(1_700_009_000),
        merge_commit: Some("bbbb2222".into()),
        ..a_stage_pr()
    });
    both_ways(&a_delivery_info());
    for outcome in [
        StageOutcome::Merged,
        StageOutcome::Closed,
        StageOutcome::OpenAtCancel,
    ] {
        both_ways(&outcome);
    }
    for method in [
        MergeMethod::Merge,
        MergeMethod::SquashOrRebase,
        MergeMethod::None,
    ] {
        both_ways(&method);
    }
    assert_eq!(
        serde_json::to_string(&StageOutcome::OpenAtCancel).unwrap(),
        r#""open_at_cancel""#
    );
    assert_eq!(
        serde_json::to_string(&MergeMethod::SquashOrRebase).unwrap(),
        r#""squash_or_rebase""#
    );
}

#[test]
fn new_requests_round_trip() {
    let start = |delivery| RunRequest::Start {
        plan_toml: "[[task]]".into(),
        dir: "/tmp/repo".into(),
        yes: true,
        trust_project: false,
        unconfined_checks: false,
        delivery,
        design: None,
    };
    let goal = |delivery| RunRequest::StartGoal {
        goal: "add a flag".into(),
        dir: "/tmp/repo".into(),
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery,
        continue_from: None,
        design: None,
    };
    for delivery in [None, Some(DeliveryMode::Pr), Some(DeliveryMode::Local)] {
        both_ways(&ClientMsg::Run(start(delivery)));
        both_ways(&ClientMsg::Run(goal(delivery)));
    }
    // A 9.1 client's `Start` and `StartGoal` carry no `delivery`.
    let old: RunRequest = serde_json::from_str(
        r#"{"Start":{"plan_toml":"","dir":"/r","yes":false,"trust_project":false}}"#,
    )
    .unwrap();
    assert_eq!(
        old,
        RunRequest::Start {
            plan_toml: String::new(),
            dir: "/r".into(),
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            delivery: None,
            design: None,
        }
    );
    let old: RunRequest = serde_json::from_str(
        r#"{"StartGoal":{"goal":"g","dir":"/r","yes":false,"trust_project":false,"unconfined_checks":false}}"#,
    )
    .unwrap();
    let RunRequest::StartGoal { delivery, .. } = old else {
        panic!("a StartGoal: {old:?}");
    };
    assert_eq!(delivery, None);

    both_ways(&ClientMsg::Run(RunRequest::Deliver {
        run_id: "run-a1b2".into(),
        stage: 2,
    }));
    for on in [true, false] {
        both_ways(&ClientMsg::Run(RunRequest::Watch {
            run_id: "run-a1b2".into(),
            on,
        }));
    }
    assert_eq!(request::DELIVER, "run deliver");
    assert_eq!(request::WATCH, "run watch");
    for label in [request::DELIVER, request::WATCH] {
        let reply = RunReply::Done {
            request: label.into(),
            message: "watching run run-a1b2's pull requests".into(),
            request_id: None,
        };
        let tagged = reply.clone().tagged(Some(7));
        assert_eq!(tagged.request_id(), Some(7));
        both_ways(&DaemonMsg::Run(tagged));
    }
}

#[test]
fn reply_comment_edit_round_trips() {
    let json =
        r#"{"op":"reply_comment","pr":142,"thread":"t9","body":"Because the API is public."}"#;
    let edit: PlanEdit = serde_json::from_str(json).unwrap();
    assert_eq!(
        edit,
        PlanEdit::ReplyComment {
            pr: 142,
            thread: "t9".into(),
            body: "Because the API is public.".into(),
        }
    );
    assert_eq!(serde_json::to_string(&edit).unwrap(), json);
    both_ways(&edit);
    for missing in [
        r#"{"op":"reply_comment","thread":"t9","body":"x"}"#,
        r#"{"op":"reply_comment","pr":142,"body":"x"}"#,
        r#"{"op":"reply_comment","pr":142,"thread":"t9"}"#,
    ] {
        assert!(
            serde_json::from_str::<PlanEdit>(missing).is_err(),
            "{missing} must not parse"
        );
    }
    assert!(
        serde_json::from_str::<PlanEdit>(
            r#"{"op":"reply_comment","pr":142,"thread":"t9","body":"x","owns":["a"]}"#
        )
        .is_err(),
        "deny_unknown_fields"
    );
}

const A_PLAN_TASK: &str = r#"{"id":"t1","title":"Add a flag","kind":"code","size":"S",
    "owns":["src/a.rs"],"brief":"b","acceptance":["a"],"stage":2}"#;

#[test]
fn plan_task_addresses_defaults_empty() {
    let task: PlanTask = serde_json::from_str(A_PLAN_TASK).expect("a 9.1 plan task decodes");
    assert!(task.addresses.is_empty());
    assert!(
        serde_json::to_value(&task)
            .unwrap()
            .get("addresses")
            .is_none(),
        "an empty addresses is left out, so a plan is written as 9.1's"
    );
    assert_eq!(task.stage, 2);
    let mut task = task;
    task.addresses = vec!["142:t98765".into(), "142:c5".into()];
    both_ways(&task);
    let with = A_PLAN_TASK.replace(r#""stage":2"#, r#""stage":2,"addresses":["142:r7"]"#);
    let task: PlanTask = serde_json::from_str(&with).unwrap();
    assert_eq!(task.addresses, vec!["142:r7".to_string()]);
    let unknown = A_PLAN_TASK.replace(r#""stage":2"#, r#""stage":2,"addressess":[]"#);
    assert!(
        serde_json::from_str::<PlanTask>(&unknown).is_err(),
        "deny_unknown_fields still refuses an unknown key"
    );
}

fn drop_strength(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::Object(map) => {
            map.remove("strength");
            map.values_mut().for_each(drop_strength);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(drop_strength),
        _ => {}
    }
}

/// `m9_1_run_info.json` is milestone 9.1's `RunInfo` (two stages, a bisect fix task),
/// serialized by 9.1's own types before milestone 9.2 changed them; `m9_1_profile.toml`
/// is a 9.1 `profile.toml`.
#[test]
fn old_snapshot_and_profile_still_decode() {
    let run: RunInfo =
        serde_json::from_str(include_str!("m9_1_run_info.json")).expect("a 9.1 RunInfo decodes");
    assert_eq!(run.delivery, None);
    assert_eq!(run.stages.len(), 2);
    assert!(run.stages.iter().all(|s| s.pr.is_none()));
    assert_eq!(run.test_slots, 8, "a 9.1 field survives");
    assert_eq!(run.tasks[0].fixes.as_deref(), Some("bisect of t4"));
    // A local run's snapshot is written exactly as 9.1 wrote it: `delivery` and each
    // stage's `pr` are left out while `None`. Milestone 9.3 adds only `round` (always
    // written, 1 for a run, task or stage from before it; `rounds_tests.rs`).
    let mut stored: serde_json::Value =
        serde_json::from_str(include_str!("m9_1_run_info.json")).unwrap();
    stored["round"] = 1.into();
    for key in ["tasks", "stages"] {
        for node in stored[key].as_array_mut().unwrap() {
            node["round"] = 1.into();
        }
    }
    // Milestone 9.8 (task M9.8.14): a route's `strength` is read and ignored, never
    // written.
    drop_strength(&mut stored);
    assert_eq!(serde_json::to_value(&run).unwrap(), stored);

    let mut run = run;
    run.delivery = Some(a_delivery_info());
    run.stages[0].pr = Some(a_stage_pr());
    both_ways(&run);

    let profile: RepoProfile =
        toml::from_str(include_str!("m9_1_profile.toml")).expect("a 9.1 profile decodes");
    assert_eq!(profile.delivery, None);
    assert_eq!(profile.check.as_deref(), Some("cargo test"));
}

#[test]
fn profile_without_delivery_writes_no_table() {
    let profile: RepoProfile = toml::from_str(include_str!("m9_1_profile.toml")).unwrap();
    let text = toml::to_string(&profile).unwrap();
    assert!(!text.contains("delivery"), "{text}");
    assert!(
        serde_json::to_value(&profile)
            .unwrap()
            .get("delivery")
            .is_none(),
        "skip_serializing_if leaves the key out"
    );
    assert_eq!(text, include_str!("m9_1_profile.toml"), "unchanged for it");

    let with = RepoProfile {
        delivery: Some(DeliveryProfile {
            mode: DeliveryMode::Pr,
            remote: "origin".into(),
        }),
        ..profile
    };
    let text = toml::to_string(&with).unwrap();
    assert!(
        text.contains("[delivery]\nmode = \"pr\"\nremote = \"origin\"\n"),
        "{text}"
    );
    assert_eq!(toml::from_str::<RepoProfile>(&text).unwrap(), with);
    let only_mode = "check = \"x\"\n\n[delivery]\nmode = \"local\"\n";
    let read: RepoProfile = toml::from_str(only_mode).unwrap();
    assert_eq!(
        read.delivery,
        Some(DeliveryProfile {
            mode: DeliveryMode::Local,
            remote: "origin".into(),
        })
    );
}

#[test]
fn hold_kind_fix_round_trips() {
    let json = r#"{"kind":"fix","stage":1,"paths":["src/x.rs"]}"#;
    let kind: HoldKind = serde_json::from_str(json).unwrap();
    assert_eq!(
        kind,
        HoldKind::Fix {
            stage: 1,
            paths: vec!["src/x.rs".into()],
        }
    );
    assert_eq!(serde_json::to_string(&kind).unwrap(), json);
    both_ways(&kind);
}

#[test]
fn stage_history_line_round_trips() {
    assert_eq!(HISTORY_VERSION, 5);
    let line = HistoryLine::Stage(a_stage_line());
    let json = serde_json::to_string(&line).unwrap();
    assert!(json.starts_with(r#"{"type":"stage","#), "{json}");
    for key in [
        r#""outcome":"merged""#,
        r#""merge_method":"squash_or_rebase""#,
        r#""record_id":"run-a1b2/stage/1""#,
    ] {
        assert!(json.contains(key), "{key} in {json}");
    }
    both_ways(&line);
    both_ways(&HistoryLine::Stage(StageLine {
        outcome: StageOutcome::OpenAtCancel,
        merge_method: MergeMethod::None,
        ..a_stage_line()
    }));

    // Version-2 lines (milestone 9) and version-3 lines (9.1) still decode.
    for line in include_str!("m9_history_v2.jsonl").lines() {
        serde_json::from_str::<HistoryLine>(line).expect("a version-2 line decodes");
    }
    let v3 = r#"{"type":"bisect","v":3,"record_id":"r/bisect/1/1","at":1,"run_id":"r",
        "stage":1,"head":"c","tests":["t"],"range":2,"probes":2,"culprit":"t4",
        "reason":null,"fix_task":"fix1"}"#;
    let HistoryLine::Bisect(bisect) = serde_json::from_str(v3).expect("a version-3 line") else {
        panic!("a bisect line");
    };
    assert_eq!(bisect.v, 3);
}

/// The variant-order helpers other test modules share, and the appended-variants test
/// (moved to `delivery_tests_wire.rs` to keep this file under 600 lines).
#[path = "delivery_tests_wire.rs"]
mod wire;
pub(crate) use wire::{tagged_names, variant_at, variant_names};
