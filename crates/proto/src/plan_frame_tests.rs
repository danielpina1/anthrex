//! Task M9.17.2: protocol 26's types (decisions 6, 8, 13, 20, 22, 26, 28), each
//! round-tripped through JSON and MessagePack (`to_vec_named`), and each decoded from
//! the shape protocol 25 wrote.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{Value, json};

use crate::delivery::tests::variant_names;
use crate::*;

/// M8a's round-trip fixtures, loaded as this module's own private copy. `a_route` is
/// imported for `adapt_fixtures`, which reads it through `use super::*`.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "run_tests_fixtures.rs"]
mod fixtures;
use fixtures::{a_finding, a_route, a_run_info, a_task_info, a_token_usage};

/// Milestone 8b's builders, for the history records.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "adapt_tests_fixtures.rs"]
mod adapt_fixtures;
use adapt_fixtures::{a_stats, a_task_record};

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

/// `value` without the key `key`, as protocol 25 wrote it.
fn without(mut value: Value, key: &str) -> Value {
    value.as_object_mut().unwrap().remove(key);
    value
}

fn a_frame() -> PlanFrameInfo {
    PlanFrameInfo {
        decided: vec![DecidedItem {
            question: "which crate owns the parser?".into(),
            answer: "crates/proto, beside the codec".into(),
        }],
        out_of_scope: vec!["a new wire format".into(), "the CLI's help text".into()],
        spec_non_goals: Some("- No GUI.\n- No Windows support.".into()),
    }
}

fn a_plan_task(id: &str) -> PlanTask {
    PlanTask {
        id: id.into(),
        title: "A task".into(),
        epic: None,
        kind: TaskKind::Code,
        size: Size::S,
        interface_change: false,
        test_mode: Some(TestMode::Tdd),
        test_mode_reason: None,
        owns: vec!["crates/x/**".into()],
        deps: vec![],
        priority: 0,
        brief: "do the thing".into(),
        acceptance: vec!["it works".into()],
        test_to_write: None,
        scout_refs: vec![],
        route: RouteSpec::default(),
        budget: None,
        review_target: None,
        stage: 1,
        atomic: false,
        atomic_reason: None,
        addresses: Vec::new(),
        race: false,
        pair: false,
        covers: Vec::new(),
    }
}

fn history_lines(text: &str) -> Vec<HistoryLine> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{e}: {l}")))
        .collect()
}

#[test]
fn finding_target_defaults_to_code_and_is_left_out() {
    let code = a_finding();
    assert_eq!(code.target, FindingTarget::Code);
    assert!(code.target.is_code());
    let wire = serde_json::to_value(&code).unwrap();
    assert!(wire.get("target").is_none(), "{wire}");
    let old: Finding = serde_json::from_value(without(wire, "target")).unwrap();
    assert_eq!(old.target, FindingTarget::Code);
    both(&code);

    let brief = Finding {
        target: FindingTarget::Brief,
        file: None,
        line: None,
        input: None,
        ..a_finding()
    };
    assert!(!brief.target.is_code());
    let wire = serde_json::to_value(&brief).unwrap();
    assert_eq!(wire["target"], json!("brief"));
    both(&brief);
    assert_eq!(FindingTarget::default(), FindingTarget::Code);
    assert_eq!(
        serde_json::to_value(FindingTarget::Code).unwrap(),
        json!("code")
    );
}

#[test]
fn task_note_kind_brief_is_appended_last() {
    // `TaskNoteKind` is `snake_case`, so serde names the variant `brief`.
    let names = variant_names::<TaskNoteKind>();
    assert_eq!(names, ["discovery", "risk", "progress", "brief"]);
    assert_eq!(
        serde_json::to_string(&TaskNoteKind::Brief).unwrap(),
        "\"brief\""
    );
    let note = TaskNoteInfo {
        task_id: "t3".into(),
        kind: TaskNoteKind::Brief,
        text: "review round 1 (important): the brief asks for a GUI".into(),
        at: 1_791_890_300,
    };
    both(&note);
    both(&TaskNoteKind::Brief);
}

