//! Milestone 9.5 task 2, continued from `tuning_tests.rs`: every new or changed message
//! round-trips, untagged and tagged; what protocol 15 sent still decodes; and the
//! appended variants keep their indices.

use super::*;
use crate::delivery::tests::{tagged_names, variant_at, variant_names};
use crate::run_wire::{RunReply, request};

/// Untagged (`ClientMsg::Run`) and tagged (`ClientMsg::RunTagged`).
fn request_both_ways(request: RunRequest) {
    both_ways(&ClientMsg::Run(request.clone()));
    both_ways(&ClientMsg::RunTagged { id: 16, request });
}

/// Untagged and tagged (`request_id: Some`).
fn reply_both_ways(reply: RunReply) {
    both_ways(&DaemonMsg::Run(reply.clone()));
    let snapshot = matches!(reply, RunReply::Snapshot(_));
    let tagged = reply.tagged(Some(16));
    assert_eq!(tagged.request_id(), (!snapshot).then_some(16));
    both_ways(&DaemonMsg::Run(tagged));
}

fn a_lane(lane: RaceLane, state: LaneState) -> LaneInfo {
    LaneInfo {
        lane,
        route: a_route(Runtime::Codex, Strength::Standard, Effort::Medium, "gpt-6"),
        state,
        checkout: format!("t1.{}", lane.label()),
        head: Some("d1d1d1d".into()),
        reason: Some("check failed twice".into()),
        salvage_ref: Some("refs/anthrex/salvage/r/t1/1".into()),
    }
}

fn a_tuning_report() -> TuningReport {
    let budget = Budget {
        tool_calls: 40,
        minutes: 15,
        tokens: None,
    };
    let class = |name: &str, refit: RefitState| ClassTuning {
        class: name.into(),
        samples: 34,
        budget,
        refit,
        configured: matches!(refit, RefitState::Configured),
        refit_budget: Some(Budget {
            tool_calls: 55,
            minutes: 18,
            tokens: Some(1_000),
        }),
        weight_secs: Some(550),
        weight_derived: true,
        route: "standard/low".into(),
    };
    TuningReport {
        path: PathBuf::from("/tmp/data/repos/p-1234/tuning.toml"),
        min_samples: 30,
        refit_budgets: true,
        classes: vec![
            class("S", RefitState::NotYet),
            class("S", RefitState::Kept),
            class("M", RefitState::Written { at: 1_790_500_000 }),
            class("hub", RefitState::Configured),
            class("hub", RefitState::Off),
        ],
        proposals: vec![
            TuningProposal {
                id: "threshold-s".into(),
                text: "S tasks reach 35 lines".into(),
                current: "20".into(),
                proposed: "35".into(),
                change: TuningChange::Threshold {
                    class: "S".into(),
                    lines: 35,
                },
            },
            TuningProposal {
                id: "route-m".into(),
                text: "M tasks escalate".into(),
                current: "standard/medium".into(),
                proposed: "frontier/high".into(),
                change: TuningChange::Route {
                    class: "M".into(),
                    route: ClassRoute {
                        strength: Strength::Frontier,
                        effort: Effort::High,
                    },
                },
            },
        ],
        moved_bad_file: Some(PathBuf::from("/tmp/data/repos/p-1234/tuning.toml.bad-1")),
        parse_error: Some("TOML parse error at line 1, column 1".into()),
        applied: vec!["route-m".into()],
        dismissed: vec!["threshold-s".into()],
        orchestrator_list: Some("codex/gpt-6.1-sol high".into()),
        project: Some(PathBuf::from("/r/demo")),
    }
}

/// A window as protocol 15 described it: no `placeholder`, and a `run` with no `lane`.
fn p15_window() -> serde_json::Value {
    json!({
        "id": 3, "name": "3f9a/t1.w1", "runtime": "claude", "cwd": "/tmp/x",
        "project": "/tmp/p", "worktree": null, "branch": null, "status": "working",
        "tool": null, "since_secs": 1, "last_output_secs": 1, "session_id": null,
        "model": null, "subagents": [], "exit": null, "kind": "headless",
        "run": {"run_id": "r", "task_id": "t1", "role": "worker", "session": 1},
        "signals_seen": false
    })
}

