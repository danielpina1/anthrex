//! Milestone 8c task 1: the snapshot fields the live run view reads, and the
//! placeholders later milestones fill. Every one is `#[serde(default)]`, so a snapshot
//! without them still decodes.

use super::fixtures::*;
use crate::messages::DaemonMsg;
use crate::run::{Effort, RouteSpec, Runtime, Strength};
use crate::run_info::{PlanEditInfo, RunsSnapshot, TaskEventInfo};
use crate::run_wire::RunReply;
use crate::{PlannerInfo, PlannerState};

fn a_planner() -> PlannerInfo {
    PlannerInfo {
        epic: "A".into(),
        title: "daemon".into(),
        area: vec!["crates/daemon/**".into()],
        route: a_route(Runtime::Claude, Strength::Frontier, Effort::High, ""),
        window_id: Some(12),
        state: PlannerState::Finished,
        started_at: 1_700_000_050,
        ended_at: Some(1_700_000_900),
        edits_accepted: 3,
        edits_rejected: 1,
        last_rejection: Some("t9 owns overlap t2".into()),
        replans: vec!["split t4".into()],
        note: None,
        covers: Vec::new(),
    }
}

/// Every new field set, each to a value no sibling of the same type has.
fn a_view_snapshot() -> RunsSnapshot {
    let mut run = a_run_info();
    run.approved_at = Some(1_700_000_010);
    run.plan_edits = vec![
        PlanEditInfo {
            at: 1_700_000_300,
            text: "split t2".into(),
            source: "user".into(),
            accepted: true,
            error: None,
            recipients: Vec::new(),
        },
        PlanEditInfo {
            at: 1_700_000_200,
            text: "amend t1".into(),
            source: "user".into(),
            accepted: true,
            error: None,
            recipients: Vec::new(),
        },
    ];
    run.plan_edits_since_approval = 2;
    run.planners = vec![a_planner()];
    run.estimate_left_secs = Some(1_800);
    run.bound_ratio_permille = Some(1_250);
    let task = &mut run.tasks[0];
    task.brief = "Add the reset token model.".into();
    task.acceptance = vec!["tokens expire after one hour".into()];
    task.route_spec = RouteSpec {
        runtime: Some(Runtime::Codex),
        model: Some("gpt-5-codex".into()),
        strength: None,
        effort: Some(Effort::Low),
    };
    task.history = vec![
        TaskEventInfo {
            at: 1_700_000_500,
            text: "review round 1 requested changes".into(),
        },
        TaskEventInfo {
            at: 1_700_000_400,
            text: "dispatched".into(),
        },
    ];
    let round = &mut task.rounds[0];
    round.rate_limited_since = Some(1_700_000_120);
    round.rate_limited_until = Some(1_700_000_420);
    round.sent_back_at = vec![1_700_000_150, 1_700_000_250];
    RunsSnapshot {
        revision: 42,
        runs: vec![run],
        now: 1_700_001_000,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
    }
}

#[test]
fn snapshot_view_fields_round_trip() {
    let msg = DaemonMsg::Run(RunReply::Snapshot(a_view_snapshot()));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    let back: DaemonMsg = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back, msg);
    let DaemonMsg::Run(RunReply::Snapshot(snap)) = back else {
        panic!("must decode back to RunReply::Snapshot");
    };
    // By name, so a swap between same-typed fields cannot survive (see run_tests.rs).
    assert_eq!(snap.now, 1_700_001_000);
    assert_eq!(snap.revision, 42);
    let run = &snap.runs[0];
    assert_eq!(run.approved_at, Some(1_700_000_010));
    assert_eq!(run.plan_edits[0].at, 1_700_000_300);
    assert_eq!(run.plan_edits[0].text, "split t2");
    assert_eq!(run.plan_edits[1].text, "amend t1");
    assert_eq!(run.plan_edits_since_approval, 2);
    assert_eq!(run.estimate_left_secs, Some(1_800));
    assert_eq!(run.bound_ratio_permille, Some(1_250));
    assert_eq!(run.planners, vec![a_planner()]);
    let planner = &run.planners[0];
    assert_eq!(
        (planner.started_at, planner.ended_at),
        (1_700_000_050, Some(1_700_000_900))
    );
    assert_eq!((planner.edits_accepted, planner.edits_rejected), (3, 1));
    let task = &run.tasks[0];
    assert_eq!(task.brief, "Add the reset token model.");
    assert_eq!(task.acceptance, ["tokens expire after one hour"]);
    assert_eq!(task.route_spec.strength, None);
    assert_eq!(task.route_spec.runtime, Some(Runtime::Codex));
    assert_eq!(task.route_spec.effort, Some(Effort::Low));
    assert_eq!(task.history[0].at, 1_700_000_500);
    assert_eq!(task.history[1].text, "dispatched");
    let round = &task.rounds[0];
    assert_eq!(round.rate_limited_since, Some(1_700_000_120));
    assert_eq!(round.rate_limited_until, Some(1_700_000_420));
    assert_eq!(round.sent_back_at, [1_700_000_150, 1_700_000_250]);
}

