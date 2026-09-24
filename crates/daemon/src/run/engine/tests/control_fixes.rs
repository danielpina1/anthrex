//! M8a.15 fix round 1: the review's probes as regression tests, under rulings T15-C1
//! (a retry starts a new budget epoch), T15-I1 (a restore re-engages every working
//! task), T15-I2 and T15-I3 (session minutes and stall silence count only while the
//! task works in a running run) and T15-minors. Every sequence ends with the liveness
//! check.

use proto::{PlanEdit, RunState, TaskState};

use super::control::{blocked, override_task, resume, retry};
use super::control_restore::{restart, resumes};
use super::dispatch::{edit, replies};
use super::done::one_reply;
use super::fixture::*;
use super::gates::{check_result, only_op};
use super::holds::{add_dep, answer, blocked_t1, delivers};
use super::liveness::assert_alive;
use super::merge::{
    candidate, claim, doc_task, head_of, pending_one, start_on, to_queue, window_of,
};
use super::turns::{exited, killed_exit, working};
use crate::headless::FailureKind;
use crate::run::contract::RESUME_WORKER;
use crate::run::engine::ladder::total_spend;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpResult, TurnOutcome};

/// The fresh session a retry started: its `DiffSoFar` answered and its window up.
fn fresh_session_up(fx: &mut Fixture, window: u32) {
    let effects = killed_exit(fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    only_op(&effects, "CreateWindow");
    fx.complete_windows();
}

fn state_and_rung(fx: &Fixture) -> (TaskState, u8) {
    let t1 = fx.task("t1");
    (t1.state, t1.rung)
}

/// The resumed session's `ResumeSession`, answered `Resumed`.
fn resumed_once(fx: &mut Fixture, effects: &[Effect]) {
    let (op, _) = only_op(effects, "ResumeSession");
    fx.done(op, OpResult::Resumed);
}

/// T15-C1 (probe S): a task rung 4 blocked on its total spend is retried at rung 2
/// with a fresh ceiling; the total before the retry stays for the report.
#[test]
fn a_retried_rung_4_task_gets_a_fresh_ceiling() {
    let (mut fx, window) = working();
    fx.send(fx.now + 3_700, EventKind::Tick);
    assert_eq!(state_and_rung(&fx), (TaskState::Blocked, 4));
    let effects = retry(&mut fx, "t1");
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    fresh_session_up(&mut fx, window);
    fx.tick();
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 2),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert!(total_spend(fx.task("t1"), fx.now).secs >= 3_700);
    assert_alive(&fx);
    // The new epoch has a ceiling of its own.
    fx.send(fx.now + 3_700, EventKind::Tick);
    assert_eq!(state_and_rung(&fx), (TaskState::Blocked, 4));
}

/// T15-I1 (probe A): a worker whose turn ended without `task_done` and whose process
/// then exited between turns loses its fallback count to the restart; the resume
/// re-engages it, with no other live task in the run.
#[test]
fn a_restore_re_engages_a_worker_whose_count_it_lost() {
    let (mut fx, window) = working();
    let effects = fx.turn_completed(window);
    only_op(&effects, "CountCommits");
    exited(&mut fx, window);
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    let messages: Vec<String> = resumes(&effects).into_iter().map(|(_, _, m)| m).collect();
    assert_eq!(messages, vec![RESUME_WORKER.to_string()], "{effects:#?}");
    for _ in 0..3 {
        fx.send(fx.now + 700, EventKind::Tick);
    }
    assert_alive(&fx);
}

/// T15-I1 (probe A3): the same with the count's retry timer lost instead.
#[test]
fn a_restore_re_engages_a_worker_whose_count_timer_it_lost() {
    let (mut fx, window) = working();
    let effects = fx.turn_completed(window);
    let (op, _) = only_op(&effects, "CountCommits");
    fx.done(
        op,
        OpResult::Failed {
            message: "index.lock".into(),
        },
    );
    assert!(fx.task("t1").rounds[0].count_retry_at.is_some());
    exited(&mut fx, window);
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    assert_eq!(resumes(&effects).len(), 1, "{effects:#?}");
    for _ in 0..3 {
        fx.send(fx.now + 700, EventKind::Tick);
    }
    assert_alive(&fx);
}