/// Protocol-15 MessagePack bytes of `value`: a named map, as `codec::encode` writes a
/// struct (`rmp_serde::to_vec_named`), with only the keys protocol 15 had.
fn p15_bytes(value: &serde_json::Value) -> Vec<u8> {
    rmp_serde::to_vec_named(value).unwrap()
}

fn a_window(placeholder: bool) -> WindowInfo {
    let mut window: WindowInfo =
        rmp_serde::from_slice(&p15_bytes(&p15_window())).expect("a protocol-15 WindowInfo");
    assert!(!window.placeholder, "absent reads false");
    window.placeholder = placeholder;
    window
}

#[test]
fn every_new_or_changed_message_round_trips() {
    request_both_ways(RunRequest::Stats {
        dir: "/tmp/repo".into(),
        apply: vec!["threshold-s".into(), "route-m".into()],
        dismiss: vec!["route-s".into()],
        read_only: true,
    });
    request_both_ways(RunRequest::McpReady {
        run_id: "run-a1b2".into(),
        window_id: 7,
    });
    assert_eq!(request::MCP_READY, "mcp ready");
    reply_both_ways(RunReply::done(request::MCP_READY, ""));

    let stats = HistoryStats {
        tuning: Some(Box::new(a_tuning_report())),
        ..a_stats()
    };
    reply_both_ways(RunReply::Stats {
        stats,
        request_id: None,
    });

    // A snapshot with a race, a pair, a lane round, a failed turn and a lane review.
    let mut task = a_task_info();
    task.race = Some(RaceInfo {
        lanes: vec![
            a_lane(RaceLane::A, LaneState::Won),
            a_lane(RaceLane::B, LaneState::Lost),
        ],
        winner: Some(RaceLane::A),
        adopted: false,
    });
    task.pair = Some(PairInfo {
        phase: PairPhase::Implementing,
        writer_route: a_route(Runtime::Claude, Strength::Standard, Effort::Low, "s"),
        test: Some("a::works".into()),
        red: Some("abcdef1".into()),
        red_checked: Some(true),
        writer_failures: 1,
    });
    task.rounds = vec![AgentRoundInfo {
        role: AgentRole::Racer,
        lane: Some(RaceLane::B),
        failed_error: Some("overloaded".into()),
        failed_until: Some(1_700_000_700),
        ..an_agent_round()
    }];
    task.reviews = vec![ReviewInfo {
        lane: Some(RaceLane::A),
        ..a_review()
    }];
    let mut run = a_run_info();
    run.tasks = vec![task];
    run.writer_caps = BTreeMap::from([("claude".to_string(), 1u8)]);
    let captured: RunInfo = serde_json::from_str(include_str!("m93_run_info.json")).unwrap();
    let mut stage = captured.stages[0].clone();
    stage.full.held = true;
    run.stages = vec![stage];
    let snapshot = RunsSnapshot {
        revision: 4,
        runs: vec![run],
        now: 1_700_000_800,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
    };
    reply_both_ways(RunReply::Snapshot(snapshot));
    both_ways(&DaemonMsg::WindowsChanged {
        windows: vec![a_window(true), a_window(false)],
    });
    both_ways(&FullInfo {
        held: true,
        ..FullInfo::default()
    });

    let call = ToolCall {
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        role: AgentRole::Racer,
        window_id: 9,
        tool: "task_done".into(),
        args: json!({"summary": "s"}),
        scout_id: None,
        epic: None,
        chain: None,
        lane: Some(RaceLane::B),
    };
    request_both_ways(RunRequest::Tool(call.clone()));
    assert_eq!(serde_json::to_value(&call).unwrap()["lane"], "b");

    both_ways(&RunRef {
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        role: AgentRole::TestWriter,
        session: 1,
        lane: None,
    });
    both_ways(&RunRef {
        run_id: "run-a1b2".into(),
        task_id: Some("t1".into()),
        role: AgentRole::Racer,
        session: 2,
        lane: Some(RaceLane::A),
    });
    let amend: PlanEdit = serde_json::from_value(
        json!({"op": "amend_task", "task_id": "t1", "race": true, "pair": false}),
    )
    .unwrap();
    both_ways(&amend);
    let file = EditFile { edits: vec![amend] };
    let text = toml::to_string(&file).unwrap();
    assert_eq!(toml::from_str::<EditFile>(&text).unwrap(), file, "{text}");
}

