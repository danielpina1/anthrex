//! M8a.14: decision 21's ref guard (a moved run ref or a rewritten base halts; an
//! advanced base is recorded), decision 37's completion (`VerifyRefs`, the final check,
//! the report), blocked tasks, the `finish` edit, `run cancel`, and an accept that
//! conflicts. Every sequence ends with the liveness check.

use proto::{FinishAction, PlanEdit, RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::gates::check_result;
use super::merge::{
    candidate, claim, commit, doc_task, head_of, merge, pending, pending_one, start, start_on,
    to_queue, window_of,
};
use super::turns_fixes::assert_alive;
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};
use crate::run::model::BaseMoved;
use crate::run::snapshot::snapshot;

const MOVED: &str = "refs/heads/anthrex/engine-test-3f9a/integration moved from 1eeeeee to 9999999";
const REWRITTEN: &str = "refs/heads/main was rewritten: b0b0b0b is not an ancestor of 7777777";
/// The base head after an advance.
const X: &str = "abababababababababababababababababababab";

fn resume(fx: &mut Fixture, rebaseline: Option<(&str, &str)>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: rebaseline.map(|(b, h)| (b.to_string(), h.to_string())),
    })
}

fn base_advanced(fx: &mut Fixture, to: &str, commits: u32) -> Vec<Effect> {
    fx.next(EventKind::BaseAdvanced {
        run_id: RUN_ID.into(),
        to: to.into(),
        commits,
    })
}

fn finish(fx: &mut Fixture, action: FinishAction) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
    })
}

fn attention(fx: &Fixture) -> Vec<String> {
    snapshot(&fx.state, fx.now).runs[0].attention.clone()
}

fn persisted(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::Persist { .. }))
}

/// Completes the pending `VerifyRefs` with `result`.
fn verify(fx: &mut Fixture, result: OpResult) -> Vec<Effect> {
    let (op, _) = pending_one(fx, "VerifyRefs", None);
    fx.done(op, result)
}

/// A run whose only task `t1` merged at `commit(1)`, its clean-up done.
fn all_merged() -> Fixture {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    fx
}

#[test]
fn ref_moved_halts_the_run() {
    for reason in [MOVED, REWRITTEN] {
        let profile = profile_with("max_writers = 1");
        let tasks = [doc_task("t1", ""), doc_task("t2", ""), doc_task("t3", "")];
        let (mut fx, windows) = start_on(&profile, &tasks);
        to_queue(&mut fx, "t1", window_of(&windows, "t1"));
        let windows = fx.launch_all();
        let t2 = window_of(&windows, "t2");
        // t2's claim is accepted; its check is still running when the run halts.
        claim(&mut fx, "t2", t2, &head_of("t2"));
        let (check, _) = pending_one(&fx, "Check", Some("t2"));
        base_advanced(&mut fx, X, 2);
        let (op, _) = candidate(&fx, "t1");
        let effects = fx.done(
            op,
            OpResult::RefMoved {
                reason: reason.into(),
            },
        );
        assert_eq!(fx.run().state, RunState::Halted, "{reason}");
        assert_eq!(fx.run().halted_reason.as_deref(), Some(reason));
        assert!(ops_in(&effects, "MergeCandidate").is_empty());
        assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
        assert_eq!(fx.run().merge_queue, vec!["t1"]);
        assert_alive(&fx);

        // Nothing merges or dispatches while halted; the windows keep running.
        let effects = fx.done(check, check_result(true));
        assert_eq!(fx.task("t2").state, TaskState::MergeQueue);
        assert!(
            ops_in(&effects, "MergeCandidate").is_empty(),
            "{effects:#?}"
        );
        assert!(
            ops_in(&effects, "PrepareWorktree").is_empty(),
            "{effects:#?}"
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::KillWindow { .. }))
        );
        assert_eq!(fx.task("t3").state, TaskState::Queued);
        let effects = fx.tick();
        assert!(ops_in(&effects, "MergeCandidate").is_empty());
        assert_alive(&fx);

        let effects = resume(&mut fx, None);
        let refused = replies(&effects);
        assert_eq!(refused.len(), 1);
        let text = refused[0].clone().unwrap_err();
        assert!(text.contains(reason), "{text}");
        assert!(text.contains("--rebaseline"), "{text}");
        assert_eq!(fx.run().state, RunState::Halted);

        let (base, head) = (X, "5555555555555555555555555555555555555555");
        let effects = resume(&mut fx, Some((base, head)));
        assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
        let run = fx.run();
        assert_eq!(run.state, RunState::Running);
        assert_eq!((run.base_sha.as_str(), run.run_head.as_str()), (base, head));
        assert_eq!(run.base_moved, None);
        assert_eq!(run.halted_reason, None);
        let merges = ops_in(&effects, "MergeCandidate");
        assert_eq!(merges.len(), 1, "{effects:#?}");
        let OpKind::MergeCandidate {
            expected_run_head,
            expected_base,
            ..
        } = &merges[0].1
        else {
            unreachable!()
        };
        assert_eq!(
            (expected_base.as_str(), expected_run_head.as_str()),
            (base, head)
        );
        assert_eq!(fx.task("t3").state, TaskState::Preparing);
        assert_alive(&fx);
    }
}

