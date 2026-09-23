//! M8a.14 fix round 1: the review's probes as regression tests (rulings T14-C1, I1–I4
//! and the minors). Every sequence ends with the liveness check.

use proto::{AgentRole, FinishAction, PlanEdit, RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::fixture::*;
use super::gates::check_result;
use super::holds::delivers;
use super::merge::{
    candidate, claim, commit, doc_task, head_of, merge, pending, pending_one, start, start_on,
    to_queue, window_of,
};
use super::turns_fixes::assert_alive;
use crate::run::contract::{UNCLAIMED_COMMITS, answer_message, conflict_message};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};

/// A commit the worker made after its claim.
const LATER: &str = "1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e";
/// The worktree's head after a clean hand-back.
const MERGED: &str = "3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e3e";

fn cancel(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    })
}

fn resume(fx: &mut Fixture, rebaseline: Option<(&str, &str)>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: rebaseline.map(|(b, h)| (b.to_string(), h.to_string())),
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

/// t1's candidate conflicts and its hand-back is in flight; returns that op.
fn conflicted(fx: &mut Fixture) -> u64 {
    let (op, _) = candidate(fx, "t1");
    fx.done(
        op,
        OpResult::Conflict {
            files: vec!["docs/t1/a.md".into()],
        },
    );
    pending_one(fx, "HandBack", Some("t1")).0
}

/// Acknowledges every delivery in flight.
fn acknowledge(fx: &mut Fixture) {
    let ids: Vec<u64> = fx
        .run()
        .outbox
        .iter()
        .filter(|m| m.delivered_at.is_some())
        .map(|m| m.id)
        .collect();
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: true,
        error: None,
    });
}

/// Probe P1 (ruling T14-C1): a hand-back merged onto a tip past the claimed commit
/// (the worker committed after `task_done`) never re-queues: clean, the task is back to
/// work and its next claim passes every gate; conflicted, the resolution does too.
#[test]
fn a_hand_back_onto_a_post_claim_commit_goes_back_through_the_gates() {
    for files in [vec![], vec!["docs/t1/a.md".to_string()]] {
        let (mut fx, windows) = start(&[doc_task("t1", "")]);
        let window = window_of(&windows, "t1");
        to_queue(&mut fx, "t1", window);
        let hand_back = conflicted(&mut fx);
        let effects = fx.done(
            hand_back,
            OpResult::HandedBack {
                files: files.clone(),
                head: Some(if files.is_empty() { MERGED } else { LATER }.into()),
                onto: Some(LATER.into()),
            },
        );
        assert!(
            ops_in(&effects, "MergeCandidate").is_empty(),
            "{effects:#?}"
        );
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Working, "{files:?}");
        assert!(!t1.handed_back);
        let told = if files.is_empty() {
            UNCLAIMED_COMMITS.to_string()
        } else {
            conflict_message(&files)
        };
        assert_eq!(delivers(&effects), vec![told]);
        assert_alive(&fx);
        let effects = claim(&mut fx, "t1", window, &head_of("t1n"));
        assert_eq!(fx.task("t1").state, TaskState::Check, "every gate again");
        assert_eq!(ops_in(&effects, "Check").len(), 1, "{effects:#?}");
        assert_alive(&fx);
    }
}

/// Probe P2 (ruling T14-I1): `run cancel` while t1's merge runs is deferred; the merge
/// lands, so t1 is `merged` and the report counts it.
#[test]
fn a_cancel_during_a_merge_that_lands_is_too_late() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    let effects = cancel(&mut fx);
    let text = replies(&effects)[0].clone().unwrap();
    assert!(text.contains("t1's merge is in flight"), "{text}");
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert_alive(&fx);
    fx.done(op, OpResult::Merged { commit: commit(1) });
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.merge_commit, Some(commit(1)));
    let logged = |text: &str| fx.run().log.iter().any(|l| l.text.contains(text));
    assert!(logged("the cancel of t1 arrived too late"));
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t1")) {
        fx.done(op, OpResult::Removed { salvage_ref: None });
    }
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    let logged = |text: &str| fx.run().log.iter().any(|l| l.text.contains(text));
    assert!(logged("complete: 1 merged, 0 cancelled"));
}

