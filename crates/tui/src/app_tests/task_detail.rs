//! Milestone 9.0.5 decisions 23 and 25: the task panel's on-demand detail, and its
//! scroll and brief keys.

use super::runs::{app_with_runs, open_run_view};
use super::*;
use crate::app::task_detail::DetailState;
use crate::tree::run_fixtures::gemini_fixture;
use crate::tree::{NodeKey, RunFilter};
use proto::{RunReply, RunRequest, TaskDetailInfo, TaskState};

fn t(id: &str) -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: id.into(),
    }
}

fn select(app: &mut App, id: &str) {
    let rows = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    app.tree.select(&rows, t(id));
    assert_eq!(app.tree.selected, Some(t(id)), "{id} is a run-view row");
}

/// The Gemini run's view on `r1`, `t2` selected, laid out at 120 x 40.
fn gemini_view() -> App {
    let (snapshot, windows) = gemini_fixture();
    let mut app = app_with_runs(windows, snapshot);
    open_run_view(&mut app, "r1");
    select(&mut app, "t2");
    let layout = crate::ui::layout(
        ratatui::layout::Rect::new(0, 0, 120, 40),
        app.sidebar_width,
        crate::app::alerts(&app).len(),
    );
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    app
}

/// The ids of the `TaskDetail` requests among `effects`, with the task each names.
fn detail_requests(effects: &[Effect]) -> Vec<(u64, String)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Send(ClientMsg::RunTagged {
                id,
                request: RunRequest::TaskDetail { run_id, task_id },
            }) => {
                assert_eq!(run_id, "r1");
                Some((*id, task_id.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Ticks `n` times and returns every detail request sent.
fn ticks(app: &mut App, n: usize) -> Vec<(u64, String)> {
    (0..n)
        .flat_map(|_| detail_requests(&app.on_tick()))
        .collect()
}

fn detail(task_id: &str, brief: &str) -> Box<TaskDetailInfo> {
    Box::new(TaskDetailInfo {
        run_id: "r1".into(),
        task_id: task_id.into(),
        brief: brief.into(),
        acceptance: vec!["it passes".into()],
        worker_summary: None,
        summary_source: None,
    })
}

fn reply(app: &mut App, task_id: &str, brief: &str, id: u64) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::TaskDetail {
        detail: detail(task_id, brief),
        request_id: Some(id),
    }))
}

fn state(app: &App) -> Option<&DetailState> {
    app.task_detail_for("r1", "t2")
}

/// Changes `t2` in the client's snapshot as a newer snapshot would.
fn change_t2(app: &mut App, change: impl FnOnce(&mut proto::TaskInfo)) {
    let task = app.runs.runs[0]
        .tasks
        .iter_mut()
        .find(|task| task.id == "t2")
        .expect("t2");
    change(task);
}

#[test]
fn selecting_a_task_asks_for_its_detail_once() {
    let mut app = gemini_view();
    let sent = ticks(&mut app, 5);
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(sent[0].1, "t2");
    assert_eq!(state(&app), Some(&DetailState::InFlight(sent[0].0)));
    // Another task: one more request, for it.
    select(&mut app, "t0");
    let sent = ticks(&mut app, 5);
    assert_eq!(
        sent.iter().map(|s| s.1.as_str()).collect::<Vec<_>>(),
        ["t0"]
    );
}

#[test]
fn a_new_turn_or_state_asks_again() {
    let mut app = gemini_view();
    let first = ticks(&mut app, 1);
    let _ = reply(&mut app, "t2", "the brief", first[0].0);
    assert!(ticks(&mut app, 3).is_empty(), "the key did not change");
    let changes: [fn(&mut proto::TaskInfo); 4] = [
        |task| task.state = TaskState::MergeQueue,
        |task| task.rounds[0].turns += 1,
        |task| task.merge_commit = Some("abcdef0123".into()),
        |task| task.reviews.clear(),
    ];
    for (n, change) in changes.into_iter().enumerate() {
        change_t2(&mut app, change);
        let sent = ticks(&mut app, 3);
        assert_eq!(sent.len(), 1, "change {n}: {sent:?}");
        // The cache is the newer request's until its reply lands.
        assert_eq!(state(&app), Some(&DetailState::InFlight(sent[0].0)));
        let _ = reply(&mut app, "t2", "the brief", sent[0].0);
    }
}

