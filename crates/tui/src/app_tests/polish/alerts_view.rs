//! M9.0.7.6: the Alerts view's keys (decision 11), reducer tests over
//! `ui/alerts_fixture.rs`'s `three_runs` (the gate of `docs-77aa`, `t2` of
//! `add-mul-0723` blocked on a question, `fix-ci-9b1e` complete) and task 8's
//! `every_source`.

use super::super::alerts::every_app;
use super::super::runs::deliver;
use super::super::*;
use crate::actions_request::ActionTarget;
use crate::app::alerts_view::{alert_actions, preselected};
use crate::app::region::KeyRegion;
use crate::app::{AlertKey, alerts};
use crate::tree::NodeKey;
use crate::ui::alerts::fixture::{three_runs, three_runs_snapshot};
use proto::{ActionKind, RunState};

fn chord(app: &mut App, c: char) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char(c), KeyModifiers::NONE)
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

fn selected(app: &App) -> Option<AlertKey> {
    app.alerts_focus.as_ref().and_then(|f| f.selected.clone())
}

fn gate() -> AlertKey {
    AlertKey::Gate("docs-77aa".into())
}

fn t2() -> AlertKey {
    AlertKey::Blocked {
        run: "add-mul-0723".into(),
        task: "t2".into(),
    }
}

/// `C-b a`, then `j` until `key` is selected.
fn view_on(app: &mut App, key: &AlertKey) {
    assert!(chord(app, 'a').is_empty());
    assert_eq!(app.key_region(), KeyRegion::Alerts);
    for _ in 0..20 {
        if selected(app).as_ref() == Some(key) {
            return;
        }
        tap(app, KeyCode::Char('j'));
    }
    panic!("{key:?} is not listed");
}

/// The open menu's run, node and selected entry's kind.
fn menu(app: &App) -> (String, ActionTarget, ActionKind) {
    match &app.modal {
        Some(Modal::Action(flow)) => (
            flow.run_id.clone(),
            flow.target.clone(),
            flow.items[flow.selected].kind.clone(),
        ),
        other => panic!("no menu: {other:?}"),
    }
}

#[test]
fn c_b_a_opens_the_view_on_the_first_alert() {
    // The sidebar's visibility is unchanged either way: the view is in the main pane.
    for shown in [true, false] {
        let mut app = three_runs();
        app.sidebar_visible = shown;
        assert!(chord(&mut app, 'a').is_empty());
        assert_eq!(app.sidebar_visible, shown);
        assert_eq!(app.key_region(), KeyRegion::Alerts);
        assert_eq!(selected(&app), Some(gate()));
        let focus = app.alerts_focus.as_ref().unwrap();
        assert_eq!((focus.at, focus.scroll), (0, 0));
    }
    // Under the plan review it is refused, as before.
    let mut app = three_runs();
    app.open_plan_review("docs-77aa".into(), ReviewTarget::Gate);
    assert!(chord(&mut app, 'a').is_empty());
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.toast_text(), Some("leave the plan review first (esc)"));
    // Pinning (`app/screens.rs`): a full-body screen keeps its own refusal.
    use crate::app::screens::Screen;
    use crate::app::stats::{StatsScreen, StatsState};
    let mut profile = three_runs();
    chord(&mut profile, 'P');
    let mut settings = three_runs();
    chord(&mut settings, 'S');
    let mut stats = three_runs();
    stats.set_screen(Some(Screen::Stats(Box::new(StatsScreen {
        project: "/tmp/repo".into(),
        state: StatsState::Failed("no history".into()),
        scroll: 0,
        request: 1,
    }))));
    for (mut app, toast) in [
        (profile, "leave the profile first (esc)"),
        (settings, "leave the settings first (esc)"),
        (stats, "leave the stats first (esc)"),
    ] {
        assert!(app.screen.is_some(), "{toast}");
        assert!(chord(&mut app, 'a').is_empty());
        assert_eq!(app.alerts_focus, None, "{toast}");
        assert_eq!(app.toast_text(), Some(toast));
    }
}

