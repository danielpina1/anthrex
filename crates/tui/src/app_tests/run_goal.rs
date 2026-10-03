//! M9.15, decision 44: `C-b g` opens the goal form for the selected Git project (or the
//! focused window's), and Ctrl-S (milestone 9.3 decision 7; `Enter` until then) sends
//! M8b's `StartGoal` as a tagged request (decision 2). The form never guesses a repository, keeps its input on a refusal,
//! and opens the run view once a snapshot names the new run.

use super::orch::tagged;
use super::runs::{app_with_runs, deliver, run_info, snapshot};
use super::*;
use crate::app::Modal;
use crate::run_goal::{GoalField, GoalForm};
use crate::tree::NodeKey;
use proto::run_wire::request::START_GOAL;
use proto::{
    DeciderSource, OrchestratorChoice, RunPath, RunReply, RunRequest, Scale, TaskKind, TriageInfo,
};

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// Ctrl-S: the goal dialog's start, from anywhere in it (milestone 9.3 decision 7).
fn start(app: &mut App) -> Vec<Effect> {
    press(app, KeyCode::Char('s'), KeyModifiers::CONTROL)
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(tap(app, KeyCode::Char(c)).is_empty());
    }
}

fn goal_form(app: &App) -> &GoalForm {
    match &app.modal {
        Some(Modal::StartGoal(form)) => form,
        other => panic!("no goal form: {other:?}"),
    }
}

fn open_goal_form(app: &mut App) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char('g'), KeyModifiers::NONE)
}

fn triage() -> TriageInfo {
    TriageInfo {
        kinds: vec![TaskKind::Code],
        scale: Scale::Plan,
        path: RunPath::Plan,
        reason: "several modules".into(),
        source: DeciderSource::Decider,
        fallback_reason: None,
        at: 0,
    }
}

fn triaged(run_id: Option<&str>, id: u64) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Triaged {
        triage: triage(),
        run_id: run_id.map(str::to_owned),
        message: "planning run r-new: the orchestrator starts in a moment".into(),
        request_id: Some(id),
    })
}

fn refused(message: &str, id: Option<u64>) -> DaemonMsg {
    DaemonMsg::Run(RunReply::Refused {
        request: START_GOAL.into(),
        message: message.into(),
        request_id: id,
    })
}

/// `/p/a`, `/p/b` and `/p/b` windows, one focused, with the runs snapshot.
fn projects_app() -> App {
    app_with_runs(project_windows(), snapshot(1, 100, vec![]))
}

/// Selects `key` in the project tree.
fn select_node(app: &mut App, key: NodeKey) {
    let rows = crate::tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, key.clone());
    assert_eq!(app.tree.selected, Some(key));
}

/// Opens the form on `/p/a` (its project row selected) and types a goal.
fn filled_form(app: &mut App) {
    select_node(app, NodeKey::Project("/p/a".into()));
    assert!(open_goal_form(app).is_empty());
    assert_eq!(goal_form(app).project, std::path::PathBuf::from("/p/a"));
    typed(app, "add a");
}

#[test]
fn goal_form_requires_a_selected_project() {
    // 9.0.7 decision 37: nothing selected and no window, the start directory.
    let mut app = app_with_runs(vec![], snapshot(1, 100, vec![]));
    assert!(open_goal_form(&mut app).is_empty());
    assert_eq!(goal_form(&app).project, std::path::PathBuf::from("/tmp"));
    // With no start directory either, the toast.
    let mut app = app_with_runs(vec![], snapshot(1, 100, vec![]));
    app.default_dir = std::path::PathBuf::new();
    assert!(open_goal_form(&mut app).is_empty());
    assert_eq!(app.modal, None);
    assert_eq!(
        app.toast_text(),
        Some("select a Git project to start a goal")
    );

    // With nothing selected, the focused window's project.
    let mut app = projects_app();
    app.tree.selected = None;
    let focused = app
        .focused_window()
        .expect("a focused window")
        .project
        .clone();
    assert!(open_goal_form(&mut app).is_empty());
    assert_eq!(goal_form(&app).project, focused);
    assert_eq!(goal_form(&app).focus, GoalField::Goal);
    assert!(!goal_form(&app).trust_project);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
}

