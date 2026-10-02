//! Milestone 9.2 task M9.2.10: replies (decision 30) and the orchestrator's review
//! decisions (decision 31). Once a push carrying a review fix's merge lands, each of
//! its threads gets `Addressed in <sha7> by task <id>.` with anthrex's marker, and the
//! posted comment is anthrex's own from then on, known by its id; a reply whose answer
//! a restart lost is sent again with the same marker, and its comment is never review
//! input. `reply_comment` is alone in its call, validated, made safe and queued;
//! `addresses` makes an added task a review fix, validated, and held outside its stage.

use proto::{PlanEdit, RunState};
use serde_json::{Value, json};

use super::control::resume;
use super::control_restore::restart;
use super::delivery_open::{answer, host_ops};
use super::delivery_review::{asked, logged, noted, on, reviewed, said, state};
use super::delivery_watch::{PR, poll_with, view};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::attention;
use super::merge::{commit, merge, to_queue, window_of};
use super::orch::{answer as orch_answer, edit_plan};
use crate::host::{HostError, PushOutcome, ReplyTarget};
use crate::run::delivery::ThreadState;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::EventKind;
use crate::run::model::{FixOf, OpId};
use crate::run::orch::EditSource;

pub(super) fn by_alice() -> Fixture {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["alice".into()];
    fx
}

/// `c5` (a conversation comment) and `t30` (a thread on `docs/t1/a.md`) by alice, each
/// with its fix task (`fix1`, `fix2`); `fix1` merged at `commit(2)` (`fix2` owns the
/// same files, so it waits). Views are off, so only the replies and pushes go out.
fn fixed(fx: &mut Fixture) {
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Please update the README.")];
    v.threads = vec![on("docs/t1/a.md", vec![noted(30, "alice", "Rename this.")])];
    let (at, _) = poll_with(fx, v);
    fx.run_mut().delivery.watching = false;
    fx.send(at + 1, EventKind::Tick);
    merge_fix(fx, "fix1", &commit(2));
}

/// Fix task `id` launches, passes its gates and merges at `at`.
pub(super) fn merge_fix(fx: &mut Fixture, id: &str, at: &str) {
    fx.tick();
    let windows = fx.launch_all();
    to_queue(fx, id, window_of(&windows, id));
    merge(fx, id, at);
}

/// The pending host ops that are replies.
pub(super) fn reply_ops(fx: &Fixture) -> Vec<(OpId, HostOp)> {
    (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Reply { .. }))
        .collect()
}

/// The one pending reply.
pub(super) fn one_reply(fx: &Fixture) -> (OpId, HostOp) {
    let ops = reply_ops(fx);
    assert_eq!(ops.len(), 1, "one reply: {:#?}", host_ops(fx));
    ops[0].clone()
}

fn marker_of(op: &HostOp) -> String {
    match op {
        HostOp::Reply { marker, .. } => marker.clone(),
        other => panic!("not a reply: {other:?}"),
    }
}

/// Answers the pending push of stage 1, which must push `sha`.
pub(super) fn push_lands(fx: &mut Fixture, sha: &str) {
    let ops: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Push { .. }))
        .collect();
    let want = HostOp::Push {
        stage: 1,
        sha: sha.into(),
    };
    assert_eq!(
        ops.iter().map(|(_, o)| o.clone()).collect::<Vec<_>>(),
        vec![want]
    );
    assert!(reply_ops(fx).is_empty(), "no reply before the push lands");
    answer(fx, ops[0].0, HostResult::Pushed(PushOutcome::Pushed));
}