/// Probe P2b (ruling T14-I1): a `cancel_task` edit while t1's merge runs is deferred.
/// A merge that lands keeps t1 and its dependent; a conflict, or refs found moved,
/// applies the cancel, and the dependent is then `blocked(dep_cancelled)`.
#[test]
fn a_cancel_edit_during_a_merge_applies_only_if_it_does_not_land() {
    for outcome in ["merged", "conflict", "ref moved"] {
        let lands = outcome == "merged";
        let tasks = [doc_task("t1", ""), doc_task("t2", "deps = [\"t1\"]")];
        let (mut fx, windows) = start(&tasks);
        to_queue(&mut fx, "t1", window_of(&windows, "t1"));
        let (op, _) = candidate(&fx, "t1");
        let effects = edit(
            &mut fx,
            vec![PlanEdit::CancelTask {
                task_id: "t1".into(),
            }],
        );
        assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
        assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
        assert_eq!(fx.task("t2").state, TaskState::Pending);
        let result = match outcome {
            "merged" => OpResult::Merged { commit: commit(1) },
            "conflict" => OpResult::Conflict {
                files: vec!["docs/t1/a.md".into()],
            },
            _ => OpResult::RefMoved {
                reason: "moved".into(),
            },
        };
        let effects = fx.done(op, result);
        if lands {
            assert_eq!(fx.task("t1").state, TaskState::Merged);
            assert_eq!(fx.task("t2").state, TaskState::Preparing);
        } else {
            assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
            assert_eq!(fx.task("t1").state, TaskState::Cancelled);
            let block = fx.task("t2").block.clone().unwrap();
            assert_eq!(block.reason, proto::BlockReason::DepCancelled);
            assert_eq!(fx.run().run_head, BASE);
        }
        assert_alive(&fx);
    }
}

/// Probe P3 (ruling T14-I2): a worktree prepared while the run is halted starts no
/// worker; the first running pass after `resume --rebaseline` does.
#[test]
fn a_worktree_prepared_while_halted_starts_no_worker() {
    let profile = profile_with("max_writers = 1");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", ""), doc_task("t2", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (prepare, _) = pending_one(&fx, "PrepareWorktree", Some("t2"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "run ref moved".into(),
        },
    );
    let effects = fx.done(prepare, OpResult::Worktree { head: BASE.into() });
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    assert!(fx.task("t2").worktree_live);
    let effects = fx.tick();
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert_alive(&fx);
    let effects = resume(&mut fx, Some((BASE, BASE)));
    let windows = ops_in(&effects, "CreateWindow");
    assert_eq!(windows.len(), 1, "{effects:#?}");
    assert_eq!(op_task(&windows[0].1), "t2");
    assert_alive(&fx);
}

/// Ruling T14-I2 for a review: a review worktree prepared while the run is halted
/// starts no reviewer; the review is prepared again and starts after the resume.
#[test]
fn a_review_prepared_while_halted_starts_no_reviewer() {
    let plan = plan_with(PROFILE, &[doc_task("t1", ""), doc_task("t2", "")]);
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let windows = fx.launch_all();
    // t2 reaches the merge queue through an override; t1 reaches review.
    claim(&mut fx, "t2", window_of(&windows, "t2"), &head_of("t2"));
    let (op, _) = pending_one(&fx, "Check", Some("t2"));
    fx.done(op, check_result(true));
    assert_eq!(fx.task("t2").state, TaskState::Review);
    let reply = fx.reply();
    fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t2".into(),
        reason: "trusted".into(),
    });
    claim(&mut fx, "t1", window_of(&windows, "t1"), &head_of("t1"));
    let (op, _) = pending_one(&fx, "Check", Some("t1"));
    fx.done(op, check_result(true));
    let (review, _) = pending_one(&fx, "PrepareReview", Some("t1"));
    let (op, _) = candidate(&fx, "t2");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "run ref moved".into(),
        },
    );
    let effects = fx.done(
        review,
        OpResult::Review {
            base: BASE.into(),
            head: head_of("t1"),
            patch: "diff".into(),
        },
    );
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Review);
    assert_alive(&fx);
    let effects = resume(&mut fx, Some((BASE, BASE)));
    assert_eq!(ops_in(&effects, "PrepareReview").len(), 1, "{effects:#?}");
    let (review, _) = pending_one(&fx, "PrepareReview", Some("t1"));
    let effects = fx.done(
        review,
        OpResult::Review {
            base: BASE.into(),
            head: head_of("t1"),
            patch: "diff".into(),
        },
    );
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1, "{effects:#?}");
    assert_alive(&fx);
}