#[test]
fn the_matching_reply_fills_the_cache_and_another_is_dropped() {
    let mut app = gemini_view();
    let (id, _) = ticks(&mut app, 1)[0].clone();
    // Another id, and no id at all: dropped.
    assert!(reply(&mut app, "t2", "stale", id + 7).is_empty());
    let _ = app.on_daemon(DaemonMsg::Run(RunReply::TaskDetail {
        detail: detail("t2", "untagged"),
        request_id: None,
    }));
    assert_eq!(state(&app), Some(&DetailState::InFlight(id)));
    assert!(reply(&mut app, "t2", "the brief", id).is_empty());
    match state(&app) {
        Some(DetailState::Ready(detail)) => assert_eq!(detail.brief, "the brief"),
        other => panic!("{other:?}"),
    }
    // A repeat of the same id after it landed changes nothing.
    let _ = reply(&mut app, "t2", "late", id);
    assert!(matches!(state(&app), Some(DetailState::Ready(d)) if d.brief == "the brief"));
}

#[test]
fn a_refusal_is_shown_not_toasted() {
    let mut app = gemini_view();
    let (id, _) = ticks(&mut app, 1)[0].clone();
    let refused = |message: &str, request_id| {
        DaemonMsg::Run(RunReply::Refused {
            request: "run task-detail".into(),
            message: message.into(),
            request_id,
        })
    };
    // Another request's refusal still toasts.
    let _ = app.on_daemon(refused("something else", Some(id + 1)));
    assert_eq!(app.toast_text(), Some("something else"));
    app.toast = None;
    let _ = app.on_daemon(refused("\n no such task \x1b[31m\nmore", Some(id)));
    assert_eq!(
        app.toast_text(),
        None,
        "the detail's refusal is not toasted"
    );
    match state(&app) {
        Some(DetailState::Failed(text)) => assert!(
            text.starts_with("no such task") && !text.contains('\x1b') && !text.contains("more"),
            "its first line, one-lined: {text:?}"
        ),
        other => panic!("{other:?}"),
    }
    assert!(
        ticks(&mut app, 3).is_empty(),
        "a refusal is not retried per tick"
    );
}

#[test]
fn no_request_while_disconnected_or_with_the_panel_hidden() {
    let mut app = gemini_view();
    app.inspector_visible = false;
    assert!(ticks(&mut app, 3).is_empty(), "the panel is hidden");
    app.inspector_visible = true;
    let _ = app.on_link_lost("gone");
    assert!(ticks(&mut app, 3).is_empty(), "not connected");
    let _ = app.on_reconnected(app.windows.clone());
    assert_eq!(ticks(&mut app, 3).len(), 1, "asks once reconnected");
    // A run's node, not a task's: nothing.
    let mut app = gemini_view();
    let rows = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    app.tree.select(&rows, NodeKey::Run("r1".into()));
    assert!(ticks(&mut app, 3).is_empty(), "no task is selected");
}

#[test]
fn a_lost_link_clears_an_in_flight_request() {
    let mut app = gemini_view();
    let first = ticks(&mut app, 1);
    assert_eq!(first.len(), 1);
    let _ = app.on_link_lost("gone");
    assert_eq!(state(&app), None, "the reply went with the link");
    let _ = app.on_reconnected(app.windows.clone());
    let again = ticks(&mut app, 3);
    assert_eq!(again.len(), 1, "{again:?}");
    assert_ne!(again[0].0, first[0].0);
    // A landed detail survives a lost link.
    let _ = reply(&mut app, "t2", "kept", again[0].0);
    let _ = app.on_link_lost("gone");
    assert!(matches!(state(&app), Some(DetailState::Ready(d)) if d.brief == "kept"));
}

