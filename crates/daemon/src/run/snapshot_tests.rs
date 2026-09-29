//! Task M9.6: the snapshot's plan text only at the gate, and its bounded task notes
//! (decisions 16a, 42d). Pure.

use proto::{
    BlockReason, HoldState, MessageKind, RouteSpec, RunState, TaskNoteKind, TaskState, Verdict,
};

use super::*;
use crate::run::orch::test_support::*;

fn info<'a>(snap: &'a RunsSnapshot, run: &str, task: &str) -> &'a TaskInfo {
    snap.runs
        .iter()
        .find(|r| r.run_id == run)
        .and_then(|r| r.tasks.iter().find(|t| t.id == task))
        .unwrap()
}

fn state_of(runs: Vec<Run>) -> EngineState {
    let mut state = EngineState::default();
    for run in runs {
        state.runs.insert(run.id.clone(), run);
    }
    state
}

fn with_route(mut run: Run) -> Run {
    for task in run.tasks.iter_mut() {
        task.spec.route = RouteSpec {
            model: Some("claude-opus-5".into()),
            ..RouteSpec::default()
        };
    }
    run
}

#[test]
fn briefs_are_published_only_at_the_gate() {
    let mut running = with_route(run_of(2));
    running.id = "running-0001".into();
    running.state = RunState::Running;
    let mut gate = with_route(run_of(2));
    gate.id = "gate-0002".into();
    gate.state = RunState::AwaitingApproval;
    // A running run with an epic hold awaiting approval on `t1` only.
    let mut held = with_route(run_of(2));
    held.id = "held-0003".into();
    held.state = RunState::Running;
    held.orch.gate_holds = vec![hold("epic:e", HoldState::Awaiting, &["t1"], None)];
    task_mut(&mut held, "t1").orch.gate_hold = Some("epic:e".into());
    let snap = snapshot(&state_of(vec![running, gate, held]), 5_000);

    let empty = |t: &TaskInfo| {
        t.brief.is_empty() && t.acceptance.is_empty() && t.route_spec == RouteSpec::default()
    };
    let filled = |t: &TaskInfo, id: &str| {
        t.brief == format!("Brief {id}")
            && t.acceptance == [format!("Accept {id}")]
            && t.route_spec.model.as_deref() == Some("claude-opus-5")
    };
    for id in ["t0", "t1"] {
        assert!(empty(info(&snap, "running-0001", id)), "{id}");
        assert!(filled(info(&snap, "gate-0002", id), id), "{id}");
    }
    assert!(empty(info(&snap, "held-0003", "t0")));
    assert!(filled(info(&snap, "held-0003", "t1"), "t1"));
    assert_eq!(
        info(&snap, "held-0003", "t1").hold.as_deref(),
        Some("epic:e")
    );
    // Once the hold is approved, its tasks' text leaves the snapshot too.
    let mut approved = with_route(run_of(2));
    approved.state = RunState::Running;
    approved.orch.gate_holds = vec![hold("epic:e", HoldState::Approved, &["t1"], Some(1))];
    task_mut(&mut approved, "t1").orch.gate_hold = Some("epic:e".into());
    let id = approved.id.clone();
    let snap = snapshot(&state_of(vec![approved]), 5_000);
    assert!(empty(info(&snap, &id, "t1")));
}

