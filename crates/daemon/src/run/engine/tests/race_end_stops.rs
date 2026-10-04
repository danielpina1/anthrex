//! Milestone 9.5 task M9.5.17b, fix rounds 1 and 2: how a stopped lane is salvaged
//! across a restart and a cancel. A lane's racer has exited only by its own
//! `ProcessExited` (or when its launch failed): a round the restore ended, a launch the
//! restart lost and a resume that failed keep the checkout and its locks (rulings
//! T17b-2, T17b-3). Salvage numbers are reserved as each removal is sent, from
//! the task's one counter (m4), and a lane records its ref only once the salvage
//! succeeded (m3). A cancel reaches every racing task whatever its state (m1), and the
//! review's missing cases (m6): a restart during a salvage, a cancel while the winner
//! awaits its crown, a run cancel, and a kept checkout on either lane.

use proto::{LaneState, PlanEdit, RaceLane, TaskState};

use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::gates::only_op;
use super::race::{lane, lane_path, racing};
use super::race_crown::crowned;
use super::race_end::{kills, removal, salvage};
use super::race_lanes::{HEAD_B, passes};
use super::turns::killed_exit;
use crate::run::engine::race_end::LANE_EXIT_WAIT_SECS;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::OpId;

const A: RaceLane = RaceLane::A;
const B: RaceLane = RaceLane::B;

/// The removals of lane `l`'s own checkout among `effects`.
fn lane_removals(effects: &[Effect], l: RaceLane) -> Vec<(OpId, OpKind)> {
    let path = lane_path(l);
    (ops_in(effects, "RemoveWorktree").into_iter())
        .filter(|(_, kind)| matches!(kind, OpKind::RemoveWorktree { path: p, .. } if *p == path))
        .collect()
}

/// The only removal of lane `l`'s checkout among `effects`.
fn only_removal(effects: &[Effect], l: RaceLane) -> (OpId, OpKind) {
    let mut removals = lane_removals(effects, l);
    assert_eq!(removals.len(), 1, "{effects:#?}");
    removals.remove(0)
}

fn removed(n: u32) -> OpResult {
    OpResult::Removed {
        salvage_ref: Some(salvage(n)),
        cleared_locks: Vec::new(),
    }
}

fn logged(fx: &Fixture, line: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == line)
}

fn cancel(fx: &mut Fixture) -> Vec<Effect> {
    let cancel = PlanEdit::CancelTask {
        task_id: "t1".into(),
    };
    edit(fx, vec![cancel])
}

/// A new daemon, then one scheduler pass.
fn restarted(fx: &mut Fixture) -> Vec<Effect> {
    let mut effects = restart(fx, Vec::new());
    effects.extend(fx.tick());
    effects
}

/// Ruling T17b-2 (review I1): a loser whose racer had not sent `ProcessExited` when
/// the daemon restarted has not exited: its checkout is salvaged and kept, no lock is
/// cleared, and the run log says why.
#[test]
fn a_restart_before_the_losers_exit_keeps_its_checkout_and_its_locks() {
    let (mut fx, _, _) = racing();
    passes(&mut fx, B, HEAD_B);
    assert_eq!(lane(&fx, A).state, LaneState::Lost);
    let effects = restarted(&mut fx);
    let (op, kind) = only_removal(&effects, A);
    assert_eq!(kind, removal(A, 1, true));
    let line = "race t1: kept a checkout: its racer had no process after the restart";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
    let la = lane(&fx, A);
    assert!(la.kept);
    fx.done(op, removed(1));
    let la = lane(&fx, A);
    assert_eq!((la.salvage_ref, la.removed), (Some(salvage(1)), false));
    assert!(lane_removals(&fx.tick(), A).is_empty());
}