#[test]
fn base_advanced_is_recorded_and_the_run_goes_on() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        doc_task("t3", "deps = [\"t2\"]"),
    ];
    let (mut fx, windows) = start(&tasks);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    candidate(&fx, "t2");
    assert_ne!(fx.run().run_head, BASE);

    let revision = fx.run().revision;
    let effects = base_advanced(&mut fx, X, 2);
    let now = fx.now;
    let run = fx.run();
    assert_eq!(
        run.base_moved,
        Some(BaseMoved {
            from: BASE.into(),
            to: X.into(),
            commits: 2,
            seen_at: now,
        })
    );
    assert_ne!(run.base_moved.as_ref().unwrap().from, run.run_head);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.revision, revision + 1);
    assert!(persisted(&effects), "{effects:#?}");
    let line = "base main moved from b0b0b0b to abababa (2 new commits); accept will list them";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:?}",
        attention(&fx)
    );

    // The pending merge completes, and the next task is dispatched.
    let effects = merge(&mut fx, "t2", &commit(2));
    assert_eq!(fx.task("t2").state, TaskState::Merged);
    assert_eq!(fx.task("t3").state, TaskState::Preparing);
    assert!(
        !ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:#?}"
    );

    // The same `to` changes nothing; a new one replaces it.
    let revision = fx.run().revision;
    let effects = base_advanced(&mut fx, X, 2);
    assert_eq!(fx.run().revision, revision);
    assert!(!persisted(&effects), "{effects:#?}");
    let y = "cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd";
    base_advanced(&mut fx, y, 5);
    let moved = fx.run().base_moved.clone().unwrap();
    assert_eq!((moved.from.as_str(), moved.to.as_str()), (BASE, y));
    assert_eq!((moved.commits, moved.seen_at), (5, fx.now));
    assert_alive(&fx);
}

#[test]
fn completion_checks_refs_then_completes() {
    let mut fx = all_merged();
    let (_, kind) = pending_one(&fx, "VerifyRefs", None);
    assert_eq!(
        kind,
        OpKind::VerifyRefs {
            root: "/tmp/x".into(),
            base_branch: "main".into(),
            expected_base: BASE.into(),
            run_branch: format!("anthrex/{RUN_ID}/integration"),
            expected_run_head: commit(1),
        }
    );
    assert_eq!(fx.run().state, RunState::Running);
    assert_alive(&fx);
    let effects = verify(&mut fx, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    assert!(effects.contains(&Effect::WriteReport {
        run_id: RUN_ID.into()
    }));
    assert!(!fx.run().final_check_failed);

    // A rebaselined run head differs from the last green candidate: the check runs in
    // the integration worktree before `complete`.
    let mut fx = all_merged();
    verify(
        &mut fx,
        OpResult::RefMoved {
            reason: MOVED.into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Halted);
    let head = "5555555555555555555555555555555555555555";
    resume(&mut fx, Some((X, head)));
    let (_, kind) = pending_one(&fx, "VerifyRefs", None);
    let OpKind::VerifyRefs {
        expected_run_head, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(expected_run_head, head);
    let effects = verify(&mut fx, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Running);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::WriteReport { .. }))
    );
    let int = task_path("integration");
    let (op, kind) = pending_one(&fx, "Check", None);
    assert_eq!(
        kind,
        OpKind::Check {
            dir: int.clone(),
            command: "cargo test".into(),
            timeout_secs: 1800,
            env: vec![("TARGET".into(), format!("{}/target", int.display()))],
            scratch: None,
        }
    );
    assert_alive(&fx);
    let effects = fx.done(op, check_result(false));
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(fx.run().final_check_failed);
    assert!(attention(&fx).contains(&"final check failed on the run head".to_string()));
    assert!(effects.contains(&Effect::WriteReport {
        run_id: RUN_ID.into()
    }));
}

#[test]
fn blocked_tasks_keep_the_run_running_with_attention() {
    let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool_as(
        proto::AgentRole::Worker,
        window_of(&windows, "t2"),
        "t2",
        "task_blocked",
        args,
    );
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let effects = merge(&mut fx, "t1", &commit(1));
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    let effects = fx.tick();
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Running);
    assert!(
        attention(&fx).contains(&"t2 blocked (question): which table?".to_string()),
        "{:?}",
        attention(&fx)
    );
    assert_alive(&fx);
}

