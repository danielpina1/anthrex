//! Milestone 9.0.6 §4.5: a run driven end to end through the TUI's own requests. Every
//! request is built by the TUI's pure functions (`GoalForm::request`, `request_for`,
//! `moved_base_confirm`) from the snapshot the test reads, and each action is asserted
//! listed and not refused before it is sent, as the action menu only sends what the
//! daemon listed.

use proto::{
    ActionInfo, ActionKind, MessageKind, RunInfo, RunReply, RunRequest, RunState, TaskState,
};
use serde_json::json;
use tui::actions_request::{ActionInput, ActionTarget, moved_base_confirm, request_for};

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_adapt::GOAL_WAIT;
use crate::support::run_harness::{FINISH_WAIT, REQUEST_WAIT, RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{approve, commit, done};

/// The actions the daemon lists on `target` of `run`.
fn listed<'a>(run: &'a RunInfo, target: &ActionTarget) -> &'a [ActionInfo] {
    match target {
        ActionTarget::Run => &run.actions,
        ActionTarget::Task(id) => &task(run, id).actions,
        ActionTarget::Stage(n) => {
            &run.stages
                .iter()
                .find(|s| s.n == *n)
                .expect("the stage is listed")
                .actions
        }
    }
}

/// The request for `kind` on `target`, after asserting the daemon lists it and does not
/// refuse it.
fn tui_request(
    h: &RunHarness,
    run: &str,
    target: ActionTarget,
    kind: ActionKind,
    input: ActionInput,
) -> RunRequest {
    let info = h.run(run).expect("the run is listed");
    let actions = listed(&info, &target);
    let entry = actions
        .iter()
        .find(|a| a.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?} not listed on {target:?}: {actions:#?}"));
    assert_eq!(entry.refused_why, None, "{kind:?} on {target:?}");
    request_for(&info, &target, &kind, &input).expect("a daemon-backed kind")
}

/// Sends the goal form's request tagged, as the form does; the run it started.
fn start_goal_from(h: &RunHarness, request: RunRequest) -> String {
    match h.tagged(10, request, GOAL_WAIT) {
        RunReply::Triaged {
            run_id: Some(run),
            request_id: Some(10),
            ..
        } => run,
        other => panic!(
            "the form's goal started no run: {other:?}\n{}",
            h.log_tail()
        ),
    }
}

/// Asserts `reply` is the `Done` of a request.
fn done_reply(reply: RunReply, h: &RunHarness) {
    assert!(
        matches!(reply, RunReply::Done { .. }),
        "{reply:?}\n{}",
        h.log_tail()
    );
}

#[test]
fn e2e_tui_goal_plan_approve_answer_accept() {
    let h = harness("");
    h.script(
        "worker-t1-1",
        &[
            call(
                "task_blocked",
                json!({"kind": "question", "reason": "which file?"}),
            ),
            crate::support::run_plans::read("Answer to your question: a.txt"),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    green(&h, "t2", "b.txt");
    // The orchestrator plans two tasks and submits; it never answers (the user does).
    h.script(
        ORCH,
        &[
            prompt(),
            edit_plan(
                vec![
                    add(plan_task("t1", &["a.txt"], json!({}))),
                    add(plan_task("t2", &["b.txt"], json!({}))),
                ],
                json!({"submit": true}),
            ),
            until("/run/complete", json!(true), RUN_WAIT * 2),
            marker(),
            read(None),
        ],
    );
    // The goal, as the goal form sends it; where checks cannot be confined (Linux), the
    // user turns `unconfined checks` on.
    let mut form = tui::run_goal::GoalForm::new(h.repo.clone());
    form.goal = tui::text_area::TextArea::from_text("add the files");
    form.unconfined_checks = !cfg!(target_os = "macos");
    let run = start_goal_from(&h, form.request().expect("the form's request"));

    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    let approve = tui_request(
        &h,
        &run,
        ActionTarget::Run,
        ActionKind::Approve,
        ActionInput::None,
    );
    done_reply(h.tagged(11, approve, REQUEST_WAIT), &h);

    wait_task(&h, &run, "t1", TaskState::Blocked);
    let answer = tui_request(
        &h,
        &run,
        ActionTarget::Task("t1".into()),
        ActionKind::Answer,
        ActionInput::Answer("a.txt".into()),
    );
    done_reply(h.tagged(12, answer, REQUEST_WAIT), &h);

    h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 2);
    wait_passed(&h, 1);
    let accept = tui_request(
        &h,
        &run,
        ActionTarget::Run,
        ActionKind::Accept,
        ActionInput::None,
    );
    done_reply(h.tagged(13, accept, FINISH_WAIT), &h);
    assert_eq!(h.run(&run).unwrap().state, RunState::Accepted);
    assert_eq!(h.git(&["show", "main:a.txt"]), "a");
    assert_eq!(h.git(&["show", "main:b.txt"]), "t2");
}

/// A one-task plan whose worker waits for `gate` before it commits.
fn gated_plan(h: &RunHarness, gate: &std::path::Path) -> String {
    use crate::support::run_plans::{plan, task};
    h.script(
        "worker-t1-1",
        &[wait_file(gate), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    plan("", &[task("t1", &["a.txt"], "")])
}

#[test]
fn e2e_tui_accept_onto_a_moved_base() {
    let h = RunHarness::new("");
    let gate = h.dir.path().join("go");
    std::fs::write(&gate, "").unwrap();
    let run = h.start(&gated_plan(&h, &gate), true);
    h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);

    std::fs::write(h.repo.join("other.txt"), "other\n").unwrap();
    h.git(&["add", "other.txt"]);
    h.git(&["commit", "-qm", "base moves on"]);
    let to = h.git(&["rev-parse", "HEAD"]);

    let accept = tui_request(
        &h,
        &run,
        ActionTarget::Run,
        ActionKind::Accept,
        ActionInput::None,
    );
    let moved = match h.tagged(21, accept.clone(), FINISH_WAIT) {
        RunReply::ConfirmNeeded {
            base_moved: Some(moved),
            request_id: Some(21),
            ..
        } => moved,
        other => panic!("expected the moved base, got {other:?}"),
    };
    assert_eq!(moved.to, to);
    assert_eq!(h.git(&["rev-parse", "HEAD"]), to, "the base was touched");
    assert_eq!(h.run(&run).unwrap().state, RunState::Complete);

    // The moved-base page's resend (decision 18): the same request, confirmed with the
    // driver's `<run id>@<full to>`.
    let RunRequest::Finish { run_id, action, .. } = accept else {
        panic!("accept is a Finish: {accept:?}")
    };
    let resend = RunRequest::Finish {
        run_id,
        action,
        confirm: Some(moved_base_confirm(&run, &moved)),
    };
    done_reply(h.tagged(22, resend, FINISH_WAIT), &h);
    assert_eq!(h.run(&run).unwrap().state, RunState::Accepted);
    let parents = h.git(&["log", "-1", "--format=%P"]);
    assert!(parents.starts_with(&to), "{parents}");
}

/// A valid input for `kind`'s form (the action matrix's, task 7): Resume carries a
/// rebaseline (preflight F31).
fn input_for(kind: &ActionKind) -> ActionInput {
    match kind {
        ActionKind::Answer => ActionInput::Answer("use a.txt".into()),
        ActionKind::Message | ActionKind::MessageStage { .. } => ActionInput::Message {
            kind: MessageKind::Info,
            text: "a note".into(),
        },
        ActionKind::Override => ActionInput::Reason("checked by hand".into()),
        ActionKind::Resume => ActionInput::Resume { rebaseline: true },
        ActionKind::Promote => ActionInput::Promote(None),
        _ => ActionInput::None,
    }
}

#[test]
fn e2e_tui_listed_refusals_match_replies() {
    let h = RunHarness::new("");
    let gate = h.dir.path().join("go");
    let run = h.start(&gated_plan(&h, &gate), true);
    let info = wait_task(&h, &run, "t1", TaskState::Working);
    assert_eq!(info.state, RunState::Running);

    let mut targets = vec![ActionTarget::Run];
    targets.extend(info.stages.iter().map(|s| ActionTarget::Stage(s.n)));
    targets.extend(info.tasks.iter().map(|t| ActionTarget::Task(t.id.clone())));
    let mut sent = Vec::new();
    for target in &targets {
        for action in listed(&info, target) {
            let Some(why) = &action.refused_why else {
                continue;
            };
            let request = request_for(&info, target, &action.kind, &input_for(&action.kind))
                .unwrap_or_else(|| panic!("{:?} on {target:?} makes no request", action.kind));
            let id = 100 + sent.len() as u64;
            match h.tagged(id, request, FINISH_WAIT) {
                RunReply::Refused { message, .. } => {
                    assert_eq!(&message, why, "{:?} on {target:?}", action.kind)
                }
                other => panic!(
                    "{:?} on {target:?}: expected {why:?}, got {other:?}",
                    action.kind
                ),
            }
            sent.push(action.kind.clone());
        }
    }
    // A running run lists accept and discard, refused; a working task lists only the
    // kinds decision 9 finds relevant to it, and none of them is refused.
    for kind in [ActionKind::Accept, ActionKind::Discard] {
        assert!(sent.contains(&kind), "{kind:?} not refused: {sent:?}");
    }
    // Nothing a refusal answered changed the run.
    let after = h.run(&run).unwrap();
    assert_eq!(after.state, RunState::Running);
    assert_eq!(task(&after, "t1").state, TaskState::Working);

    std::fs::write(&gate, "").unwrap();
    h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
}