/// Pinning `alerts_focus.rs::enter_on_each_priority`, through the view: Enter runs the
/// preselection `enter_alert` runs, and leaves the view.
#[test]
fn enter_runs_the_preselection() {
    // P1: the orchestrator's window, tree mode off.
    let mut app = every_app();
    chord(&mut app, 't');
    view_on(&mut app, &AlertKey::Orchestrator("b-gate".into()));
    assert_eq!(
        preselected(&app, &AlertKey::Orchestrator("b-gate".into())),
        None
    );
    let effects = tap(&mut app, KeyCode::Enter);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Send(ClientMsg::Subscribe { window_id: 12, .. }), ..]
        ),
        "{effects:?}"
    );
    assert_eq!((app.focused, app.tree_input), (Some(12), None));
    assert_eq!(app.alerts_focus, None);
    assert!(!app.keymap.alerts_mode());
    // P2 to P4: the menu on the alert's node; with no daemon entries listed the
    // preselection falls back to the first entry, as `enter_on_each_priority` pins.
    let hold = AlertKey::Hold {
        run: "c-held".into(),
        hold: "epic:ui".into(),
    };
    let blocked = AlertKey::Blocked {
        run: "d-bare".into(),
        task: "t1".into(),
    };
    let cases = [
        (
            AlertKey::Gate("b-gate".into()),
            ActionTarget::Run,
            ActionKind::ReviewPlan,
        ),
        (
            hold.clone(),
            ActionTarget::Run,
            ActionKind::ApproveHold {
                hold: "epic:ui".into(),
            },
        ),
        (blocked, ActionTarget::Task("t1".into()), ActionKind::Answer),
        (
            AlertKey::Halted("e-halt".into()),
            ActionTarget::Run,
            ActionKind::Resume,
        ),
        (
            AlertKey::Accept("f-done".into()),
            ActionTarget::Run,
            ActionKind::Accept,
        ),
    ];
    for (key, target, kind) in cases {
        let mut app = every_app();
        view_on(&mut app, &key);
        assert_eq!(preselected(&app, &key), Some(kind), "{key:?}");
        assert!(tap(&mut app, KeyCode::Enter).is_empty(), "{key:?}");
        let Some(Modal::Action(flow)) = &app.modal else {
            panic!("{key:?}: no menu");
        };
        assert_eq!((&flow.target, flow.selected), (&target, 0), "{key:?}");
        assert_eq!(app.alerts_focus, None, "{key:?}");
        assert_eq!(app.run_view, None, "{key:?}");
        assert!(app.plan_review.is_none(), "{key:?}");
    }
    // With the daemon's entries listed, the preselection is selected: `t2`'s answer.
    let mut app = three_runs();
    view_on(&mut app, &t2());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        menu(&app),
        (
            "add-mul-0723".into(),
            ActionTarget::Task("t2".into()),
            ActionKind::Answer
        )
    );
    // A proposal opens the Profile screen.
    let mut app = every_app();
    view_on(&mut app, &AlertKey::Proposal("/r/shop".into()));
    assert_eq!(
        tap(&mut app, KeyCode::Enter).len(),
        3,
        "status and both shows"
    );
    assert!(app.screen.is_some());
    assert_eq!(app.alerts_focus, None);
}

#[test]
fn dot_opens_the_menu_with_no_preselection() {
    let mut app = three_runs();
    view_on(&mut app, &t2());
    assert!(tap(&mut app, KeyCode::Char('.')).is_empty());
    let items = match &app.modal {
        Some(Modal::Action(flow)) => {
            assert_eq!(flow.selected, 0, "the first entry");
            flow.items
                .iter()
                .map(|a| a.label.clone())
                .collect::<Vec<_>>()
        }
        other => panic!("no menu: {other:?}"),
    };
    // The menu's own order: the client's `open conversation` leads (decision 10).
    assert_eq!(items[0], "open conversation");
    assert_eq!(app.alerts_focus, None, "the view is left");
    assert!(!app.keymap.alerts_mode());
    // The gate's menu: on the run.
    let mut app = three_runs();
    view_on(&mut app, &gate());
    tap(&mut app, KeyCode::Char('.'));
    let (run, node, kind) = menu(&app);
    assert_eq!(
        (run.as_str(), node, kind),
        ("docs-77aa", ActionTarget::Run, ActionKind::ReviewPlan)
    );
    // The row the detail lists: the daemon's entries, then the client's.
    let labels: Vec<String> = alert_actions(&three_runs(), &t2())
        .into_iter()
        .map(|a| a.label)
        .collect();
    assert_eq!(
        labels,
        [
            "answer",
            "message",
            "retry",
            "override",
            "cancel task",
            "open conversation"
        ]
    );
}