/// Removes `key` from `obj`, asserting it was there: the test must strip what an
/// earlier sender would not have sent, not keys that never existed.
fn strip(obj: &mut serde_json::Value, key: &str) {
    let map = obj.as_object_mut().expect("an object");
    assert!(map.remove(key).is_some(), "{key} is a field");
}

#[test]
fn snapshot_without_view_fields_defaults() {
    // M8b's round-trip shape: the snapshot as it was, with none of the new keys. Its
    // `history` held strings, which a protocol-9 peer does not read (the bump covers
    // the changed element type), so it goes too; the key itself defaults.
    let mut json = serde_json::to_value(a_view_snapshot()).unwrap();
    strip(&mut json, "now");
    for run in json["runs"].as_array_mut().unwrap() {
        for key in [
            "approved_at",
            "plan_edits",
            "plan_edits_since_approval",
            "planners",
            "estimate_left_secs",
            "bound_ratio_permille",
        ] {
            strip(run, key);
        }
        for task in run["tasks"].as_array_mut().unwrap() {
            for key in ["brief", "acceptance", "route_spec", "history"] {
                strip(task, key);
            }
            for round in task["rounds"].as_array_mut().unwrap() {
                for key in ["rate_limited_since", "rate_limited_until", "sent_back_at"] {
                    strip(round, key);
                }
            }
        }
    }
    let snap: RunsSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(snap.now, 0);
    let run = &snap.runs[0];
    assert_eq!(run.approved_at, None);
    assert!(run.plan_edits.is_empty());
    assert_eq!(run.plan_edits_since_approval, 0);
    assert!(run.planners.is_empty());
    assert_eq!(run.estimate_left_secs, None);
    assert_eq!(run.bound_ratio_permille, None);
    let task = &run.tasks[0];
    assert_eq!(task.brief, "");
    assert!(task.acceptance.is_empty());
    assert_eq!(task.route_spec, RouteSpec::default());
    assert!(task.history.is_empty());
    let round = &task.rounds[0];
    assert_eq!(round.rate_limited_since, None);
    assert_eq!(round.rate_limited_until, None);
    assert!(round.sent_back_at.is_empty());
}

#[test]
fn planner_state_is_snake_case_and_defaults_to_planning() {
    assert_eq!(PlannerState::default(), PlannerState::Planning);
    assert_eq!(
        serde_json::to_string(&PlannerState::Finished).unwrap(),
        "\"finished\""
    );
}

/// Milestone 9 task 2: `m8c_run_info.json` is `a_view_snapshot()`'s run as milestone
/// 8c's `RunInfo` serialized it (written from that type before milestone 9 changed it).
/// It decodes with every milestone-9 field at its default.
#[test]
fn old_run_info_still_decodes() {
    let run: crate::RunInfo =
        serde_json::from_str(include_str!("m8c_run_info.json")).expect("an M8c RunInfo decodes");
    assert_eq!(run.run_id, a_run_info().run_id);
    assert_eq!(run.orchestrator, None);
    assert!(run.holds.is_empty());
    assert!(run.integration.is_empty());
    assert_eq!(run.digest_revision, 0);
    assert_eq!(run.research_report, None);
    assert_eq!(run.planners[0].note, None);
    let edit = &run.plan_edits[0];
    assert_eq!(
        edit.source, "user",
        "decision 40: an entry without a source is the user's"
    );
    assert!(edit.accepted, "an M8c edit-log entry was an accepted batch");
    assert_eq!(edit.error, None);
    assert!(edit.recipients.is_empty());
    let task = &run.tasks[0];
    assert_eq!(task.hold, None);
    assert_eq!(task.review_target, None);
    assert_eq!(task.research_bytes, None);
    assert_eq!(task.message_count, 0);
    assert_eq!(task.last_message_kind, None);
    assert_eq!(task.last_message_line, None);
    assert!(task.task_notes.is_empty());
}

