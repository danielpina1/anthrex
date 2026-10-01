//! Milestone 9.0.6 task 6: every run request's precondition is one rule
//! (`engine/actions/rules.rs`), whose text is the reply each handler gave before the
//! rules existed (each test cites the handler lines at `e8a16ae` or the test that pins
//! the text), and every handler now refuses with exactly its rule's text.

use proto::{FinishAction, MessageKind, MessageTarget, PlanEdit, RunPath, RunState, TaskState};

use super::actions_twins::hold_stage;
use super::dispatch::{edit, replies};
use super::fixture::*;
use crate::run::engine::actions::rules;
use crate::run::engine::{Effect, EventKind, OrchEvent, orch_window};
use crate::run::orch::contract::ONE_EDIT_RULE;
use crate::run::validate::EditScope;

/// Two tasks, `t2` after `t1`, at the plan gate.
fn gate() -> Fixture {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.start(false);
    fx
}

/// The same run approved by `--yes`, its integration worktree made.
pub(super) fn running() -> Fixture {
    let plan = plan_with(
        PROFILE,
        &[
            task("t1", "S", "a", ""),
            task("t2", "S", "b", "deps = [\"t1\"]"),
        ],
    );
    let mut fx = Fixture::new(&plan);
    fx.ready(true);
    fx
}

fn some(text: impl Into<String>) -> Option<String> {
    Some(text.into())
}

/// The one reply of `event`.
fn reply_of(
    fx: &mut Fixture,
    event: impl FnOnce(crate::run::engine::ReplyId) -> EventKind,
) -> Vec<Result<String, String>> {
    let reply = fx.reply();
    replies(&fx.next(event(reply)))
}

fn edit_submit(fx: &mut Fixture, edits: Vec<PlanEdit>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits,
        scope: EditScope::Run,
        refusals: Vec::new(),
        submit: true,
    })
}

#[test]
fn approve_reads_as_before() {
    // `requests::approve` (`requests.rs:156-171`); `orch.rs::approve_while_planning_is_refused`.
    let mut fx = gate();
    assert_eq!(rules::approve(fx.run()), None);
    fx.run_mut().state = RunState::Planning;
    assert_eq!(
        rules::approve(fx.run()),
        some(format!(
            "run {RUN_ID} is still being planned; approve it when the orchestrator has submitted the plan"
        ))
    );
    let fx = running();
    assert_eq!(
        rules::approve(fx.run()),
        some(format!("run {RUN_ID} is running"))
    );
    // `dispatch.rs`'s discard test: a run with its `Discard` op in flight.
    let mut fx = gate();
    reply_of(&mut fx, |reply| EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        rules::approve(fx.run()),
        some(format!("run {RUN_ID} is being discarded"))
    );
}

#[test]
fn reject_reads_as_before() {
    // `requests::reject` (`requests.rs:201-213`).
    let mut fx = gate();
    assert_eq!(rules::reject(fx.run()), None);
    fx.run_mut().state = RunState::Paused;
    fx.run_mut().paused_from = Some(RunState::Planning);
    assert_eq!(rules::reject(fx.run()), None, "paused while planning");
    let fx = running();
    assert_eq!(
        rules::reject(fx.run()),
        some(format!(
            "run {RUN_ID} is running; reject applies only while its plan awaits approval"
        ))
    );
}

