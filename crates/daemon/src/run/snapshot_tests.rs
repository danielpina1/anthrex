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
    // `rounds`: `None` for no rounds (the bound's own runs), else one ended worker round
    // a task, with (`Some(true)`) or without its activity and last message.
    let runs = |brief: usize, rounds: Option<bool>| -> Vec<Run> {
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
                    let Some(text) = rounds else { continue };
                    let mut ended = round(1, 3, Default::default());
                    ended.ended = true;
                    ended.ended_at = Some(1_400);
                    if text {
                        ended.activity = Some("a".repeat(proto::ACTIVITY_MAX));
                        ended.last_text = Some("t".repeat(proto::WORKER_SUMMARY_MAX));
                    }
                    task.rounds = vec![ended];
                }
                run
            })
            .collect()
    };
    let bytes = |runs: Vec<Run>| {
        let snap = snapshot(&state_of(runs), 5_000);
        proto::encode(&snap).map_or(usize::MAX, |b| b.len())
    };
    let with_briefs = bytes(runs(4096, None));
    let without = bytes(runs(1, None));
    // Without decision 16a, the briefs alone would be 50 × 20 × 4 KiB = 4 MiB.
    assert_eq!(with_briefs, without, "the plan text reached the snapshot");
    assert!(with_briefs < SNAPSHOT_BOUND, "{with_briefs} bytes");
    // Milestone 9.0.5 decision 2: an ended round's activity and last message add
    // nothing either.
    assert_eq!(
        bytes(runs(1, Some(true))),
        bytes(runs(1, Some(false))),
        "an ended round's text reached the snapshot"
    );
}

/// Milestone 9.0.5 decision 2: `TaskInfo.activity` is the live round's, and `None`
/// once every round has ended.
#[test]
fn activity_is_published_only_for_a_live_round() {
    let mut run = run_of(1);
    run.state = RunState::Running;
    let t0 = task_mut(&mut run, "t0");
    let mut old = round(1, 2, Default::default());
    old.ended = true;
    old.ended_at = Some(1_200);
    old.activity = Some("Edit old.rs".into());
    let mut live = round(2, 1, Default::default());
    live.activity = Some("Bash cargo test".into());
    t0.rounds = vec![old, live];
    let id = run.id.clone();
    let snap = snapshot(&state_of(vec![run.clone()]), 5_000);
    assert_eq!(
        info(&snap, &id, "t0").activity.as_deref(),
        Some("Bash cargo test")
    );
    let live = task_mut(&mut run, "t0").rounds.last_mut().unwrap();
    live.ended = true;
    live.ended_at = Some(1_600);
    let snap = snapshot(&state_of(vec![run]), 5_000);
    assert_eq!(info(&snap, &id, "t0").activity, None);
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

#[test]
fn wake_held_reaches_the_snapshot() {
    let mut held = run_of(1);
    held.id = "held-0001".into();
    held.state = RunState::Running;
    held.orch.orchestrator = Some(orchestrator());
    held.orch.wake_held = true;
    let mut free = run_of(1);
    free.id = "free-0001".into();
    free.state = RunState::Running;
    free.orch.orchestrator = Some(orchestrator());
    let snap = snapshot(&state_of(vec![held, free]), 5_000);
    let wake_held = |id: &str| {
        let run = snap.runs.iter().find(|r| r.run_id == id).unwrap();
        run.orchestrator.as_ref().unwrap().wake_held
    };
    assert!(wake_held("held-0001"));
    assert!(!wake_held("free-0001"));
}

/// Milestone 9.0.6 decision 7: a running run's snapshot carries the run's, each
/// stage's and each task's actions, as `actions::available` lists them.
#[test]
fn the_snapshot_carries_each_nodes_actions() {
    use crate::run::engine::actions::{ActionNode, available};
    let mut run = run_of(2);
    run.state = RunState::Running;
    run.stage_layout = crate::run::model::StageLayout::Multi;
    task_mut(&mut run, "t1").spec.stage = 2;
    let snap = snapshot(&state_of(vec![run.clone()]), 5_000);
    let info = &snap.runs[0];
    assert_eq!(info.actions, available(&run, &ActionNode::Run));
    assert!(!info.actions.is_empty());
    assert_eq!(info.stages.len(), 2);
    for stage in &info.stages {
        assert_eq!(stage.actions, available(&run, &ActionNode::Stage(stage.n)));
        assert!(!stage.actions.is_empty(), "stage {}", stage.n);
    }
    for task in &info.tasks {
        assert_eq!(task.actions, available(&run, &ActionNode::Task(&task.id)));
        assert!(!task.actions.is_empty(), "{}", task.id);
    }
}

/// Milestone 9.5 decision 43 (FU-F23): a round waiting out a failed turn carries its
/// error; a rate-limited one too, and its wait stays `rate_limited_until`'s.
///
/// `failed_until` is not asserted: decision 43's `at + rate_limit_retry_secs` reads
/// `WaitingContinue.at` as the failure's time, but the engine stores the continue's
/// own time there (`not_before(now, wait)`). Stopped for a ruling (Implementation
/// notes, Task M9.5.6).
#[test]
fn a_waiting_failed_turn_is_in_the_snapshot() {
    use crate::run::model::FailedTurn;
    let failed = |rate_limit: bool| {
        let mut run = run_of(1);
        run.id = "failed-0001".into();
        run.state = RunState::Running;
        run.limits.rate_limit_retry_secs = 300;
        let mut r = round(1, 3, TokenUsage::default());
        r.failed_turn = FailedTurn::WaitingContinue {
            at: 100,
            rate_limit,
        };
        r.failed_error = Some("overloaded".into());
        task_mut(&mut run, "t0").rounds = vec![r];
        let snap = snapshot(&state_of(vec![run]), 50);
        info(&snap, "failed-0001", "t0").rounds[0].clone()
    };
    let other = failed(false);
    assert_eq!(other.failed_error.as_deref(), Some("overloaded"));
    let limited = failed(true);
    assert_eq!(limited.failed_error.as_deref(), Some("overloaded"));
    assert_eq!(limited.failed_until, None, "a rate limit has its own wait");
    // A round with no failed turn carries neither.
    let mut run = run_of(1);
    run.state = RunState::Running;
    let id = run.id.clone();
    task_mut(&mut run, "t0").rounds = vec![round(1, 3, TokenUsage::default())];
    let snap = snapshot(&state_of(vec![run]), 50);
    let plain = &info(&snap, &id, "t0").rounds[0];
    assert_eq!((&plain.failed_error, plain.failed_until), (&None, None));
}