#[test]
fn finish_cancels_unstarted_then_completes_when_live_ones_end() {
    let profile = profile_with("max_writers = 2");
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        doc_task("t3", ""),
        doc_task("t4", "deps = [\"t1\"]"),
    ];
    let (mut fx, windows) = start_on(&profile, &tasks);
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert_eq!(fx.task("t4").state, TaskState::Pending);
    let effects = edit(&mut fx, vec![PlanEdit::Finish]);
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    for id in ["t3", "t4"] {
        assert_eq!(fx.task(id).state, TaskState::Cancelled, "{id}");
    }
    for id in ["t1", "t2"] {
        assert_eq!(fx.task(id).state, TaskState::Working, "{id}");
    }
    assert_alive(&fx);

    // t2 blocks; t1 merges. Then nothing is live: t2 is cancelled, salvaged, and the
    // run completes.
    let t2 = window_of(&windows, "t2");
    let args = json!({"kind": "question", "reason": "which table?"});
    fx.tool_as(proto::AgentRole::Worker, t2, "t2", "task_blocked", args);
    assert_eq!(fx.task("t2").state, TaskState::Blocked);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    let effects = fx.done(op, OpResult::Merged { commit: commit(1) });
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    assert!(effects.contains(&Effect::KillWindow { window_id: t2 }));
    assert_eq!(fx.task("t4").state, TaskState::Cancelled);
    assert_alive(&fx);
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t1")) {
        fx.done(op, OpResult::Removed { salvage_ref: None });
    }
    assert!(pending(&fx, "VerifyRefs", None).is_empty());
    let effects = fx.signal(
        t2,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    let (op, kind) = pending_one(&fx, "RemoveWorktree", Some("t2"));
    assert!(ops_in(&effects, "RemoveWorktree").len() == 1);
    let salvage = format!("refs/anthrex/salvage/{RUN_ID}/t2/1");
    assert!(matches!(kind, OpKind::RemoveWorktree { salvage_ref, .. } if salvage_ref == salvage));
    fx.done(
        op,
        OpResult::Removed {
            salvage_ref: Some(salvage.clone()),
        },
    );
    assert_eq!(fx.task("t2").salvage_refs, vec![salvage]);
    verify(&mut fx, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
}

#[test]
fn cancel_kills_salvages_and_completes() {
    let profile = profile_with("max_writers = 1");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", ""), doc_task("t2", "")]);
    let t1 = window_of(&windows, "t1");
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(effects.contains(&Effect::KillWindow { window_id: t1 }));
    for id in ["t1", "t2"] {
        assert_eq!(fx.task(id).state, TaskState::Cancelled, "{id}");
    }
    assert!(
        pending(&fx, "VerifyRefs", None).is_empty(),
        "waits for the exit"
    );
    assert_alive(&fx);
    fx.signal(
        t1,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    let (op, _) = pending_one(&fx, "RemoveWorktree", Some("t1"));
    let salvage = format!("refs/anthrex/salvage/{RUN_ID}/t1/1");
    fx.done(
        op,
        OpResult::Removed {
            salvage_ref: Some(salvage.clone()),
        },
    );
    assert_eq!(fx.task("t1").salvage_refs, vec![salvage]);
    let effects = verify(&mut fx, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(effects.contains(&Effect::WriteReport {
        run_id: RUN_ID.into()
    }));
    let done = fx;

    // A paused run runs again to complete.
    let (mut fx, _) = start(&[doc_task("t1", "")]);
    fx.run_mut().state = RunState::Paused;
    fx.run_mut().paused_from = Some(RunState::Running);
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.run().paused_from, None);
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    assert_alive(&fx);

    // A complete run cannot be cancelled.
    let mut fx = done;
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(replies(&effects)[0].is_err(), "{effects:#?}");
}

#[test]
fn accept_conflict_keeps_the_run_complete() {
    let mut fx = all_merged();
    base_advanced(&mut fx, X, 3);
    verify(&mut fx, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);

    let effects = finish(&mut fx, FinishAction::Accept);
    assert!(replies(&effects).is_empty(), "the reply waits for the op");
    let (op, kind) = pending_one(&fx, "Accept", None);
    let OpKind::Accept {
        expected_base,
        message,
        base_branch,
        run_branch,
        branch_prefix,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(expected_base, X);
    assert_eq!(message, "anthrex: accept run engine-test-3f9a: Engine test");
    assert_eq!(base_branch, "main");
    assert_eq!(run_branch, format!("anthrex/{RUN_ID}/integration"));
    assert_eq!(branch_prefix, format!("anthrex/{RUN_ID}/"));

    let files = vec!["docs/a.md".to_string(), "docs/b.md".to_string()];
    let effects = fx.done(op, OpResult::AcceptConflict { files });
    let text = "accept conflicts with 3 commits on main: docs/a.md, docs/b.md; resolve by merging anthrex/engine-test-3f9a/integration into main yourself, or discard the run";
    assert_eq!(replies(&effects), vec![Err(text.to_string())]);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(ops_in(&effects, "Discard").is_empty());
    assert!(fx.run().log.iter().any(|l| l.text.contains(text)), "logged");
    // Its branches are kept: accept can be tried again.
    let effects = finish(&mut fx, FinishAction::Accept);
    assert_eq!(ops_in(&effects, "Accept").len(), 1, "{effects:#?}");
}
