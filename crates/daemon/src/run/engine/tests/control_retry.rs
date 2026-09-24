//! M8a.15: `run retry` (decision 42) and `run override` of a blocked task (decision
//! 35), with the carries on both: counts reset, the start commit reused, the hand-back
//! context ended, a held task handed back first and never turned into a question, and
//! a blocked task's commits counted before it is overridden. Split from `control.rs`
//! for size. Every sequence ends with the liveness check.

use proto::{BlockReason, GateCounts, PlanEdit, TaskState};

use super::control::{blocked, override_task, retry};
use super::dispatch::{edit, replies};
use super::done::{killed, one_reply};
use super::fixture::*;
use super::gates::only_op;
use super::holds::{add_dep, blocked_t1};
use super::liveness::assert_alive;
use super::turns::{killed_exit, working};
use crate::run::contract::handover_prompt;
use crate::run::engine::{OpKind, OpResult, ResolutionAt};
use crate::run::model::FreshSession;
use crate::run::roster::escalate;
#[test]
fn retry_resets_counts_and_starts_a_fresh_session_at_rung_2() {
    let (mut fx, window) = working();
    blocked(&mut fx, window, "mis_sized", "too big");
    let (reason, text) = super::done::block_of(&fx);
    assert_eq!(reason, BlockReason::MisSized);
    killed_exit(&mut fx, window);
    // What a long task leaves behind: counts, a hand-back context and a fresh session
    // decided on before the block (carries T12-later, T14-R2).
    {
        let t1 = fx.task_mut("t1");
        t1.failures = 2;
        t1.bounces.check = 2;
        t1.bounces.review = 1;
        t1.budget_exceeded = 1;
        t1.conflicts = 1;
        t1.handed_back = true;
        t1.resolution = Some(ResolutionAt {
            onto: HEAD.into(),
            run_head: BASE.into(),
            files: vec!["crates/a/x.rs".into()],
        });
        t1.fresh_session = Some(FreshSession {
            reason: "stale".into(),
            append: Some("[anthrex] stale message".into()),
        });
    }
    let route = fx.task("t1").route.clone();
    let start = fx.task("t1").start_commit.clone();
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 retried at rung 2: a fresh session starts".to_string())
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (
            t1.state,
            t1.rung,
            t1.failures,
            t1.budget_exceeded,
            t1.conflicts
        ),
        (TaskState::Working, 2, 1, 0, 0)
    );
    assert_eq!(t1.bounces, GateCounts::default());
    assert_eq!(t1.block, None);
    assert!(!t1.handed_back && t1.resolution.is_none());
    assert_eq!(t1.route, escalate(&fx.run().roster, &route));
    // The old session has exited: the diff at once, from the task's own start commit.
    let (op, kind) = only_op(&effects, "DiffSoFar");
    let OpKind::DiffSoFar { start: from, .. } = kind else {
        unreachable!()
    };
    assert_eq!(Some(from), start, "a retry reuses the start commit");
    assert!(ops_in(&effects, "PrepareWorktree").is_empty());
    let (stat, patch) = (" a | 1 +".to_string(), "+x".to_string());
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: stat.clone(),
            patch: patch.clone(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let why = format!("the user retried it (it was blocked(mis_sized): {text})");
    let expected = handover_prompt(fx.run(), fx.task("t1"), &why, &stat, &patch);
    assert_eq!(first_turn, expected, "no stale append");
    assert_eq!(fx.task("t1").session, 2);
    fx.complete_windows();
    assert_alive(&fx);
}

#[test]
fn retry_refuses_l_and_dep_cancelled() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "M", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Err("task t1 is working; retry applies only to a blocked task".into())
    );
    assert_eq!(
        one_reply(&retry(&mut fx, "t9")),
        Err("unknown task t9".into())
    );
    blocked(&mut fx, window, "mis_sized", "far too big");
    killed_exit(&mut fx, window);
    assert_eq!(fx.task("t1").size, proto::Size::L);
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Err("task t1 is L; split it first".into())
    );
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    let effects = retry(&mut fx, "t2");
    assert_eq!(
        one_reply(&effects),
        Err(
            "task t2 is blocked(dep_cancelled); retry cannot bring back a cancelled dependency"
                .into()
        )
    );
    assert_eq!(
        fx.task("t2").block.as_ref().unwrap().reason,
        BlockReason::DepCancelled
    );
    assert_alive(&fx);

    // Only a running or paused run takes a retry.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.start(false);
    assert_eq!(
        one_reply(&retry(&mut fx, "t1")),
        Err(format!("run {RUN_ID} is awaiting_approval"))
    );
}

