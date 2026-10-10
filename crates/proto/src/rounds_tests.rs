//! Milestone 9.3 task 2: rounds, chains, `run iterate`, the iterate edit and the
//! `round` history line (protocol 15, history version 5), and that what protocol 14
//! wrote still loads.

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::delivery::tests::{tagged_names, variant_at, variant_names};
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

fn a_round_info() -> RoundInfo {
    RoundInfo {
        n: 2,
        goal_head: "also add a --verbose flag".into(),
        origin: RoundOrigin::Orchestrator,
        outcome: Some(RoundOutcome::Completed),
        summary_head: Some("added --verbose".into()),
        ended: true,
    }
}

fn an_idle_orchestrator() -> IdleOrchestrator {
    IdleOrchestrator {
        chain: "o-3f9a".into(),
        project: "/tmp/repo".into(),
        after_run: "run-7c21".into(),
        outcome: RunState::Accepted,
        runtime: Runtime::Codex,
        model: "gpt-5.5".into(),
        window_id: Some(7),
        fresh: false,
        runs: 3,
    }
}

fn a_round_line() -> RoundLine {
    RoundLine {
        v: HISTORY_VERSION,
        record_id: "run-a1b2/round/2".into(),
        at: 1_700_009_000,
        run_id: "run-a1b2".into(),
        round: 2,
        origin: RoundOrigin::User,
        outcome: RoundOutcome::Rejected,
        tasks: 4,
        merged: 3,
        calls: 17,
        minutes: 42,
    }
}

#[test]
fn round_types_round_trip() {
    assert_eq!(GOAL_MAX_CHARS, 16_384);
    assert_eq!(ROUNDS_MAX, 20);
    assert_eq!(ROUND_HEAD_CHARS, 200);
    assert_eq!(first_round(), 1);
    for origin in [RoundOrigin::User, RoundOrigin::Orchestrator] {
        both_ways(&origin);
    }
    assert_eq!(
        serde_json::to_string(&RoundOrigin::Orchestrator).unwrap(),
        r#""orchestrator""#
    );
    for outcome in [
        RoundOutcome::Completed,
        RoundOutcome::Rejected,
        RoundOutcome::Cancelled,
    ] {
        both_ways(&outcome);
    }
    assert_eq!(
        serde_json::to_string(&RoundOutcome::Cancelled).unwrap(),
        r#""cancelled""#
    );
    both_ways(&a_round_info());
    both_ways(&RoundInfo {
        outcome: None,
        summary_head: None,
        ended: false,
        ..a_round_info()
    });
    // Final fix wave C-m3: a cancelled round not yet ended (decision 16).
    both_ways(&RoundInfo {
        outcome: Some(RoundOutcome::Cancelled),
        ended: false,
        ..a_round_info()
    });
    assert_eq!(serde_json::to_value(a_round_info()).unwrap()["ended"], true);
    let running: RoundInfo =
        serde_json::from_value(json!({"n": 3, "goal_head": "g", "origin": "user"})).unwrap();
    assert_eq!((running.outcome, running.summary_head), (None, None));
    assert!(!running.ended, "a round without `ended` has not ended");
    both_ways(&an_idle_orchestrator());
    both_ways(&IdleOrchestrator {
        outcome: RunState::Discarded,
        window_id: None,
        fresh: true,
        ..an_idle_orchestrator()
    });
    let bare: IdleOrchestrator = serde_json::from_value(json!({
        "chain": "o-3f9a", "project": "/r", "after_run": "r1", "outcome": "accepted",
        "runtime": "claude", "model": "m", "fresh": true
    }))
    .unwrap();
    assert_eq!((bare.window_id, bare.runs), (None, 0));
    both_ways(&a_round_line());
    assert_eq!(
        serde_json::to_value(a_round_line()).unwrap(),
        json!({
            "v": 5, "record_id": "run-a1b2/round/2", "at": 1_700_009_000u64,
            "run_id": "run-a1b2", "round": 2, "origin": "user", "outcome": "rejected",
            "tasks": 4, "merged": 3, "calls": 17, "minutes": 42
        })
    );
}

