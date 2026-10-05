//! M9.0.5.6: the plan review's state and keys (decisions 11–16). Pure reducer tests:
//! each asserts the returned `Vec<Effect>` and the state.

use super::gate::{select, send, tap, task_key};
use super::runs::{app_with_runs, deliver, open_run_view};
use super::*;
use crate::app::plan_review::{detail_lines, review_tasks};
use crate::app::{Modal, PendingAction, PlanReview, ReviewTarget, TreeInput};
use crate::tree::NodeKey;
use crate::tree::orch_fixtures::{held_fixture, hold, orch_fixture};
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, task};
use proto::{HoldState, PlanEdit, RunRequest, RunState, RunsSnapshot, Size, TaskState};
use ratatui::layout::Rect;

/// The gate fixture with a third task `t3` after `t1`, and a 40-line brief on `t1`.
pub(super) fn gate_snapshot() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = gate_fixture();
    let run = &mut snap.runs[0];
    run.tasks[0].brief = (1..=40)
        .map(|n| format!("brief line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut t3 = task("t3", "reset email", Size::S, TaskState::Pending);
    t3.deps = vec!["t1".into()];
    run.tasks.push(t3);
    (snap, windows)
}

/// The run view open on the gate snapshot, its root selected.
pub(super) fn gate_app() -> App {
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app
}

/// The held fixture (hold `epic:ui` over `t2`) and a second, older awaiting hold
/// `epic:api` over a new held task `t4`.
pub(super) fn two_holds() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = held_fixture();
    let run = &mut snap.runs[0];
    let mut t4 = task("t4", "api", Size::S, TaskState::Pending);
    t4.hold = Some("epic:api".into());
    run.tasks.push(t4);
    let mut api = hold("epic:api", HoldState::Awaiting, &["t4"]);
    api.created_at -= 30;
    run.holds.push(api);
    (snap, windows)
}

pub(super) fn held_app(snap: (RunsSnapshot, Vec<WindowInfo>)) -> App {
    let (snap, windows) = snap;
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    app
}

pub(super) fn review(app: &App) -> &PlanReview {
    app.plan_review.as_ref().expect("the review is open")
}

fn review_ids(app: &App) -> Vec<String> {
    let review = review(app);
    let run = app
        .runs
        .runs
        .iter()
        .find(|run| run.run_id == review.run_id)
        .expect("the reviewed run");
    review_tasks(run, &review.target)
        .into_iter()
        .map(|task| task.id.clone())
        .collect()
}

pub(super) fn confirm(app: &App) -> (&str, &PendingAction) {
    match &app.modal {
        Some(Modal::Confirm { message, action }) => (message, action),
        other => panic!("no confirm: {other:?}"),
    }
}

fn root() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

#[test]
fn open_from_the_run_view_at_the_gate_selects_the_first_task() {
    let mut app = gate_app();
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(
        app.plan_review,
        Some(PlanReview {
            run_id: RUN_ID.into(),
            target: ReviewTarget::Gate,
            selected: Some("t1".into()),
            scroll: 0,
            doc: None,
        })
    );
    assert!(app.keymap.review_mode());
    assert_eq!(review_ids(&app), ["t1", "t2", "t3"]);
    // Nothing underneath moved.
    assert_eq!(app.tree.selected, Some(root()));
    assert!(app.run_view.is_some());
}

#[test]
fn p_on_a_run_with_an_awaiting_hold_reviews_that_hold_only() {
    let mut app = held_app(held_fixture());
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:ui".into()));
    assert_eq!(review(&app).selected.as_deref(), Some("t2"));
    assert_eq!(review_ids(&app), ["t2"]);

    // Two awaiting holds: the selected held task's hold, else the oldest.
    let mut app = held_app(two_holds());
    select(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:ui".into()));
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    select(&mut app, root());
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:api".into()));
    assert_eq!(review_ids(&app), ["t4"]);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    // A task that is not held names no hold: the oldest again.
    select(&mut app, task_key("t0"));
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:api".into()));
}

#[test]
fn p_with_nothing_awaiting_toasts() {
    let (snap, windows) = orch_fixture(RunState::Running);
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('p')).is_empty());
    assert_eq!(app.plan_review, None);
    assert!(!app.keymap.review_mode());
    assert_eq!(
        app.toast_text(),
        Some("run add-reset-3f9a has nothing awaiting approval")
    );
}

#[test]
fn j_k_select_and_reset_the_scroll() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    let selected = |app: &App| review(app).selected.clone();
    app.plan_review.as_mut().expect("open").scroll = 3;
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(selected(&app).as_deref(), Some("t2"));
    assert_eq!(review(&app).scroll, 0);
    assert!(tap(&mut app, KeyCode::Down).is_empty());
    assert_eq!(selected(&app).as_deref(), Some("t3"));
    // The last task stays selected.
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(selected(&app).as_deref(), Some("t3"));
    app.plan_review.as_mut().expect("open").scroll = 2;
    assert!(tap(&mut app, KeyCode::Char('k')).is_empty());
    assert_eq!(selected(&app).as_deref(), Some("t2"));
    assert_eq!(review(&app).scroll, 0);
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(selected(&app).as_deref(), Some("t1"));
    // The tree's selection never moves.
    assert_eq!(app.tree.selected, Some(root()));
}