/// Carries T11-RR and M8a.14's: a started task blocked for another reason than a
/// question keeps its own block while it waits for a dependency; `run retry` waits for
/// the dependency, then hands the run head back before the fresh session starts, and
/// never turns the block into a question.
#[test]
fn a_retried_held_task_is_handed_back_before_its_fresh_session() {
    let mut fx = blocked_t1();
    let window = fx.task("t1").rounds[0].window_id.unwrap();
    let conflict = proto::BlockInfo {
        reason: BlockReason::Conflict,
        text: "its branch conflicts with the run branch again: crates/a/x.rs".into(),
    };
    fx.task_mut("t1").block = Some(conflict.clone());
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    assert!(fx.task("t1").awaiting_deps);
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Err("task t1 waits for its dependencies; retry it once they are merged".into())
    );
    fx.launch_all();
    let run_head = "c2".repeat(20);
    let effects = fx.merge("t2", &run_head);
    assert!(
        ops_in(&effects, "HandBack").is_empty(),
        "no question, no answer"
    );
    assert_eq!(fx.task("t1").block, Some(conflict.clone()), "its own block");

    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 retried at rung 2: the run head is merged into its worktree first".into())
    );
    assert!(killed(&effects, window), "{effects:#?}");
    let (op, kind) = only_op(&effects, "HandBack");
    let OpKind::HandBack { run_head: head, .. } = kind else {
        unreachable!()
    };
    assert_eq!(head, run_head);
    assert_eq!(
        fx.task("t1").block,
        Some(conflict),
        "blocked until the hand-back"
    );
    let effects = killed_exit(&mut fx, window);
    assert!(
        ops_in(&effects, "DiffSoFar").is_empty(),
        "held: no fresh session yet"
    );
    let effects = fx.done(
        op,
        OpResult::HandedBack {
            files: Vec::new(),
            head: Some(HEAD.into()),
            onto: Some(HEAD.into()),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.awaiting_deps, t1.rung),
        (TaskState::Working, false, 2)
    );
    only_op(&effects, "DiffSoFar");
    assert_alive(&fx);
}

/// Ruling T14-I3 on the retry path: a held task whose worker was resolving a told
/// conflict has a merge in progress in its worktree, so no hand-back goes there; the
/// run head follows its next accepted claim (`handback_due`).
#[test]
fn a_retried_held_task_resolving_a_conflict_is_not_handed_back_into() {
    let mut fx = blocked_t1();
    let window = fx.task("t1").rounds[0].window_id.unwrap();
    fx.task_mut("t1").resolving = true;
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    fx.launch_all();
    fx.merge("t2", &"c2".repeat(20));
    let effects = retry(&mut fx, "t1");
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.awaiting_deps), (TaskState::Working, false));
    assert!(t1.resolving && t1.handback_due, "the worktree is mid-merge");
    let effects = killed_exit(&mut fx, window);
    only_op(&effects, "DiffSoFar");
    assert_alive(&fx);
}

#[test]
fn override_only_from_review_or_blocked_with_commits() {
    let (mut fx, window) = working();
    let refused = "override applies only to a task in review, or blocked with commits";
    let effects = override_task(&mut fx, "t1", "trust me");
    assert_eq!(
        one_reply(&effects),
        Err(format!("task t1 is working; {refused}"))
    );

    // Blocked, and no accepted claim recorded its commits: they are counted first.
    blocked(&mut fx, window, "question", "which table?");
    assert_eq!(fx.task("t1").head, None);
    let effects = override_task(&mut fx, "t1", "trust me");
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    let (op, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 0,
            head: BASE.into(),
        },
    );
    assert_eq!(
        one_reply(&effects),
        Err(format!("task t1 has no commits; {refused}"))
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);

    let effects = override_task(&mut fx, "t1", "trust me");
    let (op, _) = only_op(&effects, "CountCommits");
    let again = override_task(&mut fx, "t1", "twice");
    assert_eq!(
        one_reply(&again),
        Err("task t1's commits are being counted for an override; wait for its reply".into())
    );
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 goes to the merge queue without review: trust me".into())
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.head.as_deref()),
        (TaskState::MergeQueue, Some(HEAD))
    );
    assert_eq!(t1.merged_without_approval.as_deref(), Some("trust me"));
    let (_, kind) = only_op(&effects, "MergeCandidate");
    let OpKind::MergeCandidate { task_head, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        task_head, HEAD,
        "the counted head, which the gates never saw"
    );
    assert_alive(&fx);

    // A task that never started has no commits: refused without a count.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    let (op, _) = fx.op("PrepareWorktree");
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "no make".into(),
        },
    );
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = override_task(&mut fx, "t1", "trust me");
    assert_eq!(
        one_reply(&effects),
        Err(format!("task t1 has no commits; {refused}"))
    );
    assert!(ops_in(&effects, "CountCommits").is_empty());
    assert_alive(&fx);
}

