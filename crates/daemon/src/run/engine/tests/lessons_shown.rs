//! Task M9.15.6 (decision 24): each worker launch records the lessons its first turn
//! showed, once each, and the task's history record carries them.

use proto::{LessonLayer, LessonStatus, TaskOutcome};

use super::carried_messages::{done_fails, fresh_first_turn};
use super::fixture::*;
use crate::run::contract_lessons::WORKER_HEADING;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{LessonBrief, LessonLimits, Run, RunLessons};

const NOW: u64 = 1_800_000_000;

fn lesson(id: &str, files: &[&str]) -> LessonBrief {
    LessonBrief {
        id: id.into(),
        layer: LessonLayer::Episodic,
        status: LessonStatus::OnTrial,
        title: format!("title of {id}"),
        body: format!("body of {id}"),
        files: files.iter().map(|f| f.to_string()).collect(),
        last_seen: NOW,
        usefulness: None,
    }
}

/// A `t1` owning `crates/a/**` whose done gate can fail, in a run frozen with three
/// lessons: two on its files, and one on other files last seen 90 days ago (no
/// overlap and no recency: 175 000, under `MIN_SCORE`).
fn working() -> (Fixture, u32, String) {
    let profile = PROFILE.replace("setup = ", "generated = [\"Cargo.lock\"]\nsetup = ");
    let roomy = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
    let mut fx = Fixture::new(&plan_with(&profile, &[task("t1", "S", "a", roomy)]));
    let lessons = RunLessons {
        limits: LessonLimits::default(),
        items: vec![
            lesson("l-near-b", &["crates/a/src/**"]),
            lesson("l-near-a", &["crates/a/**"]),
            LessonBrief {
                last_seen: NOW - 90 * 86_400,
                ..lesson("l-far", &["crates/zz/**"])
            },
        ],
        frozen_at: NOW,
    };
    fx.start_with(true, |run| run.lessons = Some(lessons));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.complete_prepares();
    let first_turn = (fx.run().pending_ops.values())
        .find_map(|p| match &p.kind {
            OpKind::CreateWindow { first_turn, .. } => Some(first_turn.clone()),
            _ => None,
        })
        .expect("t1's worker is launching");
    let window = fx.complete_windows()[0].1;
    (fx, window, first_turn)
}

#[test]
fn launch_records_picked_ids_once() {
    let (mut fx, window, first_turn) = working();
    assert!(first_turn.contains(WORKER_HEADING), "{first_turn}");
    assert!(first_turn.contains("[l-near-a · episodic]"), "{first_turn}");
    assert!(!first_turn.contains("l-far"), "{first_turn}");
    assert_eq!(fx.task("t1").lessons_shown, ["l-near-a", "l-near-b"]);

    // Rung 2: a fresh session shows the same lessons; the set does not grow.
    done_fails(&mut fx, window);
    done_fails(&mut fx, window);
    assert_eq!(fx.task("t1").rung, 2);
    let fresh = fresh_first_turn(&mut fx, window);
    assert!(fresh.contains("[l-near-b · episodic]"), "{fresh}");
    assert_eq!(fx.task("t1").session, 2);
    assert_eq!(fx.task("t1").lessons_shown, ["l-near-a", "l-near-b"]);
}

#[test]
fn a_run_without_lessons_records_none() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    fx.launch_all();
    assert!(fx.task("t1").lessons_shown.is_empty());
}

#[test]
fn task_record_carries_lessons_shown() {
    let (fx, _, _) = working();
    let run: &Run = fx.run();
    let record = crate::run::history::task_record(run, &run.tasks[0], TaskOutcome::Blocked, NOW);
    assert_eq!(record.lessons_shown, ["l-near-a", "l-near-b"]);
}

/// The protocol-23 fixture (task M9.15.1) loads with lessons off: no frozen lessons and
/// no task that showed any.
#[test]
fn m914_run_loads_with_empty_lessons() {
    let text = include_str!("../../../../tests/fixtures/run/m914-run.json");
    let run: Run = serde_json::from_str(text).expect("m914-run.json parses");
    assert_eq!(run.lessons, None);
    assert!(run.tasks.iter().all(|t| t.lessons_shown.is_empty()));
    // Written again, neither key appears (ruling C2).
    let written = serde_json::to_string(&run).unwrap();
    assert!(!written.contains("\"lessons\""), "{written}");
    assert!(!written.contains("lessons_shown"), "{written}");
}