/// T15-I2 (probe B): a halt charges no session minutes, as a pause charges none.
#[test]
fn a_halt_charges_no_session_minutes() {
    let profile = profile_with("max_writers = 2");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    fx.turn_completed(window_of(&windows, "t2"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "refs/heads/main was rewritten".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Halted);
    fx.now += 2 * 3_600;
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some((BASE.to_string(), "5".repeat(40))),
    });
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    fx.tick();
    let t2 = fx.task("t2");
    assert_eq!(
        (t2.state, t2.rung),
        (TaskState::Working, 0),
        "{:?}",
        t2.history.last()
    );
    assert_alive(&fx);
}

/// T15-I3 (probe Q): a question answered two hours later goes to the same session;
/// the wait breaches no budget.
#[test]
fn a_late_answer_resumes_without_a_budget_breach() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    fx.now += 2 * 3_600;
    let effects = edit(&mut fx, vec![answer("users")]);
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 0),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert_eq!(delivers(&effects).len(), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// T15-I3 (probe Q2): a retry after a long block is not charged for the block.
#[test]
fn a_retry_after_a_long_block_is_not_charged_for_it() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "environment", "need a decision");
    fx.turn_completed(window);
    fx.now += 2 * 3_600;
    assert!(one_reply(&retry(&mut fx, "t1")).is_ok());
    fresh_session_up(&mut fx, window);
    fx.tick();
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 2),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert_alive(&fx);
}

/// T15-I3: time in the gates is not the worker's: a check that takes two hours and
/// fails returns the task to work with no budget breach.
#[test]
fn time_in_the_gates_is_not_charged() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    let window = window_of(&windows, "t1");
    claim(&mut fx, "t1", window, &head_of("t1"));
    let (op, _) = pending_one(&fx, "Check", Some("t1"));
    fx.now += 2 * 3_600;
    fx.done(op, check_result(false));
    fx.tick();
    assert_eq!(
        state_and_rung(&fx),
        (TaskState::Working, 1),
        "{:?}",
        fx.task("t1").history.last()
    );
    assert_alive(&fx);
}