#[test]
fn iterate_request_round_trips() {
    assert_eq!(request::ITERATE, "run iterate");
    let iterate = RunRequest::Iterate {
        run: "run-a1b2".into(),
        goal: "also add a --verbose flag\nand document it".into(),
        design: None,
    };
    both_ways(&ClientMsg::Run(iterate.clone()));
    both_ways(&ClientMsg::RunTagged {
        id: 9,
        request: iterate,
    });
    let reply = RunReply::done(request::ITERATE, "run run-a1b2: round 2 started").tagged(Some(9));
    assert_eq!(reply.request_id(), Some(9));
    both_ways(&DaemonMsg::Run(reply));
}

fn a_goal(continue_from: Option<String>) -> RunRequest {
    RunRequest::StartGoal {
        goal: "add a flag".into(),
        dir: "/tmp/repo".into(),
        yes: false,
        trust_project: true,
        unconfined_checks: false,
        orchestrator: None,
        delivery: Some(DeliveryMode::Pr),
        continue_from,
        design: None,
    }
}

#[test]
fn start_goal_continue_from_defaults_to_none() {
    let old: RunRequest = serde_json::from_str(
        r#"{"StartGoal":{"goal":"add a flag","dir":"/tmp/repo","yes":false,
            "trust_project":true,"unconfined_checks":false,"orchestrator":null,
            "delivery":"pr"}}"#,
    )
    .expect("a protocol-14 StartGoal decodes");
    assert_eq!(old, a_goal(None));
    both_ways(&ClientMsg::Run(a_goal(None)));
    both_ways(&ClientMsg::Run(a_goal(Some("run-7c21".into()))));
    let json = serde_json::to_value(a_goal(Some("run-7c21".into()))).unwrap();
    assert_eq!(json["StartGoal"]["continue_from"], "run-7c21");
}

fn a_tool_call(chain: Option<String>) -> ToolCall {
    ToolCall {
        run_id: "run-a1b2".into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        window_id: 4,
        tool: "start_goal".into(),
        args: json!({"goal": "now make the flag configurable"}),
        scout_id: None,
        epic: None,
        chain,
        lane: None,
    }
}

#[test]
fn tool_call_chain_round_trips_and_defaults() {
    both_ways(&ClientMsg::Run(RunRequest::Tool(a_tool_call(Some(
        "o-3f9a".into(),
    )))));
    both_ways(&ClientMsg::Run(RunRequest::Tool(a_tool_call(None))));
    let json = serde_json::to_value(a_tool_call(Some("o-3f9a".into()))).unwrap();
    assert_eq!(json["chain"], "o-3f9a");
    let old: ToolCall = serde_json::from_value(json!({
        "run_id": "run-a1b2", "task_id": null, "role": "orchestrator", "window_id": 4,
        "tool": "start_goal", "args": {"goal": "now make the flag configurable"},
        "scout_id": null, "epic": null
    }))
    .expect("a protocol-14 ToolCall decodes");
    assert_eq!(old, a_tool_call(None));
}