#[test]
fn reply_after_the_push_is_exact_and_marks_the_thread_replied() {
    let mut fx = by_alice();
    fixed(&mut fx);
    push_lands(&mut fx, &commit(2));
    let (op, reply) = one_reply(&fx);
    assert_eq!(
        reply,
        HostOp::Reply {
            stage: 1,
            number: PR,
            thread: "7:c5".into(),
            target: ReplyTarget::Conversation,
            body: "@alice Addressed in 2eeeeee by task fix1.".into(),
            marker: format!("<!-- anthrex:reply {RUN_ID} 7:c5 2eeeeee -->"),
        }
    );
    answer(&mut fx, op, HostResult::Replied { comment_id: 901 });
    assert_eq!(state(&fx, "c5"), ThreadState::Replied { comment_id: 901 });
    assert!(logged(&fx, "stage 1 (PR #7): replied on 7:c5"));
    fx.tick();
    assert!(reply_ops(&fx).is_empty(), "fix2 has not merged");
    merge_fix(&mut fx, "fix2", &commit(3));
    push_lands(&mut fx, &commit(3));
    let (op, reply) = one_reply(&fx);
    let marker = format!("<!-- anthrex:reply {RUN_ID} 7:t30 3eeeeee -->");
    assert_eq!(
        reply,
        HostOp::Reply {
            stage: 1,
            number: PR,
            thread: "7:t30".into(),
            target: ReplyTarget::Thread { comment_id: 30 },
            body: "Addressed in 3eeeeee by task fix2.".into(),
            marker: marker.clone(),
        }
    );
    answer(&mut fx, op, HostResult::Replied { comment_id: 902 });
    assert_eq!(state(&fx, "t30"), ThreadState::Replied { comment_id: 902 });
    fx.tick();
    assert!(reply_ops(&fx).is_empty(), "one reply per thread");
    // anthrex's replies come back in the next view: its own, by id, never review input.
    fx.run_mut().delivery.watching = true;
    let mut v = view(&commit(3));
    v.comments = vec![
        said(5, "alice", "Please update the README."),
        said(901, "tester", "@alice Addressed in 2eeeeee by task fix1."),
    ];
    let mut t = on("docs/t1/a.md", vec![noted(30, "alice", "Rename this.")]);
    t.comments
        .push(noted(902, "tester", &format!("Addressed.\n\n{marker}")));
    v.threads = vec![t];
    poll_with(&mut fx, v);
    assert_eq!(
        state(&fx, "c901"),
        ThreadState::Ignored {
            reason: "anthrex's own".into()
        }
    );
    assert_eq!(state(&fx, "t30"), ThreadState::Replied { comment_id: 902 });
    assert!(asked(&fx).is_empty());
    assert!(fx.run().delivery.stage(1).unwrap().batch.is_none());
}

#[test]
fn reply_to_comments_false_sends_no_reply() {
    let mut fx = by_alice();
    fx.run_mut().delivery.limits.reply_to_comments = false;
    fixed(&mut fx);
    push_lands(&mut fx, &commit(2));
    merge_fix(&mut fx, "fix2", &commit(3));
    push_lands(&mut fx, &commit(3));
    fx.tick();
    assert!(reply_ops(&fx).is_empty());
    assert_eq!(
        state(&fx, "c5"),
        ThreadState::Tasked {
            task: "fix1".into()
        }
    );
    assert_eq!(
        state(&fx, "t30"),
        ThreadState::Tasked {
            task: "fix2".into()
        }
    );
}

#[test]
fn a_reply_whose_answer_was_lost_is_sent_again_and_is_never_review_input() {
    let mut fx = by_alice();
    fixed(&mut fx);
    push_lands(&mut fx, &commit(2));
    let (_, first) = one_reply(&fx);
    // GitHub posted it (id 901), but the daemon restarted before the answer.
    let json = serde_json::to_string(fx.run()).unwrap();
    *fx.run_mut() = serde_json::from_str(&json).unwrap();
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(replies(&resume(&mut fx))[0].is_ok());
    let (op, again) = one_reply(&fx);
    assert_eq!(again, first, "the same reply, with the same marker");
    // It times out this time, and a view shows the comment before it is answered.
    answer(
        &mut fx,
        op,
        HostResult::Error(HostError::TimedOut("gh api".into())),
    );
    fx.run_mut().delivery.watching = true;
    let mut v = view(&commit(2));
    let posted = format!(
        "@alice Addressed in 2eeeeee by task fix1.\n\n{}",
        marker_of(&first)
    );
    v.comments = vec![said(901, "tester", &posted)];
    poll_with(&mut fx, v);
    assert_eq!(
        state(&fx, "c901"),
        ThreadState::Ignored {
            reason: "anthrex's own".into()
        }
    );
    assert!(
        fx.run()
            .delivery
            .stage(1)
            .unwrap()
            .own_comments
            .contains(&901)
    );
    assert!(asked(&fx).is_empty() && fx.run().delivery.stage(1).unwrap().batch.is_none());
    // Sent again when due, it finds its post by the marker (decision 10).
    let due = fx.run().delivery.stage(1).unwrap().retry_at.unwrap();
    fx.send(due, EventKind::Tick);
    let (op, third) = one_reply(&fx);
    assert_eq!(third, first);
    answer(&mut fx, op, HostResult::Replied { comment_id: 901 });
    assert_eq!(state(&fx, "c5"), ThreadState::Replied { comment_id: 901 });
}