/// Review m6 and m3: a salvage the restart lost is sent again with the same number,
/// and the lane records its ref only once it succeeded.
#[test]
fn a_restart_during_a_losers_salvage_sends_it_again_with_its_number() {
    let (mut fx, a, _) = racing();
    passes(&mut fx, B, HEAD_B);
    let effects = killed_exit(&mut fx, a);
    assert_eq!(only_removal(&effects, A).1, removal(A, 1, false));
    assert_eq!(lane(&fx, A).salvage_ref, None, "not before it succeeded");
    let effects = restarted(&mut fx);
    let (op, kind) = only_removal(&effects, A);
    assert_eq!(kind, removal(A, 1, false));
    fx.done(op, removed(1));
    let la = lane(&fx, A);
    assert!(la.removed && !la.kept);
    assert_eq!(la.salvage_ref, Some(salvage(1)));
    assert_eq!(fx.task("t1").salvage_refs, [salvage(1)]);
    assert!(lane_removals(&fx.tick(), A).is_empty());
}

/// Review m3: a salvage that failed records no ref; the checkout is kept.
#[test]
fn a_failed_salvage_records_no_ref() {
    let (mut fx, a, _) = racing();
    passes(&mut fx, B, HEAD_B);
    let (op, _) = only_removal(&killed_exit(&mut fx, a), A);
    let failed = OpResult::Failed {
        message: "disk full".into(),
    };
    fx.done(op, failed);
    let la = lane(&fx, A);
    assert_eq!((la.salvage_ref, la.kept), (None, true));
    assert!(fx.task("t1").salvage_refs.is_empty());
}