/// Milestone 9 task 2: every new snapshot field set survives the wire, by name.
#[test]
fn orchestrator_snapshot_fields_round_trip() {
    use crate::orch::*;
    let mut snapshot = a_view_snapshot();
    let run = &mut snapshot.runs[0];
    run.orchestrator = Some(OrchestratorInfo {
        route: a_route(
            Runtime::Claude,
            Strength::Frontier,
            Effort::High,
            "claude-opus-5",
        ),
        window_id: Some(21),
        live: true,
        started_at: 1_700_000_020,
        plan_submitted: true,
        summary: None,
        notes: vec!["t1 is blocked".into()],
        wakes: 2,
        wake_held: false,
    });
    run.holds = vec![HoldInfo {
        id: "h1".into(),
        kind: HoldKind::Promotion,
        state: HoldState::Awaiting,
        tasks: vec!["t1".into()],
        created_at: 1_700_000_030,
        decided_at: None,
        decided_by: None,
    }];
    run.integration = vec![IntegrationInfo {
        epic: "A".into(),
        state: IntegrationState::Reviewing,
        base: Some("abc1234".into()),
        merges: vec!["def5678".into()],
        tasks: vec!["A-int1".into()],
    }];
    run.digest_revision = 17;
    run.research_report = Some("/tmp/report.md".into());
    run.planners[0].note = Some("replanned once".into());
    run.plan_edits[0].source = "orchestrator".into();
    run.plan_edits[0].accepted = false;
    run.plan_edits[0].error = Some("t9 owns overlap t2".into());
    run.plan_edits[0].recipients = vec!["t1".into()];
    let task = &mut run.tasks[0];
    task.hold = Some("h1".into());
    task.review_target = Some("main..feature".into());
    task.research_bytes = Some(4_096);
    task.message_count = 3;
    task.last_message_kind = Some(MessageKind::Change);
    task.last_message_line = Some("the schema moved".into());
    task.task_notes = vec![TaskNoteInfo {
        task_id: "t1".into(),
        kind: TaskNoteKind::Risk,
        text: "shared parser".into(),
        at: 1_700_000_600,
    }];
    let msg = DaemonMsg::Run(RunReply::Snapshot(snapshot.clone()));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    let DaemonMsg::Run(RunReply::Snapshot(back)) = rmp_serde::from_slice(&packed).unwrap() else {
        panic!("must decode back to RunReply::Snapshot");
    };
    assert_eq!(back, snapshot);
    let (run, task) = (&back.runs[0], &back.runs[0].tasks[0]);
    assert_eq!(run.digest_revision, 17);
    assert_eq!(run.orchestrator.as_ref().unwrap().window_id, Some(21));
    assert_eq!(run.orchestrator.as_ref().unwrap().wakes, 2);
    assert_eq!(
        run.plan_edits[0].error.as_deref(),
        Some("t9 owns overlap t2")
    );
    assert_eq!(task.message_count, 3);
    assert_eq!(task.research_bytes, Some(4_096));
    assert_eq!(task.hold.as_deref(), Some("h1"));
    assert_eq!(task.review_target.as_deref(), Some("main..feature"));
}