#[test]
fn old_messages_decode_with_defaults() {
    let ClientMsg::Run(RunRequest::Stats {
        dir,
        apply,
        dismiss,
        read_only,
    }) = serde_json::from_value(json!({"Run": {"Stats": {"dir": "/tmp/repo"}}})).unwrap()
    else {
        panic!("a Stats request");
    };
    assert_eq!(dir, PathBuf::from("/tmp/repo"));
    assert!(apply.is_empty() && dismiss.is_empty() && !read_only);
    // MessagePack, as a protocol-15 client sends it.
    let packed = rmp_serde::to_vec_named(&json!({"Run": {"Stats": {"dir": "/tmp/repo"}}})).unwrap();
    let ClientMsg::Run(RunRequest::Stats { read_only, .. }) =
        rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("a Stats request");
    };
    assert!(!read_only);

    let call: ToolCall = serde_json::from_value(json!({
        "run_id": "run-a1b2", "task_id": "t1", "role": "worker", "window_id": 4,
        "tool": "task_done", "args": {}, "scout_id": null, "epic": null, "chain": null
    }))
    .expect("a protocol-15 ToolCall decodes");
    assert_eq!(call.lane, None);
    let reference: RunRef = serde_json::from_value(
        json!({"run_id": "r", "task_id": "t1", "role": "worker", "session": 1}),
    )
    .unwrap();
    assert_eq!(reference.lane, None);

    for (name, text) in [
        ("m93_run_info.json", include_str!("m93_run_info.json")),
        ("m912_run_info.json", include_str!("m912_run_info.json")),
    ] {
        let run: RunInfo =
            serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} decodes: {e}"));
        assert!(run.writer_caps.is_empty(), "{name}");
        assert!(!run.tasks.is_empty(), "{name}");
        for task in &run.tasks {
            assert_eq!((&task.race, &task.pair), (&None, &None), "{name}");
            for round in &task.rounds {
                assert_eq!(round.lane, None, "{name}");
                assert_eq!((&round.failed_error, round.failed_until), (&None, None));
            }
            assert!(task.reviews.iter().all(|r| r.lane.is_none()), "{name}");
        }
        assert!(run.stages.iter().all(|s| !s.full.held), "{name}");
    }
    let captured: RunInfo = serde_json::from_str(include_str!("m93_run_info.json")).unwrap();
    assert!(
        captured.tasks.iter().any(|t| !t.reviews.is_empty())
            && captured.tasks.iter().any(|t| t.rounds.len() > 1)
            && !captured.stages.is_empty(),
        "the 9.3 fixture has a review, rounds and a stage"
    );

    // A TaskInfo without a race or pair writes neither key.
    let json = serde_json::to_value(&captured.tasks[0]).unwrap();
    assert!(
        json.get("race").is_none() && json.get("pair").is_none(),
        "{json}"
    );

    // Protocol-15 MessagePack: a window list as a 15 daemon sends it, and a stage's
    // full suite.
    let message = json!({"WindowsChanged": {"windows": [p15_window()]}});
    let DaemonMsg::WindowsChanged { windows } =
        rmp_serde::from_slice(&p15_bytes(&message)).expect("a protocol-15 WindowsChanged")
    else {
        panic!("a WindowsChanged");
    };
    assert!(!windows[0].placeholder);
    assert_eq!(windows[0].run.as_ref().map(|r| r.lane), Some(None));
    let full = json!({
        "state": "red", "at": null, "secs": null, "commit": null, "shards": 1,
        "flaky": [], "failing": [], "bisect_fixes": 0, "note": null
    });
    let full: FullInfo = rmp_serde::from_slice(&p15_bytes(&full)).expect("a protocol-15 FullInfo");
    assert!(!full.held);
    assert_eq!(full.state, crate::tiers::FullState::Red);
}