#[test]
fn plan_frame_info_round_trips_and_is_left_out_when_empty() {
    let frame = a_frame();
    both(&frame);
    let wire = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        wire["decided"][0]["question"],
        json!("which crate owns the parser?")
    );
    both(&PlanFrameInfo::default());
    assert_eq!(
        serde_json::to_value(PlanFrameInfo::default()).unwrap(),
        json!({})
    );

    let mut run = a_run_info();
    assert_eq!(run.plan_frame, None);
    let wire = serde_json::to_value(&run).unwrap();
    assert!(wire.get("plan_frame").is_none());
    let old: RunInfo = serde_json::from_value(without(wire, "plan_frame")).unwrap();
    assert_eq!(old.plan_frame, None);
    run.plan_frame = Some(a_frame());
    both(&run);

    let mut planner = PlannerInfo {
        epic: "A".into(),
        title: "daemon".into(),
        area: vec!["crates/daemon/**".into()],
        route: a_route(Runtime::Claude, Effort::HIGH, ""),
        window_id: Some(12),
        state: PlannerState::Finished,
        started_at: 1_700_000_050,
        ended_at: Some(1_700_000_900),
        edits_accepted: 3,
        edits_rejected: 1,
        last_rejection: None,
        replans: vec![],
        note: None,
        covers: Vec::new(),
        frame: None,
    };
    let wire = serde_json::to_value(&planner).unwrap();
    assert!(wire.get("frame").is_none(), "{wire}");
    let old: PlannerInfo = serde_json::from_value(without(wire, "frame")).unwrap();
    assert_eq!(old.frame, None);
    planner.frame = Some(a_frame());
    both(&planner);

    // A protocol-25 snapshot still loads.
    let old: RunInfo = serde_json::from_str(include_str!("m912_run_info.json")).unwrap();
    assert_eq!(old.plan_frame, None);
    assert!(old.planners.iter().all(|p| p.frame.is_none()));
}

#[test]
fn add_task_replaces_is_optional() {
    let task = serde_json::to_value(a_plan_task("t9")).unwrap();
    let old: PlanEdit = serde_json::from_value(json!({"op": "add_task", "task": task})).unwrap();
    assert_eq!(old, PlanEdit::add_task(a_plan_task("t9")));
    match &old {
        PlanEdit::AddTask { replaces, .. } => assert!(replaces.is_empty()),
        other => panic!("{other:?}"),
    }
    let wire = serde_json::to_value(PlanEdit::add_task(a_plan_task("t9"))).unwrap();
    assert!(wire.get("replaces").is_none(), "{wire}");
    both(&PlanEdit::add_task(a_plan_task("t9")));

    let replacing = PlanEdit::AddTask {
        task: a_plan_task("t9"),
        replaces: vec!["t1".into()],
    };
    let wire = serde_json::to_value(&replacing).unwrap();
    assert_eq!(wire["replaces"], json!(["t1"]));
    both(&replacing);

    let other = serde_json::from_value::<PlanEdit>(
        json!({"op": "cancel_task", "task_id": "t1", "replaces": ["t2"]}),
    );
    assert!(other.is_err(), "{other:?}");
}

#[test]
fn task_info_replaces_and_replaced_by_are_optional() {
    let mut task = a_task_info();
    assert!(task.replaces.is_empty() && task.replaced_by.is_empty());
    let wire = serde_json::to_value(&task).unwrap();
    assert!(wire.get("replaces").is_none(), "{wire}");
    assert!(wire.get("replaced_by").is_none(), "{wire}");
    let old: TaskInfo =
        serde_json::from_value(without(without(wire, "replaces"), "replaced_by")).unwrap();
    assert_eq!(old, task);
    task.replaces = vec!["t3".into()];
    task.replaced_by = vec!["t9".into(), "t10".into()];
    let wire = serde_json::to_value(&task).unwrap();
    assert_eq!(wire["replaces"], json!(["t3"]));
    assert_eq!(wire["replaced_by"], json!(["t9", "t10"]));
    both(&task);
}

#[test]
fn task_record_replaces_and_brief_findings_default() {
    assert_eq!(HISTORY_VERSION, 6);
    for text in [
        include_str!("m9_history_v2.jsonl"),
        include_str!("m93_history.jsonl"),
        include_str!("m914_history.jsonl"),
    ] {
        let tasks: Vec<TaskRecord> = history_lines(text)
            .into_iter()
            .filter_map(|line| match line {
                HistoryLine::Task(record) => Some(record),
                _ => None,
            })
            .collect();
        assert!(!tasks.is_empty());
        for record in tasks {
            assert!(record.replaces.is_empty());
            assert_eq!(record.brief_findings, 0);
        }
    }

    let mut record = a_task_record();
    let wire = serde_json::to_value(&record).unwrap();
    assert!(wire.get("replaces").is_none(), "{wire}");
    assert!(wire.get("brief_findings").is_none(), "{wire}");
    record.replaces = vec!["t3".into()];
    record.brief_findings = 2;
    let wire = serde_json::to_value(&record).unwrap();
    assert_eq!(wire["replaces"], json!(["t3"]));
    assert_eq!(wire["brief_findings"], json!(2));
    both(&record);
    both(&HistoryLine::Task(record));

    let mut stats = a_stats();
    let wire = serde_json::to_value(&stats).unwrap();
    assert!(wire.get("replaced").is_none(), "{wire}");
    let old: HistoryStats = serde_json::from_value(without(wire, "replaced")).unwrap();
    assert_eq!(old.replaced, 0);
    stats.replaced = 3;
    assert_eq!(serde_json::to_value(&stats).unwrap()["replaced"], json!(3));
    both(&stats);
}
