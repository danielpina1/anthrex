//! The final fix wave's race items in the reducer: a task with its own checkout never
//! races (A-I2); a lane view's refresh and `resolving` (m1, m2); the crowned task's
//! reviewer author (m3); a lane's rung 4 on adoption (m4).

use proto::{AgentRole, BlockReason, LaneState, PlanEdit, RaceLane, Runtime, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::gates_review::reviewer;
use super::kinds::approve;
use super::race::{RACING, lane, launched, racing, tdd};
use super::race_crown::{crowned, handed_back};
use super::race_end::{b_out, to_check, unreviewed};
use super::race_lanes::{HEAD_B, proof, submit, to_review};
use super::turns::exited;
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::model::task_branch;
use crate::run::orch::RefreshState;

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// The note and log line of a task that already has its own checkout.
const HAS_CHECKOUT: &str = "race skipped: the task already has a checkout";

fn amend(id: &str, race: Option<bool>, deps: Option<Vec<String>>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: id.into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps,
        stage: None,
        race,
        pair: None,
    }
}

/// `t1` runs one ordinary worker in its own checkout `t1`, never a race, and the note
/// says why, once.
fn single_in_own_checkout(fx: &Fixture) {
    let t1 = fx.task("t1");
    assert!(t1.race.is_none(), "{:?}", t1.race);
    assert_eq!(t1.worktree, task_path("t1"));
    assert_eq!(t1.notes.iter().filter(|n| *n == HAS_CHECKOUT).count(), 1);
    let line = format!("task t1: {HAS_CHECKOUT}");
    assert_eq!(fx.run().log.iter().filter(|l| l.text == line).count(), 1);
    assert!(fx.ops("CrownRacer").is_empty());
    let last = t1.rounds.last().expect("a session");
    assert_eq!((last.role, last.lane), (AgentRole::Worker, None));
    // Accept and discard list `task.worktree`: the pre-warm or start checkout is not
    // leaked.
    let racers = (fx.ops("CreateWindow").into_iter())
        .filter(|(_, kind)| format!("{kind:?}").contains("t1.a"))
        .count();
    assert_eq!(racers, 0, "no lane checkout is launched");
}

/// A-I2, route 1: a task pre-warmed at the plan gate is amended to `race = true`; its
/// pre-warmed checkout is its own, so it runs single there.
#[test]
fn a_prewarmed_task_amended_to_race_runs_single_in_its_checkout() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "M", "a", "")]));
    fx.ready(false);
    fx.complete_prepares();
    assert!(fx.task("t1").prewarmed);
    let effects = edit(&mut fx, vec![amend("t1", Some(true), None)]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(fx.task("t1").spec.race);
    fx.approve();
    fx.complete_prepares();
    fx.complete_windows();
    assert_eq!(fx.task("t1").state, TaskState::Working);
    single_in_own_checkout(&fx);
}

