//! Milestone 9.0.6 task 11: the action menu's input forms (decision 15): answer,
//! message, override, resume and promote, each ending on its confirmation page.

use super::actions::{action, flow, running_snapshot, sent_tagged_id, tap};
use super::runs::{app_with_runs, open_run_view};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::actions::ActionStep;
use crate::app::actions::forms::{ActionForm, Brief, FormOutcome, PromoteForm, PromoteOption};
use crate::tree::run_fixtures::RUN_ID;
use crate::tree::stage_fixtures::staged_fixture;
use proto::{
    ActionKind, BaseMovedInfo, BlockInfo, BlockReason, MessageKind, MessageTarget,
    OrchestratorChoice, PlanEdit, RunReply, RunRequest, Runtime, TaskDetailInfo, TaskState,
};

/// The running fixture with `t1` blocked on a question and every input kind listed.
pub(super) fn forms_app(base_moved: bool) -> App {
    let (mut snap, windows) = running_snapshot();
    let run = &mut snap.runs[0];
    run.actions
        .push(action(ActionKind::Resume, "resume halted run", None));
    run.actions
        .push(action(ActionKind::Promote, "promote", None));
    if base_moved {
        run.base_moved = Some(BaseMovedInfo {
            from: "b".repeat(40),
            to: "a".repeat(40),
            commits: vec![],
            total: 3,
        });
    }
    let t1 = &mut run.tasks[1];
    t1.state = TaskState::Blocked;
    t1.block = Some(BlockInfo {
        reason: BlockReason::Question,
        text: "which db?\nsecond line".into(),
    });
    t1.actions = vec![
        action(ActionKind::Answer, "answer", None),
        action(ActionKind::Message, "message", None),
        action(ActionKind::Override, "override", None),
    ];
    app_with_runs(windows, snap)
}

pub(super) fn open(app: &mut App, target: ActionTarget, kind: ActionKind) {
    app.open_actions((RUN_ID.into(), target), Some(kind));
}

pub(super) fn task_t1() -> ActionTarget {
    ActionTarget::Task("t1".into())
}

pub(super) fn form(app: &App) -> &ActionForm {
    match &flow(app).step {
        ActionStep::Form(form) => form,
        other => panic!("no form: {other:?}"),
    }
}

pub(super) fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        tap(app, KeyCode::Char(c));
    }
}

fn ctrl_j(app: &mut App) {
    press(app, KeyCode::Char('j'), KeyModifiers::CONTROL);
}

fn edit_of(request: &RunRequest) -> Vec<PlanEdit> {
    match request {
        RunRequest::Edit { edits, .. } => edits.clone(),
        other => panic!("{other:?}"),
    }
}

fn the_request(effects: &[Effect]) -> RunRequest {
    match effects {
        [Effect::Send(ClientMsg::RunTagged { request, .. })] => request.clone(),
        other => panic!("{other:?}"),
    }
}

pub(super) fn detail(brief: &str, request_id: u64) -> RunReply {
    RunReply::TaskDetail {
        detail: Box::new(TaskDetailInfo {
            run_id: RUN_ID.into(),
            task_id: "t1".into(),
            brief: brief.into(),
            acceptance: vec![],
            worker_summary: None,
            summary_source: None,
        }),
        request_id: Some(request_id),
    }
}

pub(super) fn answer_form(app: &App) -> &crate::app::actions::forms::AnswerForm {
    match form(app) {
        ActionForm::Answer(f) => f,
        other => panic!("{other:?}"),
    }
}

#[test]
fn answer_form_shows_the_question_title_and_brief() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let effects = tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&effects, |r| {
        *r == RunRequest::TaskDetail {
            run_id: RUN_ID.into(),
            task_id: "t1".into(),
        }
    });
    let f = answer_form(&app);
    assert_eq!(f.task, "t1 spawn");
    assert_eq!(f.question, "which db?\nsecond line");
    assert_eq!(f.brief, Brief::Loading);
    app.on_run_reply(detail("one\n\n two \nthree\nfour", id));
    assert_eq!(
        answer_form(&app).brief,
        Brief::Ready(vec!["one".into(), "two".into(), "three".into()])
    );
    assert!(app.replies.is_empty());
    assert_eq!(app.toast_text(), None, "the brief is not a toast");

    // A refused detail shows its first line, not a toast.
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Enter), |_| true);
    app.on_run_reply(RunReply::Refused {
        request: "task_detail".into(),
        message: "no such task t1\nmore".into(),
        request_id: Some(id),
    });
    assert_eq!(
        answer_form(&app).brief,
        Brief::Failed("no such task t1".into())
    );
    assert_eq!(app.toast_text(), None);
}