/// Probe P8 (ruling T14-I3): the worker was told of the queue's conflict and is
/// resolving it (a merge in progress) when it blocks and gains a dependency. When the
/// dependency merges, nothing is handed back into the worktree: the answer goes out,
/// and the new run head is handed back at the worker's next `task_done`, before any
/// gate. Clean, the new head goes through the gates; conflicted, the worker is told.
#[test]
fn a_told_conflict_is_not_handed_back_into_when_the_task_is_held() {
    for clean in [true, false] {
        let (mut fx, windows) = start(&[doc_task("t1", ""), doc_task("t2", "")]);
        let window = window_of(&windows, "t1");
        to_queue(&mut fx, "t1", window);
        let hand_back = conflicted(&mut fx);
        let files = vec!["docs/t1/a.md".to_string()];
        let effects = fx.done(
            hand_back,
            OpResult::HandedBack {
                files: files.clone(),
                head: Some(head_of("t1")),
                onto: Some(head_of("t1")),
            },
        );
        assert_eq!(delivers(&effects), vec![conflict_message(&files)]);
        acknowledge(&mut fx);
        fx.signal(window, AgentSignal::TurnStarted);
        let args = json!({"kind": "question", "reason": "which side wins?"});
        fx.tool_as(AgentRole::Worker, window, "t1", "task_blocked", args);
        fx.turn_completed(window);
        let effects = edit(
            &mut fx,
            vec![PlanEdit::AddDep {
                task_id: "t1".into(),
                dep: "t2".into(),
            }],
        );
        assert!(ops_in(&effects, "AbortMerge").is_empty(), "{effects:#?}");
        edit(
            &mut fx,
            vec![PlanEdit::Answer {
                task_id: "t1".into(),
                text: "theirs".into(),
            }],
        );
        to_queue(&mut fx, "t2", window_of(&windows, "t2"));
        let effects = merge(&mut fx, "t2", &commit(2));
        assert!(ops_in(&effects, "HandBack").is_empty(), "{effects:#?}");
        let t1 = fx.task("t1");
        assert_eq!(t1.state, TaskState::Working);
        assert!(!t1.awaiting_deps);
        assert!(t1.handback_due);
        assert_eq!(delivers(&effects), vec![answer_message("theirs")]);
        assert_alive(&fx);

        // The worker finishes the merge and claims: the new run head comes first.
        let resolved = head_of("t1r");
        let effects = claim(&mut fx, "t1", window, &resolved);
        for gate in ["Check", "Proof", "PrepareReview", "MergeCandidate"] {
            assert!(ops_in(&effects, gate).is_empty(), "{gate}: {effects:#?}");
        }
        let (op, kind) = pending_one(&fx, "HandBack", Some("t1"));
        assert_eq!(
            kind,
            OpKind::HandBack {
                worktree: task_path("t1"),
                run_head: commit(2),
                task_head: Some(resolved.clone()),
            }
        );
        assert_alive(&fx);
        let result = if clean {
            OpResult::HandedBack {
                files: vec![],
                head: Some(MERGED.into()),
                onto: Some(resolved.clone()),
            }
        } else {
            OpResult::HandedBack {
                files: files.clone(),
                head: Some(resolved.clone()),
                onto: Some(resolved.clone()),
            }
        };
        let effects = fx.done(op, result);
        if clean {
            assert_eq!(fx.task("t1").state, TaskState::Check);
            assert_eq!(fx.task("t1").head.as_deref(), Some(MERGED));
            let (_, check) = pending_one(&fx, "Check", Some("t1"));
            let OpKind::Check { scratch, .. } = check else {
                unreachable!()
            };
            assert_eq!(scratch.unwrap().commit, MERGED);
        } else {
            assert_eq!(fx.task("t1").state, TaskState::Working);
            assert_eq!(delivers(&effects), vec![conflict_message(&files)]);
            claim(&mut fx, "t1", window, &head_of("t1s"));
            assert_eq!(
                fx.task("t1").state,
                TaskState::Check,
                "gates on the resolution"
            );
        }
        assert_alive(&fx);
    }
}