/// `reviewed()` with `c5` by alice, counted and still `new`.
pub(super) fn commented() -> Fixture {
    let mut fx = by_alice();
    fx.run_mut().delivery.limits.review_batch_secs = 3_600;
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Why this name?")];
    poll_with(&mut fx, v);
    fx
}

pub(super) fn reply_comment(thread: &str, body: &str) -> PlanEdit {
    PlanEdit::ReplyComment {
        pr: PR,
        thread: thread.into(),
        body: body.into(),
    }
}

pub(super) fn refused(fx: &mut Fixture, edits: Vec<PlanEdit>) -> String {
    let result = replies(&edit(fx, edits)).remove(0);
    result.expect_err("refused")
}

#[test]
fn reply_comment_is_alone_in_its_call_and_validated() {
    let mut fx = commented();
    let ok = reply_comment("c5", "Because it is public.");
    let cancel = PlanEdit::CancelTask {
        task_id: "t1".into(),
    };
    let one = "message, refresh and reply_comment must be the only edit in their call";
    assert_eq!(refused(&mut fx, vec![ok.clone(), cancel]), one);
    let mixed = refused(&mut fx, vec![ok.clone(), ok.clone()]);
    assert_eq!(mixed, one);
    let other_pr = PlanEdit::ReplyComment {
        pr: 99,
        thread: "c5".into(),
        body: "x".into(),
    };
    let text = format!("pr #99 is not an open stage PR of run {RUN_ID}");
    assert_eq!(refused(&mut fx, vec![other_pr]), text);
    assert_eq!(
        refused(&mut fx, vec![reply_comment("c4", "x")]),
        "unknown thread 7:c4"
    );
    let length = "reply_comment: body must be 1 to 4000 characters";
    assert_eq!(refused(&mut fx, vec![reply_comment("c5", "")]), length);
    let long = "x".repeat(4_001);
    assert_eq!(refused(&mut fx, vec![reply_comment("c5", &long)]), length);
    fx.run_mut().delivery.limits.reply_to_comments = false;
    let off = "replies are turned off ([delivery] reply_to_comments = false)";
    assert_eq!(refused(&mut fx, vec![ok.clone()]), off);
    fx.run_mut().delivery.limits.reply_to_comments = true;
    // A sub-planner may not reply; a closed PR takes none.
    let mut run = fx.run().clone();
    let planner = EditSource::Planner { epic: "e".into() };
    let edit = (PR, "c5", "x");
    let error = crate::run::delivery::reply_edit::apply(&mut run, edit, &planner, 0).unwrap_err();
    assert_eq!(error.message, "a sub-planner cannot reply to a comment");
    fx.run_mut().delivery.stages[0].pr.as_mut().unwrap().state = proto::PrState::Closed;
    let text = format!("pr #7 is not an open stage PR of run {RUN_ID}");
    assert_eq!(refused(&mut fx, vec![ok]), text);
    assert!(reply_ops(&fx).is_empty());
    assert_eq!(state(&fx, "c5"), ThreadState::New);
    // The orchestrator's call is refused before any effect too.
    super::bisect::with_orchestrator(&mut fx);
    fx.run_mut().delivery.stages[0].pr.as_mut().unwrap().state = proto::PrState::Open;
    let call = json!({"edits": [{"op": "reply_comment", "pr": PR, "thread": "c5", "body": "x"}],
        "summary": "s"});
    let (ok, value) = orch_answer(&edit_plan(&mut fx, call));
    assert!(!ok);
    assert_eq!(value, json!({"error": one}));
}