#[test]
fn a_brief_for_a_closed_form_is_dropped() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    let id = sent_tagged_id(&tap(&mut app, KeyCode::Enter), |_| true);
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    app.on_run_reply(detail("late", id));
    assert!(app.replies.is_empty(), "the entry went with the reply");
    assert!(app.modal.is_none());
}

#[test]
fn answer_sends_the_typed_text_after_its_page() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    tap(&mut app, KeyCode::Enter);
    type_text(&mut app, "line one");
    ctrl_j(&mut app);
    type_text(&mut app, "line two  ");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let ActionStep::Confirm(page) = &flow(&app).step else {
        panic!("{:?}", flow(&app).step);
    };
    assert!(page.form.is_some());
    assert!(
        page.details
            .iter()
            .any(|(l, v)| l == "answer" && v == "line one line two"),
        "{:?}",
        page.details
    );
    let request = the_request(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        edit_of(&request),
        vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "line one\nline two".into()
        }]
    );
}

#[test]
fn an_empty_answer_stays_on_its_form() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    tap(&mut app, KeyCode::Enter);
    type_text(&mut app, "   ");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(matches!(flow(&app).step, ActionStep::Form(_)));
}

#[test]
fn message_cycles_its_kind() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Message);
    tap(&mut app, KeyCode::Enter);
    let kind = |app: &App| match form(app) {
        ActionForm::Message(f) => f.kind,
        other => panic!("{other:?}"),
    };
    assert_eq!(kind(&app), MessageKind::Info);
    tap(&mut app, KeyCode::Tab);
    assert_eq!(kind(&app), MessageKind::Change);
    tap(&mut app, KeyCode::Tab);
    assert_eq!(kind(&app), MessageKind::StopAndWait);
    tap(&mut app, KeyCode::Tab);
    assert_eq!(kind(&app), MessageKind::Info);
    tap(&mut app, KeyCode::BackTab);
    assert_eq!(kind(&app), MessageKind::StopAndWait);
    type_text(&mut app, "hold on");
    tap(&mut app, KeyCode::Enter);
    let request = the_request(&tap(&mut app, KeyCode::Enter));
    assert_eq!(
        edit_of(&request),
        vec![PlanEdit::Message {
            to: MessageTarget::Tasks(vec!["t1".into()]),
            text: "hold on".into(),
            kind: MessageKind::StopAndWait,
        }]
    );
}

#[test]
fn message_to_a_stage_targets_the_stage() {
    let (mut snap, windows) = staged_fixture();
    snap.runs[0].stages[0].actions = vec![action(
        ActionKind::MessageStage { stage: 1 },
        "message stage 1",
        None,
    )];
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app.open_actions((RUN_ID.into(), ActionTarget::Stage(1)), None);
    tap(&mut app, KeyCode::Enter);
    type_text(&mut app, "use the new api");
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Enter);
    let request = the_request(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        edit_of(&request),
        vec![PlanEdit::Message {
            to: MessageTarget::Stage(1),
            text: "use the new api".into(),
            kind: MessageKind::Change,
        }]
    );
}

#[test]
fn override_needs_a_reason() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Override);
    tap(&mut app, KeyCode::Enter);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let ActionForm::Override(f) = form(&app) else {
        panic!();
    };
    assert!(f.blank, "`type a reason first` shows");
    assert_eq!(f.info.kind, ActionKind::Override);
    // A blank of spaces is still blank; typing clears the message.
    type_text(&mut app, "  ");
    tap(&mut app, KeyCode::Enter);
    assert!(matches!(form(&app), ActionForm::Override(f) if f.blank));
    // One line: Ctrl-J adds nothing.
    ctrl_j(&mut app);
    type_text(&mut app, "flaky check");
    let ActionForm::Override(f) = form(&app) else {
        panic!();
    };
    assert!(!f.blank);
    assert_eq!(f.reason.text(), "  flaky check");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(matches!(flow(&app).step, ActionStep::Confirm(_)));
    let request = the_request(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        RunRequest::Override {
            run_id: RUN_ID.into(),
            task_id: "t1".into(),
            reason: "flaky check".into(),
        }
    );
}