#[test]
fn ordering_cases_read_as_before() {
    // `requests::reject` asks `finishing_as` first: a rejected run's `Discard` in flight.
    let mut fx = gate();
    reply_of(&mut fx, |reply| EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        rules::reject(fx.run()),
        some(format!("run {RUN_ID} is being discarded"))
    );
    // `restore::resume`: a run at its gate (or planning) whose orchestrator is dormant
    // restarts it (milestone 9 decision 11).
    let mut fx = super::orch::launched(false);
    orch_window::restored(fx.run_mut());
    assert_eq!(rules::resume(fx.run(), false), None, "planning");
    fx.run_mut().state = RunState::AwaitingApproval;
    assert_eq!(rules::resume(fx.run(), false), None, "at the gate");
    // A running run with a held stage retries tier 3 (ruling C-18).
    let mut fx = running();
    hold_stage(fx.run_mut());
    assert_eq!(rules::resume(fx.run(), false), None);
    let effects = reply_of(&mut fx, |reply| EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    assert_eq!(effects, vec![Ok(format!("run {RUN_ID}: tier 3 retries"))]);
}

#[test]
fn submit_reads_as_before() {
    // `requests::submit_edit` (`requests.rs:307-319`); `orch_edit.rs:311`.
    let mut fx = gate();
    assert_eq!(
        rules::submit(fx.run()),
        some(format!(
            "run {RUN_ID} is awaiting_approval; only a run being planned can be submitted"
        ))
    );
    fx.run_mut().state = RunState::Planning;
    assert_eq!(rules::submit(fx.run()), None);
    assert!(!rules::promoted_unsubmitted(running().run()));
}

#[test]
fn edit_run_reads_as_before() {
    // `requests::edit` (`requests.rs:254-279`); `fast_path.rs:302`, `cancel_work.rs:27`.
    let refresh = PlanEdit::Refresh {
        task_id: "t1".into(),
    };
    let fx = running();
    assert_eq!(
        rules::edit_run(fx.run(), &[refresh.clone(), PlanEdit::Pause], false),
        some(ONE_EDIT_RULE)
    );
    assert_eq!(
        rules::edit_run(fx.run(), &[refresh], true),
        some(ONE_EDIT_RULE)
    );
    let split = PlanEdit::SplitTask {
        task_id: "t1".into(),
        into: Vec::new(),
    };
    let mut fx = running();
    fx.run_mut().path = Some(RunPath::Fast);
    assert_eq!(
        rules::edit_run(fx.run(), std::slice::from_ref(&split), false),
        some(format!(
            "run {RUN_ID} is on the fast path: it runs one task; start a planned run instead"
        ))
    );
    let mut fx = running();
    fx.run_mut().cancelled = true;
    let cancelled = some(format!("run {RUN_ID} was cancelled"));
    assert_eq!(rules::edit_run(fx.run(), &[split], false), cancelled);
    assert_eq!(rules::edit_run(fx.run(), &[], true), cancelled);
    assert_eq!(rules::edit_run(fx.run(), &[PlanEdit::Pause], false), None);
    for state in [RunState::Complete, RunState::Failed] {
        fx.run_mut().state = state;
        assert_eq!(
            rules::edit_run(fx.run(), &[PlanEdit::Pause], false),
            some(format!("run {RUN_ID} is {}", state.label()))
        );
    }
}

#[test]
fn pause_and_unpause_read_as_before() {
    // `batch::pause_or_resume_fits` (`batch.rs:210-232`); `control.rs:247`, `:295`.
    let fx = running();
    assert_eq!(rules::pause(fx.run()), None);
    assert_eq!(
        rules::unpause(fx.run()),
        some(format!(
            "run {RUN_ID} is running; only a paused run can be resumed"
        ))
    );
    assert_eq!(
        rules::pause_in(RUN_ID, RunState::Paused),
        some(format!(
            "run {RUN_ID} is paused; only a running run can be paused"
        ))
    );
    assert_eq!(rules::unpause_in(RUN_ID, RunState::Paused), None);
}

#[test]
fn a_batch_checks_each_pause_against_the_state_before_it() {
    // Preflight F4: `[pause, pause]` is refused on its second edit's simulated state.
    let mut fx = running();
    let effects = edit(&mut fx, vec![PlanEdit::Pause, PlanEdit::Pause]);
    assert_eq!(
        replies(&effects),
        vec![Err(rules::pause_in(RUN_ID, RunState::Paused).unwrap())]
    );
    assert_eq!(
        fx.run().state,
        RunState::Running,
        "a refused batch changes nothing"
    );
}

#[test]
fn resume_reads_as_before() {
    // `merge::resume` (`merge.rs:549-566`) behind `restore::resume`.
    let mut fx = running();
    assert_eq!(
        rules::resume(fx.run(), false),
        some(format!("run {RUN_ID} is running"))
    );
    fx.run_mut().state = RunState::Paused;
    assert_eq!(rules::resume(fx.run(), false), None);
    fx.run_mut().state = RunState::Halted;
    fx.run_mut().halted_reason = Some("main moved".into());
    assert_eq!(
        rules::resume(fx.run(), false),
        some(format!(
            "run {RUN_ID} is halted: main moved; check the refs, then resume with --rebaseline"
        ))
    );
    assert_eq!(rules::resume(fx.run(), true), None);
    fx.run_mut().halt_retryable = true;
    assert_eq!(rules::resume(fx.run(), false), None);
    // A run at its gate with no orchestrator has nothing to restart.
    assert_eq!(
        rules::resume(gate().run(), false),
        some(format!("run {RUN_ID} is awaiting_approval"))
    );
}

#[test]
fn cancel_reads_as_before() {
    // `complete::cancel` (`complete.rs:294-304`).
    assert_eq!(
        rules::cancel(gate().run()),
        some(format!("run {RUN_ID} is awaiting_approval"))
    );
    let mut fx = running();
    for state in [RunState::Running, RunState::Paused, RunState::Halted] {
        fx.run_mut().state = state;
        assert_eq!(rules::cancel(fx.run()), None, "{state:?}");
    }
}

#[test]
fn promote_reads_as_before() {
    // `promote::promote` (`promote.rs:43-50`); `fast_path.rs:181`.
    let mut fx = running();
    assert_eq!(
        rules::promote(fx.run()),
        some(format!("run {RUN_ID} is not a fast-path run"))
    );
    fx.run_mut().path = Some(RunPath::Fast);
    assert_eq!(rules::promote(fx.run()), None);
    fx.run_mut().state = RunState::Complete;
    assert_eq!(
        rules::promote(fx.run()),
        some(format!("run {RUN_ID} is complete"))
    );
}

#[test]
fn finish_reads_as_before() {
    // `complete::finish` (`complete.rs:363-380`); `control.rs:335`.
    let mut fx = running();
    for (action, verb) in [
        (FinishAction::Accept, "accept"),
        (FinishAction::Discard, "discard"),
    ] {
        assert_eq!(
            rules::finish(fx.run(), action),
            some(format!(
                "run {RUN_ID} is running; {verb} applies only to a complete run"
            ))
        );
    }
    fx.run_mut().state = RunState::Complete;
    assert_eq!(rules::finish(fx.run(), FinishAction::Accept), None);
}

#[test]
fn hold_reads_as_before() {
    // `gate_holds::decide` (`gate_holds.rs:479-490`); `gate_holds.rs::hold_verdict_errors`.
    let mut fx = running();
    assert_eq!(
        rules::hold(fx.run(), "epic:nope"),
        some(format!("run {RUN_ID} has no hold epic:nope"))
    );
    fx.run_mut().state = RunState::Failed;
    assert_eq!(
        rules::hold(fx.run(), "epic:nope"),
        some(format!("run {RUN_ID} is failed"))
    );
}

#[test]
fn message_and_message_stage_read_as_before() {
    // `edits_orch::apply_message` (`edits_orch.rs:207-229`);
    // `worker_messages.rs::message_whose_every_recipient_is_refused_rejects_the_call`,
    // `worker_messages_stage.rs:67`.
    let mut fx = running();
    assert_eq!(
        rules::message(fx.run(), "t1"),
        None,
        "a queued task records it"
    );
    fx.task_mut("t1").state = TaskState::Review;
    assert_eq!(
        rules::message(fx.run(), "t1"),
        some(
            "message: no recipient can take it: task t1 is review; a message would not reach a worker"
        )
    );
    assert_eq!(rules::message_stage(fx.run(), 1), None, "t2 records it");
    assert_eq!(
        rules::message_stage(fx.run(), 2),
        some("stage 2 has no task")
    );
}

#[test]
fn answer_and_cancel_task_read_as_before() {
    // `Batch::answer` and `Batch::cancel` (`edits.rs:235-240`, `:557-567`);
    // `edits_tests_rules.rs:101`, `edits_tests.rs:340`.
    let mut fx = running();
    assert_eq!(
        rules::answer(fx.run(), "t2"),
        some("task t2 is pending; only blocked(question) or working tasks can be answered")
    );
    assert_eq!(
        rules::answer(fx.run(), "t9"),
        some("task t9: task_id: no such task")
    );
    assert_eq!(rules::cancel_task(fx.run(), "t2"), None);
    fx.task_mut("t2").state = TaskState::Cancelled;
    assert_eq!(
        rules::cancel_task(fx.run(), "t2"),
        some("task t2 is cancelled; only unfinished tasks can be cancelled")
    );
}

#[test]
fn refresh_reads_as_before() {
    // `edits_orch::apply_refresh` (`edits_orch.rs:300-330`); `refresh.rs:171`.
    let fx = running();
    assert_eq!(
        rules::refresh(fx.run(), "t2"),
        some("task t2 is pending; refresh needs a working or paused task")
    );
    assert_eq!(
        rules::refresh(fx.run(), "t9"),
        some("task t9: task_id: no such task")
    );
}

#[test]
fn retry_reads_as_before() {
    // `requests::retry` (`requests.rs:378-409`); `control_retry.rs:112-176`.
    assert_eq!(
        rules::retry(gate().run(), "t1"),
        some(format!("run {RUN_ID} is awaiting_approval"))
    );
    let mut fx = running();
    fx.launch_all();
    assert_eq!(
        rules::retry(fx.run(), "t1"),
        some("task t1 is working; retry applies only to a blocked task")
    );
    assert_eq!(rules::retry(fx.run(), "t9"), some("unknown task t9"));
}

#[test]
fn override_task_reads_as_before() {
    // `gates::override_task` (`gates.rs:348-382`) and `override_refusal`;
    // `control_retry.rs:271`, `gates_fixes.rs:309`.
    assert_eq!(
        rules::override_task(gate().run(), "t1"),
        some(format!("run {RUN_ID} is awaiting_approval"))
    );
    let mut fx = running();
    fx.launch_all();
    let applies = "override applies only to a task in review, or blocked with commits";
    assert_eq!(
        rules::override_task(fx.run(), "t1"),
        some(format!("task t1 is working; {applies}"))
    );
    assert_eq!(
        rules::override_task(fx.run(), "t9"),
        some("unknown task t9")
    );
    fx.task_mut("t2").awaiting_deps = true;
    assert_eq!(
        rules::override_task(fx.run(), "t2"),
        some("task t2 waits for its dependencies; override it once they are merged")
    );
    fx.task_mut("t1").state = TaskState::Blocked;
    fx.task_mut("t1").start_commit = None;
    assert_eq!(
        rules::override_task(fx.run(), "t1"),
        some(format!("task t1 has no commits; {applies}"))
    );
    fx.task_mut("t1").state = TaskState::Review;
    assert_eq!(rules::override_task(fx.run(), "t1"), None);
}

#[test]
fn handlers_refuse_through_the_rules() {
    // Each handler's refusal is now exactly its rule's: one representative state each.
    let run_id = || RUN_ID.to_string();
    let mut fx = running();
    let expected = rules::approve(fx.run()).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Approve {
        reply,
        run_id: run_id(),
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::reject(fx.run()).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Reject {
        reply,
        run_id: run_id(),
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::finish(fx.run(), FinishAction::Discard).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Finish {
        reply,
        run_id: run_id(),
        action: FinishAction::Discard,
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::promote(fx.run()).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Promote {
        reply,
        run_id: run_id(),
        orchestrator: None,
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::resume(fx.run(), false).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Resume {
        reply,
        run_id: run_id(),
        rebaseline: None,
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::hold(fx.run(), "epic:nope").unwrap();
    let got = reply_of(&mut fx, |reply| {
        EventKind::Orch(OrchEvent::RejectHold {
            reply,
            run_id: run_id(),
            hold: "epic:nope".into(),
        })
    });
    assert_eq!(got, vec![Err(expected)]);
    fx.launch_all();
    let expected = rules::retry(fx.run(), "t1").unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Retry {
        reply,
        run_id: run_id(),
        task_id: "t1".into(),
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::override_task(fx.run(), "t1").unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Override {
        reply,
        run_id: run_id(),
        task_id: "t1".into(),
        reason: "r".into(),
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::unpause(fx.run()).unwrap();
    assert_eq!(
        replies(&edit(&mut fx, vec![PlanEdit::Resume])),
        vec![Err(expected)]
    );
    let expected = rules::answer(fx.run(), "t2").unwrap();
    let answer = PlanEdit::Answer {
        task_id: "t2".into(),
        text: "a".into(),
    };
    assert_eq!(replies(&edit(&mut fx, vec![answer])), vec![Err(expected)]);
    let expected = rules::refresh(fx.run(), "t2").unwrap();
    let refresh = PlanEdit::Refresh {
        task_id: "t2".into(),
    };
    assert_eq!(replies(&edit(&mut fx, vec![refresh])), vec![Err(expected)]);
    let expected = rules::submit(fx.run()).unwrap();
    assert_eq!(replies(&edit_submit(&mut fx, vec![])), vec![Err(expected)]);
    fx.task_mut("t2").state = TaskState::Review;
    let expected = rules::message(fx.run(), "t2").unwrap();
    let message = PlanEdit::Message {
        to: MessageTarget::Tasks(vec!["t2".into()]),
        text: "m".into(),
        kind: MessageKind::Info,
    };
    assert_eq!(replies(&edit(&mut fx, vec![message])), vec![Err(expected)]);
    fx.task_mut("t2").state = TaskState::Cancelled;
    let expected = rules::cancel_task(fx.run(), "t2").unwrap();
    let cancel = PlanEdit::CancelTask {
        task_id: "t2".into(),
    };
    assert_eq!(replies(&edit(&mut fx, vec![cancel])), vec![Err(expected)]);
    let mut fx = gate();
    let expected = rules::cancel(fx.run()).unwrap();
    let got = reply_of(&mut fx, |reply| EventKind::Cancel {
        reply,
        run_id: run_id(),
    });
    assert_eq!(got, vec![Err(expected)]);
    let expected = rules::pause(fx.run()).unwrap();
    assert_eq!(
        replies(&edit(&mut fx, vec![PlanEdit::Pause])),
        vec![Err(expected)]
    );
}