/// 50 complete runs of 20 tasks with 4 KiB briefs. Decision 16a's own number, 256 KiB,
/// cannot hold (Implementation notes, M9.6): measured, the snapshot is 5 699 030
/// bytes before the change and 1 098 030 after, about 1.1 KiB a task of `TaskInfo`'s
/// other fields. So the test pins what 16a removes: the briefs, acceptance and route
/// specs add nothing to the encoded snapshot, and a task stays under 1.25 KiB.
#[test]
fn a_snapshot_of_fifty_terminal_runs_stays_small() {
    let runs = |brief: usize| -> Vec<Run> {
        (0..50)
            .map(|i| {
                let mut run = with_route(run_of(20));
                run.id = format!("done-run-{i:04}");
                run.state = RunState::Complete;
                for task in run.tasks.iter_mut() {
                    task.spec.brief = "b".repeat(brief);
                    task.spec.acceptance = vec!["a".repeat(brief / 8)];
                    task.state = TaskState::Merged;
                    task.merge_commit = Some("c".repeat(40));
                }
                run
            })
            .collect()
    };
    let bytes = |runs: Vec<Run>| {
        let snap = snapshot(&state_of(runs), 5_000);
        proto::encode(&snap).map_or(usize::MAX, |b| b.len())
    };
    let with_briefs = bytes(runs(4096));
    let without = bytes(runs(1));
    // Without decision 16a, the briefs alone would be 50 × 20 × 4 KiB = 4 MiB.
    assert_eq!(with_briefs, without, "the plan text reached the snapshot");
    assert!(with_briefs < SNAPSHOT_BOUND, "{with_briefs} bytes");
}

#[test]
fn task_notes_are_capped_at_ten_in_the_snapshot() {
    let mut run = run_of(1);
    let t0 = task_mut(&mut run, "t0");
    t0.orch.worker_notes = (0..15)
        .map(|i| note(1_000 + i, TaskNoteKind::Progress, &format!("note {i}")))
        .collect();
    t0.orch.worker_notes[14].text = "x".repeat(4_000);
    t0.orch.messages = vec![
        message(1_000, MessageKind::Info, "first", true),
        message(
            1_100,
            MessageKind::Change,
            "use v2\nthen rebuild\u{7}",
            false,
        ),
    ];
    let id = run.id.clone();
    let snap = snapshot(&state_of(vec![run]), 5_000);
    let t = info(&snap, &id, "t0");
    assert_eq!(t.task_notes.len(), 10);
    assert_eq!(t.task_notes[0].text, "note 5");
    assert_eq!(t.task_notes[0].task_id, "t0");
    assert_eq!(t.task_notes[0].at, 1_005);
    assert_eq!(t.task_notes[0].kind, TaskNoteKind::Progress);
    // Each text is bounded too: every push carries them.
    assert_eq!(
        t.task_notes[9].text.chars().count(),
        SNAPSHOT_NOTE_MAX + 1 // with `…`
    );
    assert_eq!(t.message_count, 2);
    assert_eq!(t.last_message_kind, Some(MessageKind::Change));
    assert_eq!(t.last_message_line.as_deref(), Some("use v2"));
}

#[test]
fn last_message_line_is_one_clean_line_of_at_most_80_characters() {
    let mut run = run_of(1);
    let long = format!("\u{1b}[31m{}\nsecond line", "y".repeat(100));
    task_mut(&mut run, "t0").orch.messages = vec![message(1, MessageKind::Info, &long, false)];
    let id = run.id.clone();
    let snap = snapshot(&state_of(vec![run]), 5_000);
    let line = info(&snap, &id, "t0").last_message_line.clone().unwrap();
    assert_eq!(line.chars().count(), 80);
    assert!(!line.chars().any(char::is_control), "{line:?}");
    assert!(!line.contains("second"));
}

/// The pre-existing fields are untouched by 16a: a blocked, reviewed task still shows
/// its block and review.
#[test]
fn the_rest_of_a_task_is_still_published() {
    let mut run = run_of(1);
    run.state = RunState::Running;
    let t0 = task_mut(&mut run, "t0");
    block(t0, BlockReason::Question, "which?");
    t0.reviews = vec![review(1, Some(Verdict::Approve), &[])];
    let id = run.id.clone();
    let snap = snapshot(&state_of(vec![run]), 5_000);
    let t = info(&snap, &id, "t0");
    assert_eq!(t.block.as_ref().unwrap().text, "which?");
    assert_eq!(t.reviews.len(), 1);
    assert_eq!(t.title, "Title t0");
}

#[test]
fn the_digest_revision_is_published() {
    let mut run = run_of(1);
    run.orch.digest_rev = 17;
    let id = run.id.clone();
    let snap = snapshot(&state_of(vec![run]), 5_000);
    let info = snap.runs.iter().find(|r| r.run_id == id).unwrap();
    assert_eq!(info.digest_revision, 17);
}