#[test]
fn reply_comment_queues_a_reply_with_the_marker() {
    let mut fx = commented();
    let body = "Thanks @bob, see <!-- anthrex:reply x 7:t1 abc -->\u{202E}\r\ndone";
    let ok = replies(&edit(&mut fx, vec![reply_comment("7:c5", body)])).remove(0);
    assert!(ok.is_ok(), "{ok:?}");
    assert_eq!(state(&fx, "c5"), ThreadState::Replied { comment_id: 0 });
    let (op, reply) = one_reply(&fx);
    let HostOp::Reply {
        thread,
        target,
        body,
        marker,
        ..
    } = reply
    else {
        unreachable!()
    };
    assert_eq!(
        (thread.as_str(), target),
        ("7:c5", ReplyTarget::Conversation)
    );
    // No mention, no comment opener (no forged marker), no hidden character.
    assert_eq!(
        body,
        "Thanks \u{FF20}bob, see &lt;!-- anthrex:reply x 7:t1 abc -->\ndone"
    );
    let prefix = format!("<!-- anthrex:reply {RUN_ID} 7:c5 ");
    let token = marker
        .strip_prefix(&prefix)
        .and_then(|m| m.strip_suffix(" -->"));
    let hex = |t: &str| t.len() == 7 && t.bytes().all(|b| b.is_ascii_hexdigit());
    assert!(token.is_some_and(hex), "{marker}");
    answer(&mut fx, op, HostResult::Replied { comment_id: 77 });
    assert_eq!(state(&fx, "c5"), ThreadState::Replied { comment_id: 77 });
    assert!(
        fx.run()
            .delivery
            .stage(1)
            .unwrap()
            .own_comments
            .contains(&77)
    );
    let last = fx.run().plan_edits.last().unwrap();
    assert!(format!("{last:?}").contains("reply to 7:c5"), "{last:?}");
    // The batch it was in closes with nothing left to do.
    fx.run_mut().delivery.limits.review_batch_secs = 0;
    fx.tick();
    assert!(fx.run().tasks.iter().all(|t| !t.id().starts_with("fix")));
}

/// A planned run's batch of `c5`, `c6` and `c7` by alice, the orchestrator woken.
pub(super) fn planned() -> Fixture {
    let mut fx = by_alice();
    super::bisect::with_orchestrator(&mut fx);
    let mut v = view(&commit(1));
    v.comments = vec![
        said(5, "alice", "a"),
        said(6, "alice", "b"),
        said(7, "alice", "c"),
    ];
    let (at, _) = poll_with(&mut fx, v);
    fx.send(at + 1, EventKind::Tick);
    fx
}

pub(super) fn add(id: &str, owns: &[&str], addresses: &[&str]) -> Value {
    json!({"op": "add_task", "task": {
        "id": id, "title": format!("Title {id}"), "size": "S", "stage": 1,
        "owns": owns, "test_mode": "check", "test_mode_reason": "docs",
        "brief": format!("Brief {id}"), "acceptance": [format!("Accept {id}")],
        "addresses": addresses
    }})
}

pub(super) fn add_plan(fx: &mut Fixture, task: Value) -> (bool, Value) {
    orch_answer(&edit_plan(fx, json!({"edits": [task]})))
}