/// A-I2, route 2: a racing task decided single starts in its own checkout, is blocked
/// on a dependency that is cancelled, and the amend of its deps releases it to
/// `pending` (which clears the latch). Decided again, it still runs single in its own
/// checkout: it has a start commit.
#[test]
fn a_started_task_released_to_pending_runs_single_again() {
    let tasks = [
        task("t0", "M", "z", "priority = 1"),
        task("t1", "S", "a", RACING),
    ];
    let mut fx = launched(PROFILE, &tasks, config::Orchestrator::default());
    let t1 = fx.task("t1");
    assert!(
        t1.race.is_none() && t1.start_commit.is_some(),
        "{:?}",
        t1.state
    );
    let window = (t1.rounds.iter().rfind(|r| r.role == AgentRole::Worker))
        .and_then(|r| r.window_id)
        .expect("t1's worker");
    let args = json!({"kind": "question", "reason": "which API?"});
    fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let add = PlanEdit::AddDep {
        task_id: "t1".into(),
        dep: "t0".into(),
    };
    let effects = edit(&mut fx, vec![add]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let cancel = PlanEdit::CancelTask {
        task_id: "t0".into(),
    };
    edit(&mut fx, vec![cancel]);
    assert_eq!(
        fx.task("t1").block.as_ref().map(|b| b.reason),
        Some(proto::BlockReason::DepCancelled)
    );
    let effects = edit(&mut fx, vec![amend("t1", None, Some(Vec::new()))]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    for (op, _) in fx.ops("PrepareWorktree") {
        if fx.run().pending_ops.contains_key(&op) {
            fx.done(op, OpResult::Worktree { head: BASE.into() });
        }
    }
    fx.complete_windows();
    let t1 = fx.task("t1");
    assert!(
        !matches!(t1.state, TaskState::Pending | TaskState::Queued),
        "{:?}",
        t1.state
    );
    single_in_own_checkout(&fx);
}

/// Each racer's turn ends, and any commit count the turn end started is answered.
fn turns_end(fx: &mut Fixture, windows: &[u32]) {
    for window in windows {
        let effects = fx.turn_completed(*window);
        for (op, _) in ops_in(&effects, "CountCommits") {
            let commits = OpResult::Commits {
                count: 0,
                head: BASE.into(),
            };
            fx.done(op, commits);
        }
    }
}

fn refresh_t1() -> PlanEdit {
    PlanEdit::Refresh {
        task_id: "t1".into(),
    }
}

/// The lane of each `HandBack` in `fx`'s log, in order.
fn hand_back_lanes(fx: &Fixture) -> Vec<String> {
    (fx.ops("HandBack").into_iter())
        .map(|(_, kind)| match kind {
            OpKind::HandBack { worktree, .. } => worktree.display().to_string(),
            other => panic!("{other:?}"),
        })
        .collect()
}

/// Minor m1: a refresh requested while the winner waits for its crown stays on the
/// task through the crown (`become_lane` keeps it), where the crowned task finds it.
#[test]
fn a_refresh_between_the_win_and_the_crown_survives_the_crown() {
    let (mut fx, _, b) = unreviewed();
    let check = to_check(&mut fx, B, b, HEAD_B);
    let effects = fx.done(check, check_result(true));
    let (crown, _) = only_op(&effects, "CrownRacer");
    assert_eq!(lane(&fx, B).state, LaneState::Won);
    let effects = edit(&mut fx, vec![refresh_t1()]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.task("t1").orch.refresh, Some(RefreshState::Due));
    crowned(&mut fx, crown, HEAD_B);
    assert_eq!(fx.task("t1").orch.refresh, Some(RefreshState::Due));
}

/// Minor m2: `resolving` is each lane's own. Lane a's refresh conflicts; lane b's due
/// refresh still goes into its checkout at its turn boundary.
#[test]
fn one_lanes_refresh_conflict_does_not_hold_the_other_lane() {
    let (mut fx, a, b) = racing();
    let effects = edit(&mut fx, vec![refresh_t1()]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    turns_end(&mut fx, &[a]);
    let (op, kind) = fx.ops("HandBack").pop().expect("lane a's hand-back");
    assert!(format!("{kind:?}").contains("t1.a"), "{kind:?}");
    let conflict = OpResult::HandedBack {
        files: vec!["crates/a/x.rs".to_string()],
        head: None,
        onto: None,
        merged: Vec::new(),
        merged_total: 0,
    };
    fx.done(op, conflict);
    assert!(lane(&fx, A).gates.resolving, "lane a resolves its conflict");
    assert!(!fx.task("t1").resolving, "the task's own flag is untouched");
    turns_end(&mut fx, &[b]);
    let lanes = hand_back_lanes(&fx);
    assert_eq!(lanes.len(), 2, "{lanes:?}");
    assert!(lanes[1].ends_with("t1.b"), "{lanes:?}");
}

/// Minor m3: after the crown the reviewer is picked against the winning racer's route.
/// Lane a (Claude) wins; lane b's (Codex) racer round is the later one, yet the
/// crowned task's next review runs on Codex, the peer of its author.
#[test]
fn after_the_crown_the_reviewers_author_is_the_winning_racer() {
    let (mut fx, a, _) = racing();
    let first = to_review(&mut fx, A, HEAD);
    let effects = submit(&mut fx, first, approve());
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD);
    exited(&mut fx, first);
    handed_back(&mut fx);
    let t1 = fx.task("t1");
    let racers: Vec<Option<RaceLane>> = (t1.rounds.iter())
        .filter(|r| r.role == AgentRole::Racer)
        .map(|r| r.lane)
        .collect();
    assert_eq!(racers.last(), Some(&Some(B)), "lane b's racer is the later");
    let effects = fx.tool_as(AgentRole::Racer, a, "t1", "task_done", tdd());
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut check = fx.clean_check("t1");
    if let OpResult::DoneChecked {
        head, head_branch, ..
    } = &mut check
    {
        *head = HEAD.to_string();
        *head_branch = Some(task_branch(RUN_ID, "t1"));
    }
    let effects = fx.done(op, check);
    let (op, _) = only_op(&effects, "Proof");
    fx.turn_completed(a);
    let effects = fx.done(op, proof(true));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(true));
    let (op, _) = only_op(&effects, "PrepareReview");
    let (_, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    match kind {
        OpKind::CreateWindow { spec, .. } => assert_eq!(spec.runtime, Runtime::Codex),
        other => panic!("{other:?}"),
    }
}

/// Minor m4: lane a, the last lane left, reaches rung 4 (its total spend at the next
/// size's budget) in its view. It is adopted, and after the crown the task is
/// `blocked(human)` at rung 4 (decision 38), not rung 3 with its size raised.
#[test]
fn a_lanes_rung_4_is_kept_on_adoption() {
    let (mut fx, _, _) = b_out();
    let later = fx.now + 1_000_000;
    let effects = fx.send(later, EventKind::Tick);
    let (op, _) = only_op(&effects, "CrownRacer");
    assert_eq!(lane(&fx, A).state, LaneState::Adopted);
    assert_eq!(lane(&fx, A).gates.rung, 4);
    crowned(&mut fx, op, BASE);
    let t1 = fx.task("t1");
    let block = t1.block.as_ref().expect("blocked");
    assert_eq!((t1.state, t1.rung), (TaskState::Blocked, 4));
    assert_eq!(block.reason, BlockReason::Human);
    assert!(block.text.contains("next size's budget"), "{}", block.text);
    assert_eq!(t1.size, proto::Size::M, "no size raised");
}