/// Review m4: the winner's merge sends its three removals; the loser's salvage, sent
/// while they are in flight, takes the next number from the same counter.
#[test]
fn a_losers_salvage_after_the_winners_merge_takes_its_own_number() {
    let (mut fx, a, _) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (op, _) = only_op(&effects, "CrownRacer");
    crowned(&mut fx, op, HEAD_B);
    let (op, _) = super::merge::candidate(&fx, "t1");
    let merged = OpResult::Merged {
        commit: "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1".into(),
        tier: None,
    };
    let effects = fx.done(op, merged);
    let refs: Vec<String> = (ops_in(&effects, "RemoveWorktree").into_iter())
        .map(|(_, kind)| match kind {
            OpKind::RemoveWorktree { salvage_ref, .. } => salvage_ref,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(refs, [salvage(1), salvage(2), salvage(3)]);
    let effects = killed_exit(&mut fx, a);
    assert_eq!(only_removal(&effects, A).1, removal(A, 4, false));
}

/// Review m6: both racers of a cancelled race ignore their stop; each lane's checkout is
/// kept with its own label in the log line.
#[test]
fn a_cancelled_race_whose_racers_do_not_exit_keeps_both_checkouts() {
    let (mut fx, _, _) = racing();
    cancel(&mut fx);
    let sent = lane(&fx, B).kill_sent_at.expect("lane b was stopped");
    let effects = fx.send(sent + LANE_EXIT_WAIT_SECS, EventKind::Tick);
    assert_eq!(only_removal(&effects, A).1, removal(A, 1, true));
    assert_eq!(only_removal(&effects, B).1, removal(B, 2, true));
    for label in ["a", "b"] {
        let line = format!("race t1: kept {label} checkout: its racer did not exit");
        assert!(logged(&fx, &line), "{line}: {:#?}", fx.run().log);
    }
    assert!(lane(&fx, A).kept && lane(&fx, B).kept);
}

/// Review m6: a cancel while the winner awaits its crown stops the winner too; its
/// checkout is salvaged once the crown has returned and its racer has exited.
#[test]
fn a_cancel_while_the_winner_awaits_its_crown_salvages_it_after_the_crown() {
    let (mut fx, a, b) = racing();
    let effects = passes(&mut fx, B, HEAD_B);
    let (crown, _) = only_op(&effects, "CrownRacer");
    let effects = cancel(&mut fx);
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert!(kills(&effects).contains(&b), "{effects:#?}");
    assert_eq!(lane(&fx, B).state, LaneState::Out);
    assert_eq!(
        only_removal(&killed_exit(&mut fx, a), A).1,
        removal(A, 1, false)
    );
    let effects = killed_exit(&mut fx, b);
    assert!(
        lane_removals(&effects, B).is_empty(),
        "its crown is in flight"
    );
    let effects = crowned(&mut fx, crown, HEAD_B);
    assert_eq!(only_removal(&effects, B).1, removal(B, 2, false));
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
}

/// Review m6: `run cancel` (the run's end) stops and salvages both lanes.
#[test]
fn a_run_cancel_stops_and_salvages_both_lanes() {
    let (mut fx, a, b) = racing();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    let killed = kills(&effects);
    assert!(killed.contains(&a) && killed.contains(&b), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert_eq!(
        (lane(&fx, A).state, lane(&fx, B).state),
        (LaneState::Out, LaneState::Out)
    );
    assert_eq!(
        only_removal(&killed_exit(&mut fx, a), A).1,
        removal(A, 1, false)
    );
    assert_eq!(
        only_removal(&killed_exit(&mut fx, b), B).1,
        removal(B, 2, false)
    );
}

/// Review m1: a race blocked after a failed crown is not live, yet a cancel stops its
/// winner and salvages its checkout by the same rules; after a restart, the winner had
/// no process, so its checkout is kept.
#[test]
fn a_cancel_reaches_a_race_blocked_after_a_failed_crown() {
    for after_restart in [false, true] {
        let (mut fx, a, b) = racing();
        let effects = passes(&mut fx, B, HEAD_B);
        let (op, _) = only_op(&effects, "CrownRacer");
        let failed = OpResult::Failed {
            message: "disk full".into(),
        };
        fx.done(op, failed);
        assert_eq!(fx.task("t1").state, TaskState::Blocked);
        let (op, _) = only_removal(&killed_exit(&mut fx, a), A);
        fx.done(op, removed(1));
        if after_restart {
            restarted(&mut fx);
        }
        let effects = cancel(&mut fx);
        assert_eq!(fx.task("t1").state, TaskState::Cancelled);
        assert_eq!(lane(&fx, B).state, LaneState::Out);
        let effects = match after_restart {
            false => {
                assert!(kills(&effects).contains(&b), "{effects:#?}");
                killed_exit(&mut fx, b)
            }
            true => [effects, fx.tick()].concat(),
        };
        assert_eq!(
            only_removal(&effects, B).1,
            removal(B, 2, after_restart),
            "after_restart: {after_restart}"
        );
    }
}

/// A racing `t1` whose lane b's racer window came and whose lane a's `CreateWindow` is
/// still in flight: the fixture and that op. Lane b's racer is window 90.
fn lane_a_launching() -> (Fixture, OpId) {
    // Unreviewed (S), so lane b wins with no reviewer window to launch.
    let config = config::Orchestrator {
        review_small: false,
        ..Default::default()
    };
    let tasks = [task("t1", "S", "a", super::race::RACING)];
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &tasks), config);
    fx.ready(true);
    fx.complete_prepares();
    let launches: Vec<OpId> = fx
        .ops("CreateWindow")
        .into_iter()
        .map(|(op, _)| op)
        .collect();
    let lane_of = |fx: &Fixture, op| super::race::lane_of(fx, op);
    let a = *launches
        .iter()
        .find(|op| lane_of(&fx, **op) == Some(A))
        .expect("a's launch");
    let b = *launches
        .iter()
        .find(|op| lane_of(&fx, **op) == Some(B))
        .expect("b's launch");
    let window = OpResult::Window {
        window_id: 90,
        pid: None,
    };
    fx.done(b, window);
    (fx, a)
}

/// Lane b (window 90) claims, passes its proof and its check, and wins.
fn b_wins_unreviewed(fx: &mut Fixture) {
    let effects = super::race::claim(fx, B, 90, HEAD_B);
    let (op, _) = only_op(&effects, "Proof");
    let effects = fx.done(op, super::race_lanes::proof(true));
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, super::gates::check_result(true));
    assert_eq!(lane(fx, B).state, LaneState::Won);
    assert_eq!(lane(fx, A).state, LaneState::Lost);
}