#[test]
fn m_preselects_message_only_where_listed() {
    let mut app = three_runs();
    view_on(&mut app, &t2());
    assert!(tap(&mut app, KeyCode::Char('m')).is_empty());
    assert_eq!(
        menu(&app),
        (
            "add-mul-0723".into(),
            ActionTarget::Task("t2".into()),
            ActionKind::Message
        )
    );
    assert_eq!(app.alerts_focus, None);
    // A gate alert lists no `Message`: nothing happens, the view stays.
    let mut app = three_runs();
    view_on(&mut app, &gate());
    assert!(tap(&mut app, KeyCode::Char('m')).is_empty());
    assert!(app.modal.is_none());
    assert_eq!(selected(&app), Some(gate()));
    // A refused `Message` is not offered either.
    let mut snap = three_runs_snapshot();
    let t2_info = &mut snap[1].tasks[1];
    t2_info.actions[1].refused_why = Some("the task is not running".into());
    let mut app = crate::ui::alerts::fixture::app_of(
        vec![crate::tree::run_fixtures::pty(
            1,
            "shell",
            "/tmp/repo",
            Status::Idle,
        )],
        snap,
    );
    view_on(&mut app, &t2());
    assert!(tap(&mut app, KeyCode::Char('m')).is_empty());
    assert!(app.modal.is_none());
    assert_eq!(selected(&app), Some(t2()));
}

#[test]
fn o_opens_the_run_view_on_the_task() {
    let mut app = three_runs();
    view_on(&mut app, &t2());
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(app.alerts_focus, None);
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("add-mul-0723")
    );
    assert_eq!(
        app.tree.selected,
        Some(NodeKey::Task {
            run: "add-mul-0723".into(),
            id: "t2".into()
        })
    );
    assert_eq!(app.key_region(), KeyRegion::Overview);
    // A gate: the run view on its root.
    let mut app = three_runs();
    view_on(&mut app, &gate());
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("docs-77aa")
    );
    assert_eq!(app.tree.selected, Some(NodeKey::Run("docs-77aa".into())));
    // Over the conversation, which closes so the run view shows.
    let mut app = three_runs();
    let _ = app.open_conversation(1);
    view_on(&mut app, &gate());
    tap(&mut app, KeyCode::Char('o'));
    assert!(!app.conversation.is_open());
    assert_eq!(app.key_region(), KeyRegion::Overview);
    // A proposal: the Profile screen, as Enter.
    let mut app = every_app();
    view_on(&mut app, &AlertKey::Proposal("/r/shop".into()));
    assert_eq!(tap(&mut app, KeyCode::Char('o')).len(), 3);
    assert!(app.screen.is_some());
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.run_view, None);
}

#[test]
fn page_keys_scroll_the_detail_and_esc_leaves() {
    let mut app = three_runs();
    // A long question: the detail no longer fits a 120x14 frame.
    let mut snap = three_runs_snapshot();
    let long = (1..=30)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    snap[1].tasks[1].block.as_mut().unwrap().text = long;
    deliver(
        &mut app,
        crate::tree::run_fixtures::snapshot(crate::ui::alerts::fixture::NOW, snap),
    );
    app.set_graph_viewport(ratatui::layout::Rect::new(34, 0, 86, 13));
    view_on(&mut app, &t2());
    let scroll = |app: &App| app.alerts_focus.as_ref().unwrap().scroll;
    assert_eq!(scroll(&app), 0);
    tap(&mut app, KeyCode::PageDown);
    let page = scroll(&app);
    assert!(page > 0, "PgDn scrolled");
    for _ in 0..20 {
        tap(&mut app, KeyCode::PageDown);
    }
    let end = scroll(&app);
    assert!(end > page, "{end} after {page}");
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(scroll(&app), end, "the end holds");
    tap(&mut app, KeyCode::PageUp);
    assert_eq!(scroll(&app), end - page);
    for _ in 0..20 {
        tap(&mut app, KeyCode::PageUp);
    }
    assert_eq!(scroll(&app), 0);
    // A move starts the next alert's detail at its top.
    tap(&mut app, KeyCode::PageDown);
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(scroll(&app), 0);
    // Esc leaves; the pane has the keys again.
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.key_region(), KeyRegion::Pane);
}