#[test]
fn a_refused_send_does_not_retry_every_tick() {
    let mut app = gemini_view();
    let effects = app.on_tick();
    let msg = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Send(msg @ ClientMsg::RunTagged { .. }) => Some(msg.clone()),
            _ => None,
        })
        .expect("a request");
    assert!(app.on_send_failed(&msg).is_empty());
    assert_eq!(app.toast_text(), None, "quiet");
    assert_eq!(
        state(&app),
        Some(&DetailState::Failed(
            "the detail request was not sent".into()
        ))
    );
    assert!(ticks(&mut app, 5).is_empty());
}

#[test]
fn page_keys_and_b_apply_only_to_the_selected_task() {
    let mut app = gemini_view();
    let id = ticks(&mut app, 1)[0].0;
    let long = (1..=60).map(|n| format!("line {n}")).collect::<Vec<_>>();
    let _ = reply(&mut app, "t2", &long.join("\n"), id);
    let none = KeyModifiers::NONE;
    assert!(!app.brief_expanded_for(&t("t2")));
    assert!(press(&mut app, KeyCode::Char('b'), none).is_empty());
    assert!(app.brief_expanded_for(&t("t2")));
    assert!(press(&mut app, KeyCode::PageDown, none).is_empty());
    let scrolled = app.inspector_scroll_for(&t("t2"));
    assert!(scrolled > 0, "PageDown scrolled");
    let (_, height) = app.task_panel_interior();
    assert_eq!(scrolled, height - 1, "a page is the interior less one");
    let _ = press(&mut app, KeyCode::PageDown, none);
    let _ = press(&mut app, KeyCode::PageUp, none);
    assert_eq!(app.inspector_scroll_for(&t("t2")), height - 1, "and back");
    // Many pages down stop at the end: the last body row is the panel's last row.
    for _ in 0..20 {
        let _ = press(&mut app, KeyCode::PageDown, none);
    }
    let end = app.inspector_scroll_for(&t("t2"));
    let (width, height) = app.task_panel_interior();
    let rows = crate::inspector::task_panel_rows(&app, width);
    assert_eq!(usize::from(end), rows - usize::from(height - 1));
    // Another task starts at the top with its brief collapsed.
    select(&mut app, "t0");
    assert_eq!(app.inspector_scroll_for(&t("t0")), 0);
    assert!(!app.brief_expanded_for(&t("t0")));
    // Coming back is a new selection too: at the top, collapsed (decision 25). Any
    // way back is a key, a click or a tick first; here a key on t0 (`x` is a no-op
    // for the panel) and then the selection returns.
    let _ = press(&mut app, KeyCode::Char('x'), none);
    select(&mut app, "t2");
    assert_eq!(app.inspector_scroll_for(&t("t2")), 0);
    assert!(!app.brief_expanded_for(&t("t2")));
    // A click or a tick drops them as well.
    for event in 0..2 {
        let _ = press(&mut app, KeyCode::Char('b'), none);
        let _ = press(&mut app, KeyCode::PageDown, none);
        select(&mut app, "t0");
        if event == 0 {
            let _ = app.on_tick();
        } else {
            let layout = crate::ui::layout(
                ratatui::layout::Rect::new(0, 0, 120, 40),
                app.sidebar_width,
                crate::app::alerts(&app).len(),
            );
            let _ = app.on_click(0, 39, &layout);
        }
        select(&mut app, "t2");
        assert_eq!(app.inspector_scroll_for(&t("t2")), 0, "event {event}");
        assert!(!app.brief_expanded_for(&t("t2")), "event {event}");
    }
    // PageUp from the top stays at the top; `b` twice collapses again.
    let _ = press(&mut app, KeyCode::PageUp, none);
    assert_eq!(app.inspector_scroll_for(&t("t2")), 0);
    let _ = press(&mut app, KeyCode::Char('b'), none);
    let _ = press(&mut app, KeyCode::Char('b'), none);
    assert!(!app.brief_expanded_for(&t("t2")));
}