#[test]
fn goal_form_takes_a_selected_runs_project() {
    let mut run = run_info("r-old");
    run.project = "/p/c".into();
    let mut app = app_with_runs(project_windows(), snapshot(1, 100, vec![run]));
    select_node(&mut app, NodeKey::Run("r-old".into()));
    assert!(open_goal_form(&mut app).is_empty());
    assert_eq!(goal_form(&app).project, std::path::PathBuf::from("/p/c"));
}

#[test]
fn goal_form_sends_start_goal_and_opens_the_run() {
    let mut app = projects_app();
    filled_form(&mut app);
    // Ctrl-J is a newline in the goal.
    assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL).is_empty());
    typed(&mut app, "b");
    assert!(tap(&mut app, KeyCode::Tab).is_empty());
    assert_eq!(goal_form(&app).focus, GoalField::Runtime);
    assert!(tap(&mut app, KeyCode::Right).is_empty());
    assert!(tap(&mut app, KeyCode::Tab).is_empty());
    // No settings cache here: the picker is `default`, `custom…` (9.0.6 decision 39).
    assert!(tap(&mut app, KeyCode::Right).is_empty());
    typed(&mut app, "claude-opus-5");
    assert!(tap(&mut app, KeyCode::Tab).is_empty());
    // Milestone 9.3 decision 25: the orchestrator row, `new` with no idle one.
    assert_eq!(goal_form(&app).focus, GoalField::Orchestrator);
    assert!(tap(&mut app, KeyCode::Tab).is_empty());
    // Ruling R-13: configured, then local, then pr.
    assert_eq!(goal_form(&app).focus, GoalField::Delivery);
    assert!(tap(&mut app, KeyCode::Right).is_empty());
    assert!(tap(&mut app, KeyCode::Right).is_empty());
    assert!(tap(&mut app, KeyCode::Tab).is_empty());
    assert_eq!(goal_form(&app).focus, GoalField::Trust);
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    let (id, request) = tagged(&start(&mut app));
    assert_eq!(
        request,
        RunRequest::StartGoal {
            goal: "add a\nb".into(),
            dir: "/p/a".into(),
            yes: false,
            trust_project: true,
            unconfined_checks: false,
            orchestrator: Some(OrchestratorChoice {
                runtime: proto::Runtime::Claude,
                model: Some("claude-opus-5".into()),
            }),
            delivery: Some(proto::DeliveryMode::Pr),
            continue_from: None,
        }
    );
    assert!(goal_form(&app).submitting);
    // While submitting only Esc acts: a second start sends nothing.
    assert!(start(&mut app).is_empty());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());

    // The snapshot names the run first here, so the reply opens the view at once.
    let mut new = run_info("r-new");
    new.project = "/p/a".into();
    deliver(&mut app, snapshot(2, 100, vec![new]));
    assert!(app.on_daemon(triaged(Some("r-new"), id)).is_empty());
    assert_eq!(app.modal, None);
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("r-new")
    );
}

#[test]
fn goal_form_sends_no_orchestrator_by_default() {
    let mut app = projects_app();
    filled_form(&mut app);
    let (_, request) = tagged(&start(&mut app));
    assert!(matches!(
        request,
        RunRequest::StartGoal {
            orchestrator: None,
            trust_project: false,
            yes: false,
            unconfined_checks: false,
            ..
        }
    ));
}