/// Carry T11-RR: a held task whose fresh session the window limit refused keeps its own
/// block when its dependency merges; only a retry or an answer is handed back.
#[test]
fn a_refused_fresh_launch_that_gains_a_dependency_keeps_its_block() {
    let mut fx = blocked_t1();
    let window = fx.task("t1").rounds[0].window_id.unwrap();
    let effects = retry(&mut fx, "t1");
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 retried at rung 2: a fresh session starts".into())
    );
    let effects = killed_exit(&mut fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.run_mut().limits.max_windows = fx.run().windows_created;
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty());
    let t1 = fx.task("t1");
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::Environment);
    assert!(t1.fresh_session.is_some() && !t1.held_answered);
    edit(&mut fx, vec![add_dep("t1", "t2")]);
    assert!(fx.task("t1").awaiting_deps);
    let effects = fx.merge("t2", &"c2".repeat(20));
    assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::Environment);
    assert_alive(&fx);
}

/// An override's count is told apart from a fallback count still in flight for the
/// same task: each result goes to the op that asked for it.
#[test]
fn an_override_count_is_not_confused_with_a_fallback_count() {
    let (mut fx, window) = working();
    let effects = fx.turn_completed(window);
    let (fallback, _) = only_op(&effects, "CountCommits");
    // Rung 4: the task's total spend reached the next size's budget.
    let later = fx.now + 1_000_000;
    fx.send(later, crate::run::engine::EventKind::Tick);
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = override_task(&mut fx, "t1", "trust me");
    let (op, _) = only_op(&effects, "CountCommits");
    let effects = fx.done(
        fallback,
        OpResult::Commits {
            count: 0,
            head: BASE.into(),
        },
    );
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    let effects = fx.done(
        op,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
        },
    );
    assert_eq!(
        one_reply(&effects),
        Ok("task t1 goes to the merge queue without review: trust me".into())
    );
    assert_alive(&fx);
}

/// Ruling T22-I1b, probe B: an unreviewed S task at its rung-2 route (Claude, `high`),
/// blocked and retried, is escalated again, onto Codex. The runtime it lands on was in
/// the reachable set decisions 50 and 53 were checked against at the start.
#[test]
fn probe_b_a_retried_task_stays_inside_the_reachable_set() {
    let route = "test_mode = \"tdd\"\ntest_to_write = \"a::works\"\n[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\neffort = \"medium\"";
    let config = config::Orchestrator {
        review_small: false,
        ..config::Orchestrator::default()
    };
    let mut fx = Fixture::with_config(&plan_with(PROFILE, &[task("t1", "S", "a", route)]), config);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(
        fx.task("t1").review_level,
        None,
        "the probe's task is not reviewed"
    );
    let reachable = crate::run::reach::reachable_runtimes(fx.run());
    let rung2 = escalate(&fx.run().roster, &fx.task("t1").route);
    assert_eq!(rung2.runtime, proto::Runtime::Claude);
    fx.task_mut("t1").route = rung2;
    blocked(&mut fx, window, "environment", "stuck");
    killed_exit(&mut fx, window);
    let effects = retry(&mut fx, "t1");
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    let runtime = fx.task("t1").route.runtime;
    assert_eq!(runtime, proto::Runtime::Codex);
    assert!(
        reachable.contains(&runtime),
        "retry moved t1 to {runtime:?}, outside the checked set {reachable:?}"
    );
}