/// T15-minors (probe C): an override of a task whose hold was relaxed with a hand-back
/// due does the hand-back first; the merge candidate follows its result.
#[test]
fn an_override_with_a_due_hand_back_hands_back_first() {
    let mut fx = blocked_t1();
    fx.task_mut("t1").resolving = true;
    // A message waits for the held question, so the merge relaxes the hold.
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: Some("a new brief".into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
    };
    edit(&mut fx, vec![add_dep("t1", "t2"), amend]);
    fx.launch_all();
    fx.merge("t2", &"c2".repeat(20));
    assert!(fx.task("t1").handback_due);
    let effects = override_task(&mut fx, "t1", "trust me");
    let (op, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    let (hand_back, _) = only_op(&effects, "HandBack");
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").merge_op, Some(hand_back));
    let tip = "d4".repeat(20);
    let effects = fx.done(
        hand_back,
        OpResult::HandedBack {
            files: vec![],
            head: Some(tip.clone()),
            onto: Some(HEAD.into()),
        },
    );
    let (_, kind) = only_op(&effects, "MergeCandidate");
    assert!(format!("{kind:?}").contains(&tip), "{kind:?}");
    assert_alive(&fx);
}

/// T15-minors (probe L): an override of a blocked task with an accepted claim merges
/// that claim only while it is the branch's tip; later commits need a retry.
#[test]
fn an_override_merges_an_accepted_claim_only_at_the_branch_tip() {
    for moved in [false, true] {
        let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
        let window = window_of(&windows, "t1");
        claim(&mut fx, "t1", window, &head_of("t1"));
        let (op, _) = pending_one(&fx, "Check", Some("t1"));
        fx.done(op, check_result(false));
        blocked(&mut fx, window, "question", "is the check wrong?");
        let effects = override_task(&mut fx, "t1", "the check is flaky");
        let (op, _) = only_op(&effects, "CountCommits");
        let tip = if moved {
            "e5".repeat(20)
        } else {
            head_of("t1")
        };
        let effects = fx.done(
            op,
            OpResult::Commits {
                count: 3,
                head: tip,
            },
        );
        if moved {
            assert_eq!(
                one_reply(&effects),
                Err(
                    "task t1 has commits after its accepted claim; retry it to have them checked"
                        .into()
                )
            );
            assert_eq!(fx.task("t1").state, TaskState::Blocked);
        } else {
            assert!(one_reply(&effects).is_ok(), "{effects:#?}");
            let (_, kind) = only_op(&effects, "MergeCandidate");
            assert!(format!("{kind:?}").contains(&head_of("t1")), "{kind:?}");
        }
        assert_alive(&fx);
    }
}

/// T15-minors (mutant V2): a task retried while its override's count is in flight is
/// not sent to the merge queue by the count's result.
#[test]
fn an_override_count_that_comes_after_a_retry_is_refused() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    let effects = override_task(&mut fx, "t1", "trust me");
    let (op, _) = only_op(&effects, "CountCommits");
    assert!(one_reply(&retry(&mut fx, "t1")).is_ok());
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert_eq!(
        one_reply(&effects),
        Err("task t1 is working; override applies only to a task in review, or blocked with commits".into())
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_alive(&fx);
}

/// T15-minors (mutant V16): a `task_done` whose check the restart lost leaves no mark
/// on the resumed session: its next turn end runs the fallback.
#[test]
fn a_resumed_session_s_turn_end_runs_the_fallback() {
    let (mut fx, window) = working();
    fx.tool(
        window,
        "task_done",
        serde_json::json!({"summary": "did it", "test": "a::works", "red": "abcdef1"}),
    );
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    resumed_once(&mut fx, &effects);
    let effects = fx.turn_completed(window);
    only_op(&effects, "CountCommits");
    assert_alive(&fx);
}

/// T15-minors (mutant V5): a sub-agent the restart killed does not defer the resumed
/// session's fallback.
#[test]
fn a_sub_agent_the_restart_killed_defers_nothing() {
    let (mut fx, window) = working();
    fx.signal(
        window,
        AgentSignal::SubagentStart {
            agent_id: "a1".into(),
        },
    );
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    resumed_once(&mut fx, &effects);
    let effects = fx.turn_completed(window);
    only_op(&effects, "CountCommits");
    assert_alive(&fx);
}

/// T15-minors (mutant V15): a rate-limit streak the restart cut is over; the resumed
/// session's failed rate-limited turn is a new event.
#[test]
fn a_rate_limit_streak_ends_at_the_restart() {
    let (mut fx, window) = working();
    fx.signal(
        window,
        AgentSignal::ApiRetry {
            error: "rate_limit".into(),
            delay_ms: 1_000,
        },
    );
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&1));
    restart(&mut fx, Vec::new());
    let effects = resume(&mut fx);
    resumed_once(&mut fx, &effects);
    fx.turn_ended(
        window,
        TurnOutcome::Failed {
            error: "rate limit reached".into(),
            kind: FailureKind::RateLimit,
        },
    );
    assert_eq!(fx.run().rate_limits.get("claude"), Some(&2));
    assert_alive(&fx);
}

/// T15-minors (mutant V6): the restart ends a deferred fallback's wait with the
/// session: nothing is left waiting on the resumed session.
#[test]
fn a_restart_ends_a_deferred_fallback_s_wait() {
    let (mut fx, window) = working();
    fx.signal(
        window,
        AgentSignal::SubagentStart {
            agent_id: "a1".into(),
        },
    );
    fx.turn_completed(window);
    assert!(fx.task("t1").rounds[0].fallback_waiting);
    restart(&mut fx, Vec::new());
    let round = &fx.task("t1").rounds[0];
    assert!(!round.fallback_waiting && round.open_subagents.is_empty());
    let effects = resume(&mut fx);
    assert_eq!(resumes(&effects).len(), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// T15-minors (M-4): `run cancel` of a restored, paused run leaves no resume behind.
#[test]
fn a_cancel_clears_the_restore_mark() {
    let (mut fx, _) = working();
    restart(&mut fx, Vec::new());
    assert!(fx.run().restored.is_some());
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    assert_eq!(fx.run().restored, None);
}