#[test]
fn resume_defaults_rebaseline_on_when_the_base_moved() {
    let rebaseline = |app: &App| match form(app) {
        ActionForm::Resume(f) => (f.rebaseline, f.moved.is_some()),
        other => panic!("{other:?}"),
    };
    let mut app = forms_app(true);
    open(&mut app, ActionTarget::Run, ActionKind::Resume);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(rebaseline(&app), (true, true));
    tap(&mut app, KeyCode::Tab);
    assert_eq!(rebaseline(&app), (false, true));
    tap(&mut app, KeyCode::Char(' '));
    assert_eq!(rebaseline(&app), (true, true));
    tap(&mut app, KeyCode::Enter);
    let request = the_request(&tap(&mut app, KeyCode::Char('y')));
    assert_eq!(
        request,
        RunRequest::Resume {
            run_id: RUN_ID.into(),
            rebaseline: true
        }
    );

    let mut app = forms_app(false);
    open(&mut app, ActionTarget::Run, ActionKind::Resume);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(rebaseline(&app), (false, false));
    tap(&mut app, KeyCode::Enter);
    let request = the_request(&tap(&mut app, KeyCode::Enter));
    assert_eq!(
        request,
        RunRequest::Resume {
            run_id: RUN_ID.into(),
            rebaseline: false
        }
    );
}

#[test]
fn promote_offers_configured_without_a_cache() {
    let mut app = forms_app(false);
    open(&mut app, ActionTarget::Run, ActionKind::Promote);
    tap(&mut app, KeyCode::Enter);
    let ActionForm::Promote(f) = form(&app) else {
        panic!();
    };
    assert_eq!(f.options, vec![PromoteOption::configured()]);
    // Cycling with one entry stays on it.
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Enter);
    let request = the_request(&tap(&mut app, KeyCode::Enter));
    assert_eq!(
        request,
        RunRequest::Promote {
            run_id: RUN_ID.into(),
            orchestrator: None
        }
    );
}

/// The picker's input for a roster entry (task 12 fills the options from the cache).
#[test]
fn a_roster_option_becomes_the_orchestrator_choice() {
    let mut f = ActionForm::Promote(PromoteForm {
        info: action(ActionKind::Promote, "promote", None),
        options: vec![
            PromoteOption::configured(),
            PromoteOption::roster(Runtime::Claude, "claude-opus-5-5"),
            PromoteOption::roster(Runtime::Codex, "gpt-6-sol"),
        ],
        at: 0,
    });
    let labels: Vec<_> = match &f {
        ActionForm::Promote(p) => p.options.iter().map(|o| o.label.clone()).collect(),
        _ => unreachable!(),
    };
    assert_eq!(labels, ["configured", "cl claude-opus-5-5", "cx gpt-6-sol"]);
    f.on_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    f.on_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
    assert_eq!(
        f.input(),
        Ok(crate::actions_request::ActionInput::Promote(Some(
            OrchestratorChoice {
                runtime: Runtime::Codex,
                model: Some("gpt-6-sol".into()),
                effort: None
            }
        )))
    );
    f.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    f.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
    assert_eq!(
        f.input(),
        Ok(crate::actions_request::ActionInput::Promote(None))
    );
    assert_eq!(
        f.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        FormOutcome::Back
    );
}

#[test]
fn esc_on_a_page_returns_to_its_form_with_the_text_kept() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Message);
    tap(&mut app, KeyCode::Enter);
    type_text(&mut app, "keep me");
    tap(&mut app, KeyCode::Tab);
    tap(&mut app, KeyCode::Enter);
    assert!(matches!(flow(&app).step, ActionStep::Confirm(_)));
    tap(&mut app, KeyCode::Esc);
    let ActionForm::Message(f) = form(&app) else {
        panic!();
    };
    assert_eq!((f.text.text(), f.kind), ("keep me", MessageKind::Change));
    // Esc on the form goes to the menu.
    tap(&mut app, KeyCode::Esc);
    assert!(matches!(flow(&app).step, ActionStep::Menu));
}

#[test]
fn a_paste_goes_to_the_form_and_a_reason_stays_one_line() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Answer);
    tap(&mut app, KeyCode::Enter);
    assert!(app.on_paste("a\r\nb\x1b[2J".into()).is_empty());
    assert_eq!(answer_form(&app).text.text(), "a\nb[2J");
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Esc);
    open(&mut app, task_t1(), ActionKind::Override);
    tap(&mut app, KeyCode::Enter);
    app.on_paste("why\nnot".into());
    let ActionForm::Override(f) = form(&app) else {
        panic!();
    };
    assert_eq!(f.reason.text(), "why not");
}

#[test]
fn a_form_whose_action_left_goes_back_to_the_menu() {
    let mut app = forms_app(false);
    open(&mut app, task_t1(), ActionKind::Override);
    tap(&mut app, KeyCode::Enter);
    let (mut snap, _) = running_snapshot();
    snap.runs[0].tasks[1].actions = vec![action(ActionKind::Message, "message", None)];
    app.on_run_reply(RunReply::Snapshot(snap));
    assert!(matches!(flow(&app).step, ActionStep::Menu));
    assert_eq!(app.toast_text(), Some("override is no longer available"));
}
