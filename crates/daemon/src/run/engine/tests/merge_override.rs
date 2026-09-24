//! M8a.25: a task blocked at rung 3 (its worker session stopped by the engine), then
//! overridden, whose merge candidate conflicts: the hand-back's conflict message is
//! carried by a resume of that stopped session (decision 36 step 6: "decision 29
//! resumes an ended session to carry it"), whether the kill's exit reached the engine
//! before the hand-back or after it. Found by `e2e_conflict_is_handed_back_and_resolved`:
//! the message sat in the outbox and the task stayed `working` with no session.

use proto::TaskState;

use super::control::{blocked, override_task};
use super::fixture::*;
use super::gates::only_op;
use super::liveness::assert_alive;
use super::turns::{killed_exit, working};
use crate::run::contract::conflict_message;
use crate::run::engine::{Effect, OpKind, OpResult};

/// `t1` blocked by `task_blocked mis_sized` (rung 3), overridden, its candidate
/// conflicting, and handed back; the kill's exit arrives first when `exit_first`.
/// Every effect from the hand-back's result on.
fn handed_back_after_rung_3(exit_first: bool) -> (Fixture, Vec<Effect>) {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "mis_sized", "too big");
    assert_eq!(fx.task("t1").rung, 3);
    if exit_first {
        killed_exit(&mut fx, window);
    }
    let effects = override_task(&mut fx, "t1", "shared-ok");
    let (op, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 1,
            head: HEAD.into(),
        },
    );
    let (op, _) = only_op(&effects, "MergeCandidate");
    let effects = fx.done(
        op,
        OpResult::Conflict {
            files: vec!["b/shared.txt".into()],
        },
    );
    let (op, _) = only_op(&effects, "HandBack");
    let mut effects = fx.done(
        op,
        OpResult::HandedBack {
            files: vec!["b/shared.txt".into()],
            head: None,
            onto: Some(HEAD.into()),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    if !exit_first {
        effects.extend(killed_exit(&mut fx, window));
    }
    effects.extend(fx.tick());
    (fx, effects)
}

fn assert_resumed_with_the_conflict(fx: &Fixture, effects: &[Effect]) {
    let session = fx.task("t1").rounds[0]
        .session_id
        .clone()
        .expect("session 1's id");
    let resumes = ops_in(effects, "ResumeSession");
    assert_eq!(resumes.len(), 1, "{effects:#?}");
    let OpKind::ResumeSession {
        session_id,
        message,
        ..
    } = &resumes[0].1
    else {
        unreachable!()
    };
    assert_eq!(*session_id, session);
    assert_eq!(*message, conflict_message(&["b/shared.txt".to_string()]));
    let round = &fx.task("t1").rounds[0];
    assert!(!round.ended && !round.retiring, "{round:#?}");
    assert_alive(fx);
}

#[test]
fn a_hand_back_resumes_the_session_rung_3_stopped() {
    let (fx, effects) = handed_back_after_rung_3(true);
    assert_resumed_with_the_conflict(&fx, &effects);
}

#[test]
fn a_hand_back_before_the_kills_exit_resumes_the_session_once_it_ends() {
    let (fx, effects) = handed_back_after_rung_3(false);
    assert_resumed_with_the_conflict(&fx, &effects);
}