#[test]
fn appended_variants_keep_their_indices() {
    // A unit variant also decodes from its index in MessagePack.
    for (index, role) in [(6u8, AgentRole::Racer), (7, AgentRole::TestWriter)] {
        assert_eq!(
            rmp_serde::from_slice::<AgentRole>(&[index]).ok(),
            Some(role)
        );
    }
    assert!(
        rmp_serde::from_slice::<AgentRole>(&[8]).is_err(),
        "no role after"
    );
    let roles = variant_names::<AgentRole>();
    assert_eq!(
        roles,
        [
            "orchestrator",
            "worker",
            "reviewer",
            "scout",
            "planner",
            "decider",
            "racer",
            "test_writer"
        ]
    );
    let names = variant_names::<RunRequest>();
    assert_eq!(
        names[names.len() - 3..],
        ["Watch", "Iterate", "McpReady"],
        "{names:?}"
    );
    let n = names.len() as u8;
    assert_eq!(
        variant_at::<RunRequest>(n - 1, &json!({"run_id": "r1", "window_id": 3})),
        Some(RunRequest::McpReady {
            run_id: "r1".into(),
            window_id: 3,
        })
    );
    assert_eq!(
        variant_at::<RunRequest>(n - 2, &json!({"run": "r1", "goal": "more"})),
        Some(RunRequest::Iterate {
            run: "r1".into(),
            goal: "more".into(),
        })
    );
    let names = tagged_names::<PlanEdit>("op");
    assert_eq!(names[names.len() - 1], "iterate", "{names:?}");
}

/// Ruling T8-2: a moved bad file's parse error rides `TuningReport.parse_error`, absent
/// from what the daemon sent before (MessagePack without the key decodes as `None`),
/// and not written while unset.
#[test]
fn a_tuning_report_without_its_parse_error_still_decodes() {
    let mut value = serde_json::to_value(a_tuning_report()).unwrap();
    assert_eq!(value["parse_error"], "TOML parse error at line 1, column 1");
    value.as_object_mut().unwrap().remove("parse_error");
    let report: TuningReport =
        rmp_serde::from_slice(&p15_bytes(&value)).expect("a report without parse_error");
    assert_eq!(report.parse_error, None);
    assert_eq!(report.moved_bad_file, a_tuning_report().moved_bad_file);
    let unset = TuningReport {
        parse_error: None,
        ..a_tuning_report()
    };
    let json = serde_json::to_value(&unset).unwrap();
    assert!(json.get("parse_error").is_none(), "{json}");
    both_ways(&unset);
}

/// Task M9.5.11's fix round: `TuningReport.project`, absent from what the daemon sent
/// before, decodes as `None` and is not written while unset.
#[test]
fn a_tuning_report_without_its_project_still_decodes() {
    let mut value = serde_json::to_value(a_tuning_report()).unwrap();
    assert_eq!(value["project"], "/r/demo");
    value.as_object_mut().unwrap().remove("project");
    let report: TuningReport =
        rmp_serde::from_slice(&p15_bytes(&value)).expect("a report without project");
    assert_eq!(report.project, None);
    let unset = TuningReport {
        project: None,
        ..a_tuning_report()
    };
    let json = serde_json::to_value(&unset).unwrap();
    assert!(json.get("project").is_none(), "{json}");
    both_ways(&unset);
}
