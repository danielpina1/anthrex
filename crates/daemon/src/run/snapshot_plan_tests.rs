//! Milestone 9.17 task M9.17.4 (decision 10): what the user sees of a `brief` note in
//! the run's attention lines. Pure.

use proto::TaskNoteKind;

use crate::run::engine::EngineState;
use crate::run::model::Run;
use crate::run::orch::add_worker_note;
use crate::run::orch::test_support::{note, orchestrator, run_of};
use crate::run::snapshot::snapshot;

const STICKY: &str =
    "t1: the reviewer says the brief is wrong (1 notes); read them in the task's notes";

/// Run `run`'s attention lines as the snapshot carries them.
fn attention(run: &Run, now: u64) -> Vec<String> {
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    snapshot(&state, now).runs.remove(0).attention
}

/// `run_of(2)` with one `brief` note on `t1` and then four later `risk` notes.
fn noted_run() -> Run {
    let mut run = run_of(2);
    let text = "review round 1 (critical): the brief asks for the wrong table";
    assert!(add_worker_note(
        &mut run,
        "t1",
        note(100, TaskNoteKind::Brief, text)
    ));
    for n in 0..4 {
        let risk = note(200 + n, TaskNoteKind::Risk, &format!("risk {n}"));
        assert!(add_worker_note(&mut run, "t0", risk));
    }
    run
}

#[test]
fn plan_file_run_has_a_sticky_brief_attention_line() {
    let run = noted_run();
    assert!(run.orch.orchestrator.is_none());
    let lines = attention(&run, 1_000);
    assert!(lines.contains(&STICKY.to_string()), "{lines:?}");
    assert!(
        !lines.iter().any(|l| l.contains("review flagged the brief")),
        "pushed out of the last three notes: {lines:?}"
    );
    // The count follows the notes.
    let mut run = run;
    let more = note(300, TaskNoteKind::Brief, "review round 2 (minor): again");
    assert!(add_worker_note(&mut run, "t1", more));
    let two = STICKY.replace("(1 notes)", "(2 notes)");
    assert!(attention(&run, 1_000).contains(&two));
    // A finished run has nobody left to act.
    run.state = proto::RunState::Accepted;
    assert!(
        !attention(&run, 1_000)
            .iter()
            .any(|l| l.contains("brief is wrong"))
    );
}

#[test]
fn orchestrator_run_has_no_sticky_line() {
    let mut run = noted_run();
    run.orch.orchestrator = Some(orchestrator());
    let lines = attention(&run, 1_000);
    assert!(
        !lines.iter().any(|l| l.contains("brief is wrong")),
        "{lines:?}"
    );
}

/// Decision 7: a round whose only serious findings are `brief` did not block, and the
/// snapshot's `ReviewInfo.blocking` says so.
#[test]
fn brief_only_review_is_not_blocking() {
    use crate::run::orch::test_support::review;
    use proto::{Severity, Verdict};
    let mut run = run_of(2);
    let mut brief_only = review(1, Some(Verdict::Changes), &[(Severity::Critical, "b")]);
    brief_only.findings[0].target = proto::FindingTarget::Brief;
    let code = review(2, Some(Verdict::Changes), &[(Severity::Important, "c")]);
    run.tasks[1].reviews = vec![brief_only, code];
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run);
    let snap = snapshot(&state, 1_000);
    let blocking: Vec<bool> = (snap.runs[0].tasks.iter())
        .find(|t| t.id == "t1")
        .map(|t| t.reviews.iter().map(|r| r.blocking).collect())
        .unwrap();
    assert_eq!(blocking, [false, true]);
}

#[test]
fn noted_line_words_a_brief_note() {
    let mut run = run_of(2);
    let long = format!("review round 1 (important): {}", "y".repeat(100));
    assert!(add_worker_note(
        &mut run,
        "t1",
        note(100, TaskNoteKind::Brief, &long)
    ));
    assert!(add_worker_note(
        &mut run,
        "t0",
        note(90, TaskNoteKind::Risk, "r")
    ));
    let first: String = long.chars().take(80).collect();
    let lines = attention(&run, 1_000);
    assert!(
        lines.contains(&format!("t1 review flagged the brief: {first}")),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"t0 noted a risk: r".to_string()),
        "{lines:?}"
    );
}

/// Task M9.17.6: run `run`'s snapshot entry.
fn info_of(run: &Run) -> proto::RunInfo {
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    snapshot(&state, 1_000).runs.remove(0)
}