#[test]
fn addresses_sets_origin_review_and_is_validated() {
    let mut fx = planned();
    let (ok, value) = add_plan(&mut fx, add("rev1", &["docs/t1/**"], &["7:c5"]));
    assert!(ok, "{value}");
    assert_eq!(value["held"], Value::Null);
    let t = fx.run().task("rev1").unwrap();
    assert_eq!(t.origin, proto::TaskOrigin::Review);
    assert_eq!(
        t.fixes,
        Some(FixOf::Review {
            stage: 1,
            pr: PR,
            threads: vec!["7:c5".into()]
        })
    );
    assert_eq!(
        state(&fx, "c5"),
        ThreadState::Tasked {
            task: "rev1".into()
        }
    );
    assert_eq!(fx.run().delivery.stage(1).unwrap().review_rounds, 1);
    // Every ref must be a new thread of the task's stage's open PR.
    for bad in ["7:c5", "7:c999", "8:c6", "c6"] {
        let (ok, value) = add_plan(&mut fx, add("rev2", &["docs/t1/**"], &[bad]));
        assert!(!ok);
        let text = format!("task rev2: addresses {bad}, which is not a new thread of stage 1's PR");
        assert_eq!(value["errors"][0]["message"], text, "{bad}");
    }
    // A review fix outside its stage waits for the user (decision 26).
    let (ok, value) = add_plan(&mut fx, add("rev3", &["Cargo.toml"], &["7:c6"]));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "hold-rev3");
    let line = format!(
        "fix task rev3 for stage 1 needs approval: it owns Cargo.toml, outside the stage; anthrex run approve {RUN_ID} --hold hold-rev3"
    );
    assert!(attention(&fx).contains(&line), "{:#?}", attention(&fx));
    assert_eq!(
        fx.run().delivery.stage(1).unwrap().review_rounds,
        1,
        "one round"
    );
    // No task joins a stage whose PR is merged or closed.
    for (state, word) in [
        (proto::PrState::Merged, "merged"),
        (proto::PrState::Closed, "closed"),
    ] {
        fx.run_mut().delivery.stages[0].pr.as_mut().unwrap().state = state;
        let (ok, value) = add_plan(&mut fx, add("rev4", &["docs/t1/**"], &[]));
        assert!(!ok);
        let text = format!("stage 1's PR is {word}; add the task to a later stage");
        assert_eq!(value["errors"][0]["message"], text);
    }
}

#[test]
fn a_reply_to_a_thread_that_is_gone_is_dropped() {
    let mut fx = commented();
    assert!(replies(&edit(&mut fx, vec![reply_comment("c5", "Why not?")]))[0].is_ok());
    let (op, _) = one_reply(&fx);
    let gone = HostResult::Error(HostError::NotFound("gh: Not Found (HTTP 404)".into()));
    answer(&mut fx, op, gone);
    assert!(fx.run().delivery.stage(1).unwrap().replies.is_empty());
    assert!(logged(
        &fx,
        "stage 1: the reply on thread c5 was dropped: not found"
    ));
    let due = fx
        .run()
        .delivery
        .stage(1)
        .unwrap()
        .retry_at
        .unwrap_or(fx.now);
    fx.send(due + 1, EventKind::Tick);
    assert!(reply_ops(&fx).is_empty(), "not sent again");
}

#[test]
fn an_adopted_remote_branch_carries_the_fix_and_its_reply_goes() {
    let mut fx = by_alice();
    fixed(&mut fx);
    // fix1's push fails; meanwhile the user pushes a commit on top of the stage branch.
    let (op, _) = (host_ops(&fx).into_iter())
        .find(|(_, o)| matches!(o, HostOp::Push { .. }))
        .expect("a push");
    answer(
        &mut fx,
        op,
        HostResult::Error(HostError::Failed("network".into())),
    );
    fx.run_mut().delivery.watching = true;
    poll_with(&mut fx, view(&commit(41)));
    assert!(
        reply_ops(&fx).is_empty(),
        "not before the fix is on the remote"
    );
    let due = fx.run().delivery.stage(1).unwrap().retry_at.unwrap();
    fx.send(due, EventKind::Tick);
    let (op, fetch) = host_ops(&fx).remove(0);
    assert!(matches!(fetch, HostOp::Fetch { .. }), "{fetch:?}");
    let adopted = crate::host::FetchOutcome::Adopted { sha: commit(41) };
    answer(&mut fx, op, HostResult::Fetched(adopted));
    // The remote branch descends from the stage head, which holds fix1's merge.
    let (_, reply) = one_reply(&fx);
    assert!(marker_of(&reply).ends_with("7:c5 2eeeeee -->"), "{reply:?}");
}