/// Ruling T14-I4 and review m2: accept and discard apply only to a complete run —
/// refused on a running, halted or paused one — except that a cancelled, halted run
/// may be discarded without a rebaseline.
#[test]
fn accept_and_discard_apply_only_to_a_complete_run() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    for state in [RunState::Running, RunState::Halted, RunState::Paused] {
        fx.run_mut().state = state;
        for action in [FinishAction::Accept, FinishAction::Discard] {
            let effects = finish(&mut fx, action);
            let refused = replies(&effects);
            assert_eq!(refused.len(), 1, "{state:?} {action:?}");
            assert!(refused[0].is_err(), "{state:?} {action:?}: {refused:?}");
            assert!(ops_in(&effects, "Accept").is_empty());
            assert!(ops_in(&effects, "Discard").is_empty());
        }
    }
    fx.run_mut().state = RunState::Running;

    // Cancelled while halted: discard needs no rebaseline; accept stays refused.
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = candidate(&fx, "t1");
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "moved".into(),
        },
    );
    cancel(&mut fx);
    assert_eq!(fx.task("t1").state, TaskState::Cancelled);
    fx.signal(
        window_of(&windows, "t1"),
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t1")) {
        fx.done(op, OpResult::Removed { salvage_ref: None });
    }
    assert_eq!(fx.run().state, RunState::Halted);
    let effects = finish(&mut fx, FinishAction::Accept);
    assert!(replies(&effects)[0].is_err());
    let effects = finish(&mut fx, FinishAction::Discard);
    let (op, _) = only(&effects, "Discard");
    fx.done(
        op,
        OpResult::Finished {
            outcome: "discarded".into(),
            kept_branches: vec![],
        },
    );
    assert_eq!(fx.run().state, RunState::Discarded);
}

fn only(effects: &[Effect], name: &str) -> (u64, OpKind) {
    let ops = ops_in(effects, name);
    assert_eq!(ops.len(), 1, "{effects:#?}");
    ops[0].clone()
}

/// Review m1: refs that cannot be read are read again; a second failure halts the run,
/// and a plain `run resume` (no rebaseline) verifies them once more.
#[test]
fn a_failed_ref_read_is_retried_then_halts_with_a_plain_resume() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    let failed = || OpResult::Failed {
        message: "index.lock".into(),
    };
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    let effects = fx.done(op, failed());
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(ops_in(&effects, "VerifyRefs").len(), 1, "retried");
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, failed());
    assert_eq!(fx.run().state, RunState::Halted);
    let reason = fx.run().halted_reason.clone().unwrap();
    assert!(reason.contains("could not verify the refs"), "{reason}");
    assert_alive(&fx);
    let effects = resume(&mut fx, None);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().run_head, commit(1), "nothing re-recorded");
    // Failures count in a row: one after a read that succeeded is retried again.
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, failed());
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(
        op,
        OpResult::RefMoved {
            reason: "moved".into(),
        },
    );
    resume(&mut fx, Some((BASE, &commit(1))));
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, failed());
    assert_eq!(fx.run().state, RunState::Running);
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
}

/// Review m3: a `BaseAdvanced` to the recorded base is no advance.
#[test]
fn an_advance_to_the_recorded_base_is_no_advance() {
    let (mut fx, _) = start(&[doc_task("t1", "")]);
    let revision = fx.run().revision;
    fx.next(EventKind::BaseAdvanced {
        run_id: RUN_ID.into(),
        to: BASE.into(),
        commits: 0,
    });
    assert_eq!(fx.run().base_moved, None);
    assert_eq!(fx.run().revision, revision);
    assert_alive(&fx);
}

/// Review m4: a cancelled task's salvage ref follows the highest one recorded, as the
/// merge's and accept's do.
#[test]
fn a_cancelled_worktree_takes_the_next_salvage_number() {
    let (mut fx, windows) = start(&[doc_task("t1", "")]);
    let salvage = |n: u32| format!("refs/anthrex/salvage/{RUN_ID}/t1/{n}");
    fx.task_mut("t1").salvage_refs = vec![salvage(3)];
    cancel(&mut fx);
    let effects = fx.signal(
        window_of(&windows, "t1"),
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    let (_, kind) = only(&effects, "RemoveWorktree");
    let OpKind::RemoveWorktree { salvage_ref, .. } = kind else {
        unreachable!()
    };
    assert_eq!(salvage_ref, salvage(4));
}
