//! M8a.15: plan edits during a run (decision 13, engine side), `answer`, the `pause`
//! edit and a paused run's requests (decision 45), and `Stop` (decision 46). Retry and
//! override are in `control_retry.rs`, restore and resume in `control_restore.rs`.
//! Every sequence ends with the liveness check, which a paused run passes when its
//! resume would leave nothing stuck.

use proto::{AgentRole, BlockReason, FinishAction, PlanEdit, RunState, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies, task_path};
use super::done::{killed, one_reply};
use super::fixture::*;
use super::gates::only_op;
use super::holds::{add_dep, answer, delivers};
use super::liveness::assert_alive;
use super::merge::{config, doc_task};
use super::turns::{exited, killed_exit, working, working_on};
use crate::run::contract::{DONE_ACCEPTED, amend_message, answer_message};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};

pub(super) fn retry(fx: &mut Fixture, task: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Retry {
        reply,
        run_id: RUN_ID.into(),
        task_id: task.into(),
    })
}

pub(super) fn override_task(fx: &mut Fixture, task: &str, reason: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: task.into(),
        reason: reason.into(),
    })
}

pub(super) fn resume(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    })
}

pub(super) fn blocked(fx: &mut Fixture, window: u32, kind: &str, reason: &str) {
    let effects = fx.tool(
        window,
        "task_blocked",
        json!({"kind": kind, "reason": reason}),
    );
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
}

fn amend_brief(task: &str, brief: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task.into(),
        brief: Some(brief.into()),
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
    }
}

fn applied(effects: &[Effect]) {
    assert_eq!(replies(effects), vec![Ok("applied 1 edit".to_string())]);
}

fn no_kill(effects: &[Effect]) -> bool {
    !effects.iter().any(|e| {
        matches!(
            e,
            Effect::KillWindow { .. } | Effect::RetireWindow { .. } | Effect::RemoveWindow { .. }
        )
    })
}