/// `run_of(2)` with a round-1 frame and epic `e1` (owning `t1`) with its own.
fn framed_run() -> Run {
    use crate::run::orch::test_support::{plan_frame, task_mut};
    use crate::run::orch::{EpicRecord, PlannerPhase};
    let mut run = run_of(2);
    run.orch.plan_frames = vec![plan_frame(1, &[("Which DB?", "SQLite")], &["no UI"])];
    let mut epic = EpicRecord::new("e1", PlannerPhase::Finished);
    epic.frame = Some(plan_frame(1, &[], &["no docs"]));
    run.orch.epics.push(epic);
    task_mut(&mut run, "t1").spec.epic = Some("e1".into());
    run
}

/// Decision 20 (and ruling C7): the run's frame and an epic's are published only while
/// the run awaits approval, or a hold awaits: an epic's hold publishes the epic's frame
/// on its `PlannerInfo`, the `promotion` hold the run's.
#[test]
fn frame_is_published_only_at_the_gate_or_an_awaiting_hold() {
    use crate::run::orch::test_support::hold;
    use proto::{HoldState, RunState};
    let mut run = framed_run();
    let epic_frame = |info: &proto::RunInfo| info.planners[0].frame.clone();
    run.state = RunState::Planning;
    let info = info_of(&run);
    assert_eq!((info.plan_frame, epic_frame(&info_of(&run))), (None, None));
    run.state = RunState::AwaitingApproval;
    let info = info_of(&run);
    let frame = info
        .plan_frame
        .clone()
        .expect("the gate shows the round's frame");
    assert_eq!(frame.decided[0].answer, "SQLite");
    assert_eq!(frame.out_of_scope, ["no UI"]);
    assert_eq!(epic_frame(&info).unwrap().out_of_scope, ["no docs"]);
    run.state = RunState::Running;
    let info = info_of(&run);
    assert_eq!((info.plan_frame.clone(), epic_frame(&info)), (None, None));
    run.orch.gate_holds = vec![hold("epic:e1", HoldState::Awaiting, &["t1"], None)];
    let info = info_of(&run);
    assert_eq!(
        info.plan_frame, None,
        "an epic's hold shows its epic's frame only"
    );
    assert_eq!(epic_frame(&info).unwrap().out_of_scope, ["no docs"]);
    run.orch.gate_holds = vec![hold("epic:e1", HoldState::Approved, &["t1"], Some(2))];
    let info = info_of(&run);
    assert_eq!((info.plan_frame.clone(), epic_frame(&info)), (None, None));
}

/// Ruling C7: a promoted run's `promotion` hold is its plan approval, so the round's
/// frame is published while it awaits.
#[test]
fn promotion_hold_review_shows_the_run_frame() {
    use crate::run::orch::test_support::hold;
    use proto::{HoldState, RunState};
    let mut run = framed_run();
    run.state = RunState::Running;
    run.orch.gate_holds = vec![hold("promotion", HoldState::Awaiting, &["t0", "t1"], None)];
    let info = info_of(&run);
    assert_eq!(info.plan_frame.unwrap().out_of_scope, ["no UI"]);
    run.orch.gate_holds[0].state = HoldState::Approved;
    assert_eq!(info_of(&run).plan_frame, None);
}

/// Decision 20: a design run's gate carries the approved spec's Non-goals, cut to
/// 2 KiB, with or without a frame.
#[test]
fn design_run_publishes_spec_non_goals() {
    use crate::run::design::state::DesignState;
    let mut run = run_of(1);
    run.state = proto::RunState::AwaitingApproval;
    let text = format!("- no mobile app\n{}", "- more\n".repeat(600));
    run.orch.design = Some(DesignState {
        non_goals_section: text.clone(),
        ..DesignState::default()
    });
    let frame = info_of(&run)
        .plan_frame
        .expect("the spec's Non-goals are a frame");
    let goals = frame.spec_non_goals.unwrap();
    assert!(goals.starts_with("- no mobile app\n"), "{goals}");
    assert!(goals.len() <= 2048 && text.starts_with(&goals));
    assert!(frame.decided.is_empty() && frame.out_of_scope.is_empty());
    // Running, it is not published.
    run.state = proto::RunState::Running;
    assert_eq!(info_of(&run).plan_frame, None);
}

/// Decision 26's read side: `replaces` as stored, and `replaced_by` computed from every
/// task that names this one, in plan order.
#[test]
fn task_info_shows_replaces_and_replaced_by() {
    let mut run = run_of(4);
    run.tasks[0].state = proto::TaskState::Cancelled;
    run.tasks[2].replaces = vec!["t0".into()];
    run.tasks[3].replaces = vec!["t0".into(), "t1".into()];
    let info = info_of(&run);
    let of = |id: &str| info.tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(of("t0").replaced_by, ["t2", "t3"]);
    assert_eq!(of("t1").replaced_by, ["t3"]);
    assert_eq!(of("t3").replaces, ["t0", "t1"]);
    assert!(of("t2").replaced_by.is_empty() && of("t1").replaces.is_empty());
}