/// Milestone 9.0.5 task 1: every new field set survives the wire, by name.
#[test]
fn new_fields_round_trip() {
    use crate::orch::OrchestratorInfo;
    use crate::profile::ProposalAlertInfo;
    let mut snapshot = a_view_snapshot();
    snapshot.proposals = vec![
        ProposalAlertInfo {
            project: "/tmp/calc".into(),
            updated_at: 1_700_000_700,
        },
        ProposalAlertInfo {
            project: "/tmp/web".into(),
            updated_at: 1_700_000_800,
        },
    ];
    let run = &mut snapshot.runs[0];
    run.orchestrator = Some(OrchestratorInfo {
        route: a_route(Runtime::Codex, Strength::Frontier, Effort::High, "gpt-5"),
        window_id: Some(4),
        live: true,
        started_at: 1_700_000_020,
        plan_submitted: false,
        summary: None,
        notes: Vec::new(),
        wakes: 0,
        wake_held: true,
    });
    run.tasks[0].activity = Some("Bash cargo test -p calc".into());
    let msg = DaemonMsg::Run(RunReply::Snapshot(snapshot.clone()));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    let DaemonMsg::Run(RunReply::Snapshot(back)) = rmp_serde::from_slice(&packed).unwrap() else {
        panic!("must decode back to RunReply::Snapshot");
    };
    assert_eq!(back, snapshot);
    let json: RunsSnapshot =
        serde_json::from_str(&serde_json::to_string(&snapshot).unwrap()).unwrap();
    assert_eq!(json, snapshot);
    assert_eq!(back.proposals.len(), 2);
    assert_eq!(back.proposals[1].project, std::path::Path::new("/tmp/web"));
    assert_eq!(back.proposals[1].updated_at, 1_700_000_800);
    let run = &back.runs[0];
    assert!(run.orchestrator.as_ref().unwrap().wake_held);
    assert_eq!(
        run.tasks[0].activity.as_deref(),
        Some("Bash cargo test -p calc")
    );

    let mut window: crate::WindowInfo = serde_json::from_value(m9_window_json()).unwrap();
    window.signals_seen = true;
    let packed = rmp_serde::to_vec_named(&window).unwrap();
    let back: crate::WindowInfo = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back, window);
    assert!(back.signals_seen);
    let json = serde_json::to_string(&window).unwrap();
    assert_eq!(
        serde_json::from_str::<crate::WindowInfo>(&json).unwrap(),
        window
    );
}

/// A `WindowInfo` as milestone 9 serialized it: no `signals_seen`.
fn m9_window_json() -> serde_json::Value {
    serde_json::json!({
        "id": 7, "name": "orchestrator", "runtime": "claude",
        "cwd": "/tmp/repo", "project": "/tmp/repo", "worktree": null, "branch": null,
        "status": "idle", "tool": null, "since_secs": 12,
        "last_output_secs": 1, "session_id": null, "model": null,
        "subagents": [], "exit": null, "kind": "pty",
        "run": {"run_id": "run-a1b2", "task_id": null, "role": "orchestrator", "session": 1}
    })
}

/// Milestone 9.0.5 task 1: `m9_run_info.json` is milestone 9's
/// `orchestrator_snapshot_fields_round_trip` run as milestone 9's `RunInfo` serialized
/// it (written from `main`'s types at `566a653`, before this milestone changed them). It,
/// a snapshot holding it, and a milestone-9 window decode with every new field at its
/// default.
#[test]
fn m9_snapshot_and_window_decode_with_defaults() {
    let run: crate::RunInfo =
        serde_json::from_str(include_str!("m9_run_info.json")).expect("an M9 RunInfo decodes");
    assert_eq!(run.run_id, a_run_info().run_id);
    let orchestrator = run.orchestrator.as_ref().expect("the M9 run has one");
    assert_eq!(orchestrator.window_id, Some(21));
    assert!(!orchestrator.wake_held);
    assert!(!run.tasks.is_empty());
    for task in &run.tasks {
        assert_eq!(task.activity, None);
    }
    let snapshot = format!(
        r#"{{"revision": 3, "runs": [{}], "now": 1700001000}}"#,
        include_str!("m9_run_info.json")
    );
    let snapshot: RunsSnapshot = serde_json::from_str(&snapshot).expect("an M9 snapshot");
    assert!(snapshot.proposals.is_empty());
    assert_eq!(snapshot.runs[0], run);
    let packed = rmp_serde::to_vec_named(&snapshot).unwrap();
    assert_eq!(
        rmp_serde::from_slice::<RunsSnapshot>(&packed).unwrap(),
        snapshot
    );

    let window: crate::WindowInfo =
        serde_json::from_value(m9_window_json()).expect("an M9 window decodes");
    assert!(!window.signals_seen);
    assert_eq!(window.id, 7);
}