#[test]
fn view_keys_send_no_input() {
    let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
    codes.extend([
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Delete,
        KeyCode::Esc,
        KeyCode::F(1),
    ]);
    for start in [gate(), t2()] {
        for code in &codes {
            for mods in [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::ALT] {
                let mut app = three_runs();
                view_on(&mut app, &start);
                let effects = press(&mut app, *code, mods);
                assert!(
                    !effects
                        .iter()
                        .any(|e| matches!(e, Effect::Send(ClientMsg::Input { .. }))),
                    "{start:?} {code:?} {mods:?}: {effects:?}"
                );
            }
        }
    }
}

/// Pinning `repair_alerts_focus`: the view follows its alert, and a resolved one gives
/// way to the alert now at its position.
#[test]
fn the_view_follows_a_resolved_alert() {
    let mut app = three_runs();
    view_on(&mut app, &t2());
    // The gate approved: `t2` moves up one, the view stays on it.
    let mut snap = three_runs_snapshot();
    snap[0].state = RunState::Running;
    deliver(
        &mut app,
        crate::tree::run_fixtures::snapshot(crate::ui::alerts::fixture::NOW, snap.clone()),
    );
    assert_eq!(selected(&app), Some(t2()));
    assert_eq!(app.alerts_focus.as_ref().unwrap().at, 0);
    // `t2` answered: the complete run's alert takes its place.
    snap[1].tasks[1].state = proto::TaskState::Working;
    snap[1].tasks[1].block = None;
    deliver(
        &mut app,
        crate::tree::run_fixtures::snapshot(crate::ui::alerts::fixture::NOW, snap),
    );
    assert_eq!(selected(&app), Some(AlertKey::Accept("fix-ci-9b1e".into())));
    assert_eq!(alerts(&app).len(), 1);
    assert_eq!(app.key_region(), KeyRegion::Alerts);
}

/// Decision 11: the view covers the main pane, so no click, drag or wheel reaches what
/// is under it — the terminal's program, the overview's graph.
#[test]
fn no_click_or_wheel_reaches_the_main_pane_under_the_view() {
    use ratatui::layout::Rect;
    let area = Rect::new(0, 0, 120, 40);
    // The wheel over the terminal: a program that asked for the mouse gets the report
    // once the view is closed, never while it is open.
    let mut app = three_runs();
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    chord(&mut app, 'a');
    let layout = crate::ui::layout_for(&app, area);
    let (x, y) = (layout.main_inner.x + 5, layout.main_inner.y + 5);
    assert_eq!(app.on_scroll(false, x, y, &layout), vec![]);
    assert_eq!(app.on_click(x, y, &layout), vec![]);
    assert_eq!(app.scroll_offset, 0);
    tap(&mut app, KeyCode::Esc);
    assert!(matches!(
        app.on_scroll(false, x, y, &layout).as_slice(),
        [Effect::Send(ClientMsg::Input { .. })]
    ));
    // A click on a node of the overview's graph selects it once the view is closed,
    // and changes nothing while the view is open.
    let mut app = three_runs();
    chord(&mut app, 'T');
    let layout = crate::ui::layout_for(&app, area);
    app.set_graph_viewport(layout.main);
    let buffer = crate::ui::audit::draw(&app, 120, 40);
    let node = crate::ui::audit::find(&buffer, "run 9b1e")
        .into_iter()
        .find(|&(x, y)| layout.main.contains((x, y).into()))
        .unwrap_or_else(|| panic!("{}", crate::ui::audit::rows(&buffer).join("\n")));
    let before = app.tree.selected.clone();
    chord(&mut app, 'a');
    assert_eq!(app.on_click(node.0, node.1, &layout), vec![]);
    assert_eq!(app.on_drag(node.0 + 3, node.1 + 2, &layout), vec![]);
    assert_eq!(app.on_scroll(false, node.0, node.1, &layout), vec![]);
    assert_eq!(app.tree.selected, before);
    tap(&mut app, KeyCode::Esc);
    app.on_click(node.0, node.1, &layout);
    assert_eq!(app.tree.selected, Some(NodeKey::Run("fix-ci-9b1e".into())));
}