#[test]
fn page_keys_scroll_within_the_content() {
    let mut app = gate_app();
    let body = Rect::new(0, 0, 80, 23);
    app.set_body_area(body);
    tap(&mut app, KeyCode::Char('p'));
    // Milestone 9.0.7 decision 26: the detail is the stacked geometry's last area.
    let detail = app.review_layout(body).detail;
    let run = &app.runs.runs[0];
    let rows = detail_lines(run, &run.tasks[0], detail.width, app.palette()).len() as u16;
    let page = detail.height - 1;
    assert!(
        rows > detail.height + page,
        "the fixture's brief spans pages"
    );
    let max = rows - detail.height;

    assert!(tap(&mut app, KeyCode::PageDown).is_empty());
    assert_eq!(review(&app).scroll, page);
    for _ in 0..10 {
        tap(&mut app, KeyCode::PageDown);
    }
    assert_eq!(review(&app).scroll, max, "clamped to the content");
    assert!(tap(&mut app, KeyCode::PageUp).is_empty());
    assert_eq!(review(&app).scroll, max - page);
    for _ in 0..10 {
        tap(&mut app, KeyCode::PageUp);
    }
    assert_eq!(review(&app).scroll, 0);

    // A task whose detail fits never scrolls.
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(review(&app).scroll, 0);
}

#[test]
fn a_in_the_review_asks_to_approve_the_run() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(review(&app).selected.as_deref(), Some("t2"));
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(
        confirm(&app),
        (
            "Approve run add-reset-3f9a? 3 tasks start.",
            &PendingAction::ApproveRun(RUN_ID.into())
        )
    );
    assert_eq!(
        tap(&mut app, KeyCode::Char('y')),
        send(RunRequest::Approve {
            run_id: RUN_ID.into()
        })
    );
    assert_eq!(app.modal, None);
    assert!(
        app.plan_review.is_some(),
        "the snapshot closes it, not the key"
    );
}

#[test]
fn x_e_d_act_on_the_reviewed_task() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(app.tree.selected, Some(root()), "the tree names no task");

    assert!(tap(&mut app, KeyCode::Char('d')).is_empty());
    assert_eq!(
        confirm(&app),
        (
            "Remove t2 from run add-reset-3f9a's plan?",
            &PendingAction::RemoveTask {
                run_id: RUN_ID.into(),
                task_id: "t2".into()
            }
        )
    );
    assert_eq!(
        tap(&mut app, KeyCode::Char('y')),
        send(RunRequest::Edit {
            run_id: RUN_ID.into(),
            edits: vec![PlanEdit::CancelTask {
                task_id: "t2".into()
            }],
            submit: false,
        })
    );

    assert!(tap(&mut app, KeyCode::Char('e')).is_empty());
    match &app.modal {
        Some(Modal::EditTask(form)) => assert_eq!(form.task_id, "t2"),
        other => panic!("no edit form: {other:?}"),
    }
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.modal, None);
    assert!(
        app.plan_review.is_some(),
        "Esc closed the form, not the review"
    );

    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(confirm(&app).1, &PendingAction::RejectRun(RUN_ID.into()));
    assert!(tap(&mut app, KeyCode::Char('n')).is_empty());
}

#[test]
fn a_on_a_hold_review_sends_approve_hold_at_once() {
    let mut app = held_app(two_holds());
    select(&mut app, task_key("t2"));
    tap(&mut app, KeyCode::Char('p'));
    select(&mut app, root());
    assert_eq!(
        tap(&mut app, KeyCode::Char('a')),
        send(RunRequest::ApproveHold {
            run_id: RUN_ID.into(),
            hold: "epic:ui".into(),
        })
    );
    assert_eq!(app.modal, None);
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(
        confirm(&app).1,
        &PendingAction::RejectHold {
            run_id: RUN_ID.into(),
            hold: "epic:ui".into(),
        }
    );
    assert!(tap(&mut app, KeyCode::Char('n')).is_empty());
    // Past the gate, `e` and `d` toast as the run view does (Out: no edits of holds).
    assert!(tap(&mut app, KeyCode::Char('e')).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("the plan gate is closed: run add-reset-3f9a is running")
    );
    assert_eq!(app.modal, None);
}

#[test]
fn esc_returns_to_where_it_was_opened() {
    // From the run view: the view and its selection as they were.
    let mut app = gate_app();
    select(&mut app, task_key("t2"));
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::PageDown);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.plan_review, None);
    assert!(!app.keymap.review_mode());
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, Some(task_key("t2")));
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(app.keymap.tree_mode());

    // Opened over the terminal (the alerts' way in): the terminal, tree mode off.
    let (snap, windows) = gate_snapshot();
    let mut app = app_with_runs(windows, snap);
    assert!(
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate)
            .is_empty()
    );
    assert!(app.keymap.review_mode());
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.plan_review, None);
    assert!(!app.overview);
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert!(!app.keymap.review_mode());
    // Bare keys reach the terminal again.
    assert_eq!(
        tap(&mut app, KeyCode::Char('j')),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"j".to_vec()
        })]
    );
}