#[test]
fn edit_applies_and_delivers() {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    let windows = fx.launch_all();
    assert_eq!(windows.len(), 1, "t2 waits for t1");
    let window = windows[0].1;

    let effects = edit(&mut fx, vec![amend_brief("t1", "Use argon2.")]);
    applied(&effects);
    let text = amend_message(fx.task("t1"));
    assert!(text.contains("Use argon2."), "{text}");
    let queued: Vec<&str> = fx.run().outbox.iter().map(|m| m.text.as_str()).collect();
    assert_eq!(queued, vec![text.as_str()]);
    // The worker's turn is open: the amendment is its next turn.
    let effects = fx.turn_completed(window);
    assert_eq!(delivers(&effects), vec![text]);

    let cancel = PlanEdit::CancelTask {
        task_id: "t1".into(),
    };
    let effects = edit(&mut fx, vec![cancel.clone()]);
    applied(&effects);
    assert!(killed(&effects, window), "{effects:#?}");
    assert!(
        ops_in(&effects, "RemoveWorktree").is_empty(),
        "the session is live"
    );
    let t2 = fx.task("t2");
    assert_eq!(t2.state, TaskState::Blocked);
    assert_eq!(t2.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
    let effects = killed_exit(&mut fx, window);
    let removals = ops_in(&effects, "RemoveWorktree");
    assert_eq!(removals.len(), 1, "{effects:#?}");
    let OpKind::RemoveWorktree {
        path, salvage_ref, ..
    } = &removals[0].1
    else {
        unreachable!()
    };
    assert_eq!(path, &task_path("t1"));
    assert_eq!(salvage_ref, &format!("refs/anthrex/salvage/{RUN_ID}/t1/1"));

    // A rejected batch changes nothing and names every error.
    let before = fx.run().tasks.clone();
    let effects = edit(&mut fx, vec![add_dep("t2", "nope"), amend_brief("t9", "x")]);
    let error = one_reply(&effects).unwrap_err();
    assert!(error.lines().count() >= 2, "{error}");
    assert!(error.contains("nope") && error.contains("t9"), "{error}");
    assert_eq!(fx.run().tasks, before);
    assert_alive(&fx);
}

#[test]
fn answer_resumes_a_question() {
    // The session is still open: the answer is its next turn.
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    let effects = edit(&mut fx, vec![answer("users")]);
    applied(&effects);
    assert_eq!(
        (fx.task("t1").state, &fx.task("t1").block),
        (TaskState::Working, &None)
    );
    let delivered: Vec<(u32, String)> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver {
                window_id, text, ..
            } => Some((*window_id, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(delivered, vec![(window, answer_message("users"))]);
    assert_alive(&fx);

    // The session ended between turns: a resume carries the answer.
    let (mut fx, window) = working();
    blocked(&mut fx, window, "question", "which table?");
    fx.turn_completed(window);
    exited(&mut fx, window);
    assert!(fx.task("t1").rounds[0].ended);
    let effects = edit(&mut fx, vec![answer("users")]);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let (op, kind) = only_op(&effects, "ResumeSession");
    let OpKind::ResumeSession {
        window_id, message, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!((window_id, message), (window, answer_message("users")));

    // That resume fails: a fresh session's prompt ends with the answer.
    let effects = fx.done(
        op,
        OpResult::ResumeFailed {
            error: "no such session".into(),
        },
    );
    let (op, _) = only_op(&effects, "DiffSoFar");
    let effects = fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    assert!(
        first_turn.ends_with(&answer_message("users")),
        "{first_turn}"
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.session, t1.rung, t1.failures), (2, 0, 0));
    assert_alive(&fx);
}

#[test]
fn pause_edit_stops_dispatch_and_gates_but_keeps_windows() {
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[doc_task("t1", ""), doc_task("t2", "")],
    );
    let mut fx = Fixture::with_config(&plan, config());
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    let pause = |fx: &mut Fixture| {
        let effects = edit(fx, vec![PlanEdit::Pause]);
        applied(&effects);
        assert_eq!(fx.run().state, RunState::Paused);
        assert_eq!(fx.run().paused_from, Some(RunState::Running));
        assert_alive(fx);
    };
    let unpause = |fx: &mut Fixture| {
        let effects = edit(fx, vec![PlanEdit::Resume]);
        applied(&effects);
        assert_eq!(
            (fx.run().state, fx.run().paused_from),
            (RunState::Running, None)
        );
        effects
    };

    // Deliveries wait.
    pause(&mut fx);
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(
        one_reply(&effects),
        Err(format!(
            "run {RUN_ID} is paused; only a running run can be paused"
        ))
    );
    edit(&mut fx, vec![amend_brief("t1", "Say more.")]);
    let effects = fx.turn_completed(window);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let effects = unpause(&mut fx);
    assert_eq!(delivers(&effects).len(), 1, "{effects:#?}");
    assert!(
        ops_in(&effects, "ResumeSession").is_empty(),
        "nothing was restored"
    );

    // Gates wait: a claim accepted while paused starts no check.
    let args = json!({"summary": "s"});
    let effects = fx.tool_as(AgentRole::Worker, window, "t1", "task_done", args);
    let (verify, _) = only_op(&effects, "VerifyDone");
    pause(&mut fx);
    let result = fx.clean_check("t1");
    let effects = fx.done(verify, result);
    assert_eq!(replies(&effects), vec![Ok(DONE_ACCEPTED.to_string())]);
    assert_eq!(fx.task("t1").state, TaskState::Check);
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");
    fx.turn_completed(window);
    let effects = unpause(&mut fx);
    let (check, _) = only_op(&effects, "Check");

    // Dispatch waits: t1 leaves its writer slot while paused, and t2 stays queued.
    pause(&mut fx);
    let effects = fx.done(check, super::gates::check_result(true));
    assert_eq!(fx.task("t1").state, TaskState::MergeQueue);
    assert!(
        ops_in(&effects, "MergeCandidate").is_empty(),
        "{effects:#?}"
    );
    assert!(
        ops_in(&effects, "PrepareWorktree").is_empty(),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    let effects = unpause(&mut fx);
    only_op(&effects, "MergeCandidate");
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
    assert!(no_kill(&fx.log), "the windows were kept");
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    assert_eq!(
        one_reply(&effects),
        Err(format!(
            "run {RUN_ID} is running; only a paused run can be resumed"
        ))
    );
    assert_alive(&fx);
}

#[test]
fn a_paused_run_refuses_tools_and_most_requests() {
    let (mut fx, window) = working_on("");
    edit(&mut fx, vec![PlanEdit::Pause]);
    let paused = format!("run {RUN_ID} is paused; the user must resume it");
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    assert_eq!(one_reply(&effects), Err(paused.clone()));
    let effects = fx.tool_as(AgentRole::Reviewer, 9, "t1", "submit_review", json!({}));
    assert_eq!(one_reply(&effects), Err(paused));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Approve {
        reply,
        run_id: RUN_ID.into(),
    });
    assert_eq!(one_reply(&effects), Err(format!("run {RUN_ID} is paused")));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(one_reply(&effects).is_err());
    for (action, verb) in [
        (FinishAction::Accept, "accept"),
        (FinishAction::Discard, "discard"),
    ] {
        let reply = fx.reply();
        let effects = fx.next(EventKind::Finish {
            reply,
            run_id: RUN_ID.into(),
            action,
        });
        assert_eq!(
            one_reply(&effects),
            Err(format!(
                "run {RUN_ID} is paused; {verb} applies only to a complete run"
            ))
        );
    }
    assert_eq!(fx.run().state, RunState::Paused);
    // Edits apply while paused (the user pauses to change the plan).
    let priority = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: Some(5),
        size: None,
    };
    applied(&edit(&mut fx, vec![priority]));
    assert_alive(&fx);

    let effects = resume(&mut fx);
    assert_eq!(one_reply(&effects), Ok(format!("run {RUN_ID} resumed")));
    assert_eq!(
        (fx.run().state, fx.run().paused_from),
        (RunState::Running, None)
    );
    edit(&mut fx, vec![PlanEdit::Pause]);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(one_reply(&effects).is_ok(), "{effects:#?}");
    assert_eq!(
        fx.run().state,
        RunState::Running,
        "a cancelled run runs to complete"
    );
    assert_alive(&fx);
}

/// Decision 46: after `Stop` the engine ignores every event, so sessions killed at
/// shutdown are not failures and the last `run.json` is the one written at stop.
#[test]
fn stop_ignores_everything_after() {
    let (mut fx, window) = working();
    let effects = fx.next(EventKind::Stop);
    assert!(effects.is_empty(), "{effects:#?}");
    assert!(fx.state.stopped);
    let frozen = fx.state.clone();
    let events = vec![
        EventKind::Tick,
        EventKind::Signal {
            window_id: window,
            signal: AgentSignal::ProcessExited {
                code: None,
                killed_by_engine: false,
                pid: 7,
            },
        },
        EventKind::Cancel {
            reply: 90,
            run_id: RUN_ID.into(),
        },
        EventKind::Retry {
            reply: 91,
            run_id: RUN_ID.into(),
            task_id: "t1".into(),
        },
    ];
    for kind in events {
        let effects = fx.next(kind);
        assert!(effects.is_empty(), "{effects:#?}");
        assert_eq!(fx.state, frozen);
    }
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    assert!(effects.is_empty());
    fx.now += 10_000;
    assert!(fx.tick().is_empty(), "no stall after stop");
    assert_eq!(fx.state, frozen);
}