/// Ruling T17b-3 (N1): a loser whose racer's `CreateWindow` the restart lost may have
/// been started by the old daemon, so its checkout is kept and its locks too; only a
/// launch whose failure came back counts as exited.
#[test]
fn a_losers_launch_lost_at_a_restart_keeps_its_checkout() {
    for lost in [true, false] {
        let (mut fx, launch) = lane_a_launching();
        b_wins_unreviewed(&mut fx);
        let effects = match lost {
            true => restarted(&mut fx),
            false => {
                let failed = OpResult::Failed {
                    message: "no such binary".into(),
                };
                [fx.done(launch, failed), fx.tick()].concat()
            }
        };
        assert_eq!(
            only_removal(&effects, A).1,
            removal(A, 1, lost),
            "lost: {lost}"
        );
        let line = "race t1: kept a checkout: its racer had no process after the restart";
        assert_eq!(logged(&fx, line), lost, "{:#?}", fx.run().log);
    }
}

/// Ruling T17b-4: a live lane whose launch the restart lost is relaunched, but the old
/// daemon may have started a racer whose pid was never recorded (decision 28 cannot
/// stop it): the relaunched racer's lane stays orphaned, and once stopped its checkout
/// is kept and its locks are left.
#[test]
fn a_relaunched_racers_lane_is_still_kept() {
    let (mut fx, _) = lane_a_launching();
    restart(&mut fx, Vec::new());
    let mut effects = super::control::resume(&mut fx);
    effects.extend(fx.tick());
    let (op, _) = only_op(&effects, "CreateWindow");
    let window = OpResult::Window {
        window_id: 91,
        pid: None,
    };
    fx.done(op, window);
    let racer = (fx.task("t1").rounds.iter()).rfind(|r| r.lane == Some(A));
    assert!(
        racer.expect("a's racer").orphaned,
        "a relaunch clears nothing"
    );
    let effects = cancel(&mut fx);
    assert!(
        lane_removals(&effects, A).is_empty(),
        "it waits for its exit"
    );
    let (_, kind) = only_removal(&killed_exit(&mut fx, 91), A);
    assert!(
        matches!(
            kind,
            OpKind::RemoveWorktree {
                keep_path: true,
                clear_locks: false,
                keep_head: true,
                ..
            }
        ),
        "{kind:?}"
    );
    let line = "race t1: kept a checkout: its racer had no process after the restart";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
}

/// Ruling T17b-3 (N2): a restored racer is its own again only once its resume
/// succeeded. Lane b's resume fails and a fresh session takes the lane: cancelled, its
/// checkout is kept once that session exits (the old daemon's racer may still run).
/// Lane b's resume succeeds: its checkout is removed once its racer exits.
#[test]
fn a_racer_whose_resume_failed_keeps_its_checkout() {
    for resumed in [false, true] {
        let (mut fx, _, b) = racing();
        super::race_end_requests::with_codex_session(&mut fx);
        restart(&mut fx, Vec::new());
        let mut effects = super::control::resume(&mut fx);
        effects.extend(fx.tick());
        let resume = (ops_in(&effects, "ResumeSession").into_iter())
            .find(|(_, kind)| matches!(kind, OpKind::ResumeSession { window_id, .. } if *window_id == b))
            .expect("lane b's resume")
            .0;
        let result = match resumed {
            true => OpResult::Resumed,
            false => OpResult::ResumeFailed {
                error: "no such thread".into(),
            },
        };
        fx.done(resume, result);
        // A failed resume starts a fresh session in the lane, after its diff so far.
        let diffs = (fx.run().pending_ops.values())
            .filter(|p| p.kind.name() == "DiffSoFar" && p.lane == Some(B))
            .map(|p| p.op);
        for op in diffs.collect::<Vec<_>>() {
            let diff = OpResult::Diff {
                stat: String::new(),
                patch: String::new(),
            };
            fx.done(op, diff);
        }
        fx.complete_windows();
        let racer = super::race::window(&fx, B);
        assert_eq!(racer == b, resumed);
        let effects = cancel(&mut fx);
        assert!(
            lane_removals(&effects, B).is_empty(),
            "it waits for its exit"
        );
        let effects = killed_exit(&mut fx, racer);
        assert_eq!(
            only_removal(&effects, B).1,
            removal(B, 1, !resumed),
            "resumed: {resumed}"
        );
        let line = "race t1: kept b checkout: its racer had no process after the restart";
        assert_eq!(logged(&fx, line), !resumed, "{:#?}", fx.run().log);
    }
}