#[test]
fn the_review_closes_when_the_gate_closes() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    // A snapshot that keeps the gate open keeps the review.
    let (snap, _) = gate_snapshot();
    deliver(&mut app, snap);
    assert!(app.plan_review.is_some());

    let (mut snap, _) = gate_snapshot();
    snap.runs[0].state = RunState::Running;
    assert!(deliver(&mut app, snap).is_empty());
    assert_eq!(app.plan_review, None);
    assert!(!app.keymap.review_mode());
    assert_eq!(app.toast_text(), None, "no toast of its own");
    assert!(app.run_view.is_some(), "the run view stays");

    // A run that is gone closes it too.
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    let (mut snap, _) = gate_snapshot();
    snap.runs.clear();
    deliver(&mut app, snap);
    assert_eq!(app.plan_review, None);
    assert!(!app.keymap.review_mode());
}

#[test]
fn the_review_closes_when_its_hold_is_decided() {
    let mut app = held_app(two_holds());
    select(&mut app, task_key("t2"));
    tap(&mut app, KeyCode::Char('p'));
    // The other hold decided: this one still awaits.
    let (mut snap, _) = two_holds();
    snap.runs[0].holds[1].state = HoldState::Approved;
    deliver(&mut app, snap);
    assert_eq!(review(&app).target, ReviewTarget::Hold("epic:ui".into()));

    let (mut snap, _) = two_holds();
    snap.runs[0].holds[0].state = HoldState::Approved;
    deliver(&mut app, snap);
    assert_eq!(app.plan_review, None);
    assert!(!app.keymap.review_mode());
    assert_eq!(app.toast_text(), None);
}

#[test]
fn a_dropped_task_moves_the_selection_to_its_place() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    let (mut snap, _) = gate_snapshot();
    snap.runs[0].tasks[1].state = TaskState::Cancelled;
    deliver(&mut app, snap);
    assert_eq!(review_ids(&app), ["t1", "t3"]);
    assert_eq!(review(&app).selected.as_deref(), Some("t3"));
}

#[test]
fn c_b_a_toasts_while_the_review_is_open() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert_eq!(app.toast_text(), Some("leave the plan review first (esc)"));
    assert!(app.plan_review.is_some());
    assert!(app.keymap.review_mode());
}

#[test]
fn review_keys_send_nothing_else() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    tap(&mut app, KeyCode::Char('j'));
    let before = app.plan_review.clone();
    let tree_before = app.tree.selected.clone();
    let acting = ['a', 'x', 'e', 'd', 'j', 'k'];
    let mut codes: Vec<KeyCode> = (' '..='~')
        .filter(|c| !acting.contains(c))
        .map(KeyCode::Char)
        .collect();
    codes.extend([
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Delete,
        KeyCode::Insert,
        KeyCode::F(1),
    ]);
    for code in codes {
        for mods in [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::ALT] {
            assert_eq!(press(&mut app, code, mods), vec![], "{code:?} {mods:?}");
            assert_eq!(app.plan_review, before, "{code:?} {mods:?}");
            assert_eq!(app.modal, None, "{code:?} {mods:?}");
            assert_eq!(app.tree.selected, tree_before, "{code:?} {mods:?}");
            assert!(app.run_view.is_some(), "{code:?} {mods:?}");
        }
    }
}

/// The review covers the body, so a click, drag or wheel there never reaches the
/// sidebar, the canvas or the terminal drawn under it (M9.0.5.7).
#[test]
fn the_mouse_does_nothing_under_the_review() {
    let mut app = gate_app();
    tap(&mut app, KeyCode::Char('p'));
    let layout = crate::ui::layout(
        Rect::new(0, 0, 120, 40),
        app.sidebar_width,
        crate::app::alerts(&app).len(),
    );
    let before_tree = app.tree.selected.clone();
    let before_pan = app.graph_pan;
    let before = app.plan_review.clone();
    let before_sidebar = app.tree.sidebar;
    let before_overview = app.tree.overview;
    for (x, y) in [(3, 2), (3, 4), (60, 10), (100, 30)] {
        assert!(app.on_click(x, y, &layout).is_empty());
        assert!(app.on_drag(x + 5, y + 3, &layout).is_empty());
        assert!(app.on_scroll(true, x, y, &layout).is_empty());
        assert!(app.on_scroll(false, x, y, &layout).is_empty());
    }
    assert_eq!(app.tree.selected, before_tree);
    assert_eq!(app.graph_pan, before_pan);
    assert_eq!(app.plan_review, before);
    assert_eq!(app.tree.sidebar, before_sidebar);
    assert_eq!(app.tree.overview, before_overview);
}