#[test]
fn goal_form_opens_the_view_only_once_the_snapshot_names_the_run() {
    let mut app = projects_app();
    filled_form(&mut app);
    let (id, _) = tagged(&start(&mut app));
    assert!(app.on_daemon(triaged(Some("r-new"), id)).is_empty());
    assert_eq!(app.modal, None, "success closes the form");
    assert_eq!(
        app.toast_text(),
        Some("planning run r-new: the orchestrator starts in a moment")
    );
    assert_eq!(app.run_view, None, "no snapshot names r-new yet");
    deliver(&mut app, snapshot(2, 100, vec![run_info("r-other")]));
    assert_eq!(app.run_view, None);
    deliver(&mut app, snapshot(3, 100, vec![run_info("r-new")]));
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("r-new")
    );
    assert_eq!(app.tree.selected, Some(NodeKey::Run("r-new".into())));
    // Only once: leaving the view is not undone by the next snapshot.
    app.close_run_view();
    deliver(&mut app, snapshot(4, 100, vec![run_info("r-new")]));
    assert_eq!(app.run_view, None);
}

#[test]
fn a_goal_triage_refused_closes_the_form_with_its_message() {
    let mut app = projects_app();
    filled_form(&mut app);
    let (id, _) = tagged(&start(&mut app));
    app.on_daemon(triaged(None, id));
    assert_eq!(app.modal, None);
    assert_eq!(
        app.toast_text(),
        Some("planning run r-new: the orchestrator starts in a moment")
    );
    deliver(&mut app, snapshot(2, 100, vec![run_info("r-new")]));
    assert_eq!(app.run_view, None);
}

#[test]
fn goal_form_keeps_its_input_on_error() {
    let mut app = projects_app();
    select_node(&mut app, NodeKey::Project("/p/a".into()));
    open_goal_form(&mut app);
    // An empty goal is refused inline, with nothing sent.
    assert!(start(&mut app).is_empty());
    assert_eq!(goal_form(&app).error.as_deref(), Some("type a goal first"));
    typed(&mut app, "add a");
    let (id, _) = tagged(&start(&mut app));
    app.toast("");
    app.on_daemon(refused(
        "run start --goal needs a Git repository: /p/a is not one\nsecond line",
        Some(id),
    ));
    let form = goal_form(&app);
    assert_eq!(
        form.error.as_deref(),
        Some("run start --goal needs a Git repository: /p/a is not one (+1 more)")
    );
    assert!(!form.submitting);
    assert_eq!(form.goal.text(), "add a");
    assert_eq!(app.toast_text(), Some(""), "shown inline, not toasted");
    // Ctrl-S sends it again, under a new id.
    let (again, _) = tagged(&start(&mut app));
    assert_ne!(again, id);
}

#[test]
fn a_refusal_for_an_earlier_request_does_not_reach_the_form() {
    let mut app = projects_app();
    filled_form(&mut app);
    let (id, _) = tagged(&start(&mut app));
    let before = goal_form(&app).clone();
    app.on_daemon(refused("an older goal was refused", Some(id + 100)));
    assert_eq!(goal_form(&app), &before);
    assert_eq!(app.toast_text(), Some("an older goal was refused"));
    app.on_daemon(refused("an untagged refusal", None));
    assert_eq!(goal_form(&app), &before);
    app.on_daemon(triaged(Some("r-else"), id + 1));
    assert_eq!(goal_form(&app), &before, "another request's triage");
}

#[test]
fn a_goal_that_was_not_sent_frees_the_form() {
    let mut app = projects_app();
    filled_form(&mut app);
    let effects = start(&mut app);
    let Effect::Send(msg) = &effects[0] else {
        panic!("{effects:?}")
    };
    app.on_send_failed(msg);
    let form = goal_form(&app);
    assert!(!form.submitting);
    assert_eq!(
        form.error.as_deref(),
        Some("the goal was not sent; press Ctrl-S to retry")
    );
}

/// M9.15 review: a goal's reply that finds its form closed (`Esc` while it waited) is
/// still shown, as an edit's `Done` is.
#[test]
fn a_goal_reply_after_the_form_closed_is_toasted() {
    let mut app = projects_app();
    filled_form(&mut app);
    let (id, _) = tagged(&start(&mut app));
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
    app.toast("");
    app.on_daemon(triaged(Some("r-new"), id));
    assert_eq!(
        app.toast_text(),
        Some("planning run r-new: the orchestrator starts in a moment")
    );
}