#[test]
fn iterate_edit_round_trips() {
    let json = r#"{"op":"iterate","goal":"also add a --verbose flag"}"#;
    let edit: PlanEdit = serde_json::from_str(json).unwrap();
    assert_eq!(
        edit,
        PlanEdit::Iterate {
            goal: "also add a --verbose flag".into(),
        }
    );
    assert_eq!(serde_json::to_string(&edit).unwrap(), json);
    both_ways(&edit);
    assert!(serde_json::from_str::<PlanEdit>(r#"{"op":"iterate"}"#).is_err());
    assert!(
        serde_json::from_str::<PlanEdit>(r#"{"op":"iterate","goal":"g","submit":true}"#).is_err(),
        "deny_unknown_fields"
    );
}

/// `m912_run_info.json` is milestone 9.1's `RunInfo` from before milestone 9.0.6,
/// checked in by 9.0.6; it never had a round, a chain or an idle orchestrator.
#[test]
fn old_snapshot_still_decodes() {
    let run: RunInfo = serde_json::from_str(include_str!("m912_run_info.json"))
        .expect("a protocol-12 RunInfo decodes");
    assert_eq!(run.round, 1);
    assert!(run.rounds.is_empty());
    assert_eq!(run.chain, None);
    assert!(!run.tasks.is_empty() && !run.stages.is_empty());
    assert!(run.tasks.iter().all(|t| t.round == 1));
    assert!(run.stages.iter().all(|s| s.round == 1));

    let written = serde_json::to_value(&run).unwrap();
    assert!(
        written.get("rounds").is_none(),
        "one round writes no rounds"
    );
    assert!(written.get("chain").is_none());
    let packed: serde_json::Value =
        rmp_serde::from_slice(&rmp_serde::to_vec_named(&run).unwrap()).unwrap();
    assert!(packed.get("rounds").is_none() && packed.get("chain").is_none());
    assert_eq!(packed["round"], 1);

    let snapshot = RunsSnapshot {
        revision: 3,
        runs: vec![run.clone()],
        now: 1_700_000_000,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
    };
    let written = serde_json::to_value(&snapshot).unwrap();
    assert!(written.get("idle_orchestrators").is_none());
    let mut old = written.clone();
    old.as_object_mut().unwrap().remove("idle_orchestrators");
    let back: RunsSnapshot = serde_json::from_value(old).expect("a protocol-14 snapshot");
    assert!(back.idle_orchestrators.is_empty());
    assert_eq!(back, snapshot);

    let mut run = run;
    run.round = 2;
    run.rounds = vec![
        RoundInfo {
            n: 1,
            goal_head: "add a flag".into(),
            origin: RoundOrigin::User,
            outcome: Some(RoundOutcome::Completed),
            summary_head: None,
            ended: true,
        },
        RoundInfo {
            outcome: None,
            ended: false,
            ..a_round_info()
        },
    ];
    run.chain = Some("o-3f9a".into());
    run.tasks[0].round = 2;
    run.stages[0].round = 2;
    both_ways(&run);
    let snapshot = RunsSnapshot {
        runs: vec![run],
        idle_orchestrators: vec![an_idle_orchestrator()],
        ..snapshot
    };
    both_ways(&snapshot);
    both_ways(&DaemonMsg::Run(RunReply::Snapshot(snapshot.clone())));

    // Final fix wave C-m3: a snapshot written before `RoundInfo.ended` (9.3's own
    // earlier builds) still decodes, every round not ended; MessagePack likewise.
    let mut old = serde_json::to_value(&snapshot).unwrap();
    for round in old["runs"][0]["rounds"].as_array_mut().unwrap() {
        assert!(round.as_object_mut().unwrap().remove("ended").is_some());
    }
    let back: RunsSnapshot = serde_json::from_value(old.clone()).expect("no `ended`");
    assert!(back.runs[0].rounds.iter().all(|r| !r.ended));
    let packed = rmp_serde::to_vec_named(&old).unwrap();
    let back: RunsSnapshot = rmp_serde::from_slice(&packed).expect("no `ended`, packed");
    assert!(back.runs[0].rounds.iter().all(|r| !r.ended));
    assert_eq!(
        back.runs[0].rounds[0].outcome,
        Some(RoundOutcome::Completed)
    );
}

#[test]
fn round_history_line_round_trips() {
    assert_eq!(HISTORY_VERSION, 5);
    let line = HistoryLine::Round(a_round_line());
    let json = serde_json::to_string(&line).unwrap();
    assert!(json.starts_with(r#"{"type":"round","#), "{json}");
    both_ways(&line);
    both_ways(&HistoryLine::Round(RoundLine {
        origin: RoundOrigin::Orchestrator,
        outcome: RoundOutcome::Cancelled,
        ..a_round_line()
    }));

    // Version-1 and 2 lines (the fixture) and a version-4 line (inline) still decode.
    for line in include_str!("m9_history_v2.jsonl").lines() {
        serde_json::from_str::<HistoryLine>(line).expect("a version-1 or 2 line decodes");
    }
    let v4 = r#"{"type":"stage","v":4,"record_id":"r/stage/1","run_id":"r","stage":1,
        "pr":142,"time_to_open_secs":1,"human_review_secs":2,"ci_rounds":3,
        "review_rounds":4,"sync_tasks":5,"outcome":"merged","merge_method":"merge","at":9}"#;
    let HistoryLine::Stage(stage) = serde_json::from_str(v4).expect("a version-4 line") else {
        panic!("a stage line");
    };
    assert_eq!(stage.v, 4);

    let stats = HistoryStats {
        path: "/tmp/repo/history.jsonl".into(),
        task_records: 4,
        run_records: 2,
        rows: Vec::new(),
        decider_calls: 0,
        decider_fallbacks: 0,
        size_checked: 0,
        size_raised: 0,
        problems: Vec::new(),
        flaky_proposals: Vec::new(),
        window_days: 30,
        quarantine_after: 3,
        rounds: 5,
        iterated_runs: 2,
        tuning: None,
    };
    both_ways(&stats);
    let mut old = serde_json::to_value(&stats).unwrap();
    let map = old.as_object_mut().unwrap();
    map.remove("rounds");
    map.remove("iterated_runs");
    let old: HistoryStats = serde_json::from_value(old).expect("protocol-14 stats decode");
    assert_eq!((old.rounds, old.iterated_runs), (0, 0));
}

#[test]
fn appended_variants_keep_their_indices() {
    let names = tagged_names::<PlanEdit>("op");
    assert_eq!(
        names[names.len() - 7..names.len() - 5],
        ["reply_comment", "iterate"],
        "{names:?}"
    );
    // Milestone 9.6 appends `review_doc`, `phase`, `DocGate` and `ShowDoc` after these
    // (`design_tests.rs`).
    let names = tagged_names::<ActionKind>("kind");
    assert_eq!(
        names[names.len() - 3..names.len() - 1],
        ["open_conversation", "iterate"],
        "{names:?}"
    );
    let names = tagged_names::<HistoryLine>("type");
    assert_eq!(
        names[names.len() - 3..names.len() - 1],
        ["stage", "round"],
        "{names:?}"
    );
    let names = variant_names::<RunRequest>();
    // Milestone 9.5 appends `McpReady` after `Iterate` (`tuning_tests.rs`).
    assert_eq!(
        names[names.len() - 7..names.len() - 3],
        ["Deliver", "Watch", "Iterate", "McpReady"],
        "{names:?}"
    );
    let n = names.len() as u8 - 4;
    assert_eq!(
        variant_at::<RunRequest>(n - 1, &json!({"run": "r1", "goal": "more"})),
        Some(RunRequest::Iterate {
            run: "r1".into(),
            goal: "more".into(),
            design: None,
        })
    );
    assert_eq!(
        variant_at::<RunRequest>(n - 2, &json!({"run_id": "r1", "on": true})),
        Some(RunRequest::Watch {
            run_id: "r1".into(),
            on: true,
        })
    );
}

#[test]
fn iterate_action_opens_locally() {
    let kind = ActionKind::Iterate;
    assert_eq!(kind.needs(), ActionNeeds::Open);
    assert!(!kind.destructive());
    assert!(!kind.is_local(), "the daemon lists it");
    assert_eq!(
        serde_json::to_value(&kind).unwrap(),
        json!({"kind": "iterate"})
    );
    both_ways(&kind);
    both_ways(&ActionInfo {
        kind,
        label: "iterate".into(),
        effect: "start round 2 of this run".into(),
        needs: ActionNeeds::Open,
        destructive: false,
        refused_why: None,
    });
}
