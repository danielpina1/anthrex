//! M8c.6: Enter in the run view (decisions 24 and 25), a headless window in the plain
//! tree (decision 26), and the guard that no run-view path offers a headless window
//! input, a terminal or a control command.

use super::run_view::{gate, keys, run_key, select_nav, three};
use super::*;
use crate::tree::run_fixtures::{RUN_ID, headless, run_ref, three_task_fixture};
use proto::AgentRole;

use super::super::headless::{inputs, subscribes};
use super::super::runs::{app_with_runs, open_run_view};

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

fn conversation_of(window_id: u32) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::SubscribeConversation {
        window_id,
        agent_id: None,
        from_rev: None,
    })]
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

fn round_key(task: &str, role: AgentRole, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: task.into(),
        role,
        lane: None,
        session,
        round,
    }
}

#[test]
fn enter_on_the_root_focuses_the_orchestrator() {
    let mut app = three();
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    let effects = tap(&mut app, KeyCode::Enter);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Send(ClientMsg::Subscribe { window_id: 3, .. })]
        ),
        "{effects:?}"
    );
    assert_eq!(app.focused, Some(3));
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert!(!app.overview);
    assert_eq!(app.run_view, None);
}

#[test]
fn enter_on_the_root_without_an_orchestrator_toasts() {
    let mut app = gate();
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.toast_text(),
        Some(
            "run add-reset-3f9a has no orchestrator window; Enter on an agent opens its conversation"
        )
    );
    assert!(app.run_view.is_some());
    assert_eq!(app.focused, Some(1));
    assert!(!app.conversation.is_open());
}

#[test]
fn enter_on_a_round_opens_its_conversation_without_focusing() {
    let mut app = three();
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    let round = round_key("t1", AgentRole::Worker, 1, 1);
    select_nav(&mut app, round.clone());
    app.conversation_follow = Some(1);
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(effects, conversation_of(6));
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.conversation_follow, None);
    assert!(app.keymap.conversation_mode());
    assert!(app.run_view.is_some());
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));

    // M6.5's `q` closes it, and the run view is where it was.
    let effects = tap(&mut app, KeyCode::Char('q'));
    assert!(!app.conversation.is_open(), "{effects:?}");
    assert!(!app.keymap.conversation_mode());
    assert_eq!(app.nav_rows()[0].key, run_key());
    assert!(app.nav_rows().len() > app.rows().len());
    assert_eq!(app.tree.selected, Some(round));
}

#[test]
fn a_run_view_conversation_follows_focus_like_any_other() {
    let mut app = three();
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    let round = round_key("t1", AgentRole::Worker, 1, 1);
    select_nav(&mut app, round.clone());
    assert_eq!(tap(&mut app, KeyCode::Enter), conversation_of(6));

    // `C-b <n>` focuses the orchestrator, a second PTY window: the view follows focus.
    let index = tree::agent_order(&app.rows())
        .iter()
        .position(|id| *id == 3)
        .expect("the orchestrator has a position");
    prefix(&mut app);
    let digit = char::from_digit(index as u32 + 1, 10).expect("a digit");
    let effects = tap(&mut app, KeyCode::Char(digit));
    assert_eq!(app.focused, Some(3));
    assert!(
        effects.contains(&conversation_of(3)[0]),
        "the view re-opens on the focused window: {effects:?}"
    );
    assert_eq!(app.conversation.window_id(), Some(3));

    // Closing it lands in the run view, on the same node.
    tap(&mut app, KeyCode::Char('q'));
    assert!(!app.conversation.is_open());
    assert!(app.run_view.is_some());
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(app.tree.selected, Some(round));
}

#[test]
fn enter_on_a_task_opens_its_live_rounds_conversation() {
    let mut app = three();
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, task_key("t1"));
    assert_eq!(tap(&mut app, KeyCode::Enter), conversation_of(6));
    assert_eq!(app.focused, Some(1));

    // A live round wins over a finished one whose window is still listed, even an
    // earlier session's.
    let (mut snapshot, mut windows) = three_task_fixture();
    let mut first = crate::tree::run_fixtures::worker(1, Some(8), proto::Runtime::Claude, 9_600);
    first.ended_at = Some(9_650);
    let t1 = &mut snapshot.runs[0].tasks[1];
    t1.rounds[0].session = 2;
    t1.rounds[0].round = 2;
    t1.rounds.push(first);
    windows.push(headless(
        8,
        "3f9a/t1.w1",
        crate::tree::run_fixtures::PROJECT,
        Some(run_ref(RUN_ID, Some("t1"), AgentRole::Worker, 1)),
    ));
    let mut app = app_with_runs(windows, snapshot);
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, task_key("t1"));
    assert_eq!(tap(&mut app, KeyCode::Enter), conversation_of(6));

    // With no live round, the latest round whose window is still listed: `t0`'s
    // reviewer, retired but not yet removed.
    let (snapshot, mut windows) = three_task_fixture();
    windows.push(headless(
        5,
        "3f9a/t0.r1",
        crate::tree::run_fixtures::PROJECT,
        Some(run_ref(RUN_ID, Some("t0"), AgentRole::Reviewer, 1)),
    ));
    windows.push(headless(
        4,
        "3f9a/t0.w1",
        crate::tree::run_fixtures::PROJECT,
        Some(run_ref(RUN_ID, Some("t0"), AgentRole::Worker, 1)),
    ));
    let mut app = app_with_runs(windows, snapshot);
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, task_key("t0"));
    assert_eq!(tap(&mut app, KeyCode::Enter), conversation_of(5));
}

#[test]
fn enter_on_a_task_with_no_agent_toasts() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, task_key("t2"));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("t2 has no agent yet"));
    assert!(!app.conversation.is_open());

    // `t0`'s rounds ended and their windows are gone: nothing to open either.
    select_nav(&mut app, task_key("t0"));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("t0 has no agent yet"));
}

#[test]
fn enter_on_a_round_whose_window_is_not_listed_yet() {
    let (mut snapshot, windows) = three_task_fixture();
    snapshot.runs[0].tasks[1].rounds[0].window_id = Some(99);
    let mut app = app_with_runs(windows, snapshot);
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, round_key("t1", AgentRole::Worker, 1, 1));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("window #99 is not listed yet"));
    assert!(!app.conversation.is_open());

    select_nav(&mut app, round_key("t0", AgentRole::Worker, 1, 1));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("worker #1 has finished and its window is gone")
    );
    assert!(!app.conversation.is_open());
    assert_eq!(app.focused, Some(1));
}

/// A headless window of a run the snapshot does not name (review focus 5).
fn plain_headless() -> App {
    let mut windows = vec![
        crate::tree::run_fixtures::pty(1, "shell", "/r/demo", proto::Status::Idle),
        crate::tree::run_fixtures::pty(2, "api", "/r/demo", proto::Status::Idle),
        headless(
            7,
            "gone/t1.w1",
            "/r/demo",
            Some(run_ref("gone-0001", Some("t1"), AgentRole::Worker, 1)),
        ),
    ];
    windows[2].status = proto::Status::Working;
    let mut app = app_with(windows);
    let _ = app.focus(1);
    app
}

fn assert_opened_seven(app: &mut App, effects: Vec<Effect>) {
    assert_eq!(effects, conversation_of(7));
    assert!(!subscribes(&effects, 7));
    assert_eq!(app.focused, Some(1));
    assert!(app.conversation.is_open());
    let closed = tap(app, KeyCode::Char('q'));
    assert!(!app.conversation.is_open(), "{closed:?}");
}

#[test]
fn enter_on_a_headless_window_in_the_plain_tree_opens_its_conversation() {
    let window = NodeKey::Window(7);
    // The sidebar tree.
    let mut app = plain_headless();
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('t'));
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, window.clone());
    let effects = tap(&mut app, KeyCode::Enter);
    assert_opened_seven(&mut app, effects);

    // The project overview.
    let mut app = plain_headless();
    assert!(toggle(&mut app).is_empty());
    select(&mut app, window.clone());
    let effects = tap(&mut app, KeyCode::Enter);
    assert_opened_seven(&mut app, effects);
    assert!(app.overview);

    // A canvas double click.
    let layout = crate::ui::layout(
        Rect::new(0, 0, 120, 30),
        app.sidebar_width,
        crate::app::alerts(&app).len(),
    );
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    let (x, y) = box_middle(&app, layout.main, &window);
    assert!(app.on_click(x, y, &layout).is_empty());
    let effects = app.on_click(x, y, &layout);
    assert_opened_seven(&mut app, effects);

    // A sidebar click, outside tree mode.
    let mut app = plain_headless();
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    let index = tree::row_index(&app.rows(), &window).expect("a sidebar row");
    let effects = app.on_click(
        layout.sidebar_list.x + 2,
        layout.sidebar_list.y + index as u16,
        &layout,
    );
    assert_opened_seven(&mut app, effects);

    // A PTY window's Enter still focuses it.
    let mut app = plain_headless();
    assert!(toggle(&mut app).is_empty());
    select(&mut app, NodeKey::Window(2));
    let effects = tap(&mut app, KeyCode::Enter);
    assert!(subscribes(&effects, 2), "{effects:?}");
    assert_eq!(app.focused, Some(2));
}

/// Every effect is allowed except input anywhere, and a terminal subscription or a
/// control command naming a headless window.
fn assert_watch_only(app: &App, effects: &[Effect], what: &str) {
    assert!(!inputs(effects), "{what}: {effects:?}");
    for effect in effects {
        let named = match effect {
            Effect::Send(ClientMsg::Subscribe { window_id, .. })
            | Effect::Send(ClientMsg::Kill { window_id })
            | Effect::Send(ClientMsg::Restart { window_id }) => Some(*window_id),
            Effect::Send(ClientMsg::Remove { window_id, .. }) => Some(*window_id),
            _ => None,
        };
        if let Some(id) = named {
            assert!(!app.is_headless(id), "{what}: {effect:?}");
        }
    }
}

/// Back into the run view from wherever the last key left the client.
fn reopen(app: &mut App, log: &mut Vec<Effect>) {
    if app.modal.is_some() {
        log.extend(tap(app, KeyCode::Esc));
    }
    // Bounded: a keymap that sent `q` elsewhere would otherwise loop for ever (a
    // precedence mutant hung the whole test binary here, milestone 9.0.5 review).
    for _ in 0..16 {
        if !app.conversation.is_open() {
            break;
        }
        log.extend(tap(app, KeyCode::Char('q')));
    }
    assert!(
        !app.conversation.is_open(),
        "`q` did not close the conversation"
    );
    if app.tree_input == Some(TreeInput::Filter) {
        log.extend(tap(app, KeyCode::Esc));
    }
    // Keep every node reachable for the next key: no fold, no filter. Cleared first,
    // since a sidebar click on the project leaves the view and folds it (review I1).
    app.tree.collapsed.clear();
    if app.run_view.is_none() {
        open_run_view(app, RUN_ID);
    }
    if let Some(view) = app.run_view.as_mut() {
        view.filter = crate::tree::RunFilter::All;
    }
}

#[test]
fn no_run_view_path_sends_input() {
    let mut app = three();
    let _ = app.focus(1);
    let layout = crate::ui::layout(
        Rect::new(0, 0, 120, 40),
        app.sidebar_width,
        crate::app::alerts(&app).len(),
    );
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    let mut log = Vec::new();
    let mut codes: Vec<KeyCode> = "axedfhljki/"
        .chars()
        .map(KeyCode::Char)
        .chain([KeyCode::Enter, KeyCode::Char(' '), KeyCode::Esc])
        .collect();
    codes.extend(
        ('a'..='z')
            .chain('A'..='Z')
            .chain("0123456789?!".chars())
            .map(KeyCode::Char),
    );
    let mut opened_a_conversation = false;
    for code in &codes {
        // Each key on every node of the run view, the root last (Enter there leaves).
        reopen(&mut app, &mut log);
        let count = app.nav_rows().len();
        for index in (0..count).rev() {
            reopen(&mut app, &mut log);
            let key = keys(&app.nav_rows())[index.min(app.nav_rows().len() - 1)].clone();
            if key == run_key() && *code == KeyCode::Enter {
                continue;
            }
            select_nav(&mut app, key.clone());
            let effects = tap(&mut app, *code);
            assert_watch_only(&app, &effects, &format!("{code:?} on {key:?}"));
            opened_a_conversation |= app.conversation.is_open();
            // A key typed into whatever that opened is watch-only too.
            let effects = tap(&mut app, KeyCode::Char('y'));
            assert_watch_only(&app, &effects, &format!("y after {code:?} on {key:?}"));
            log.extend(effects);
        }
    }
    assert!(
        opened_a_conversation,
        "Enter reached an agent's conversation"
    );

    // Every gesture over every node: a click, a double click, the wheel, a drag.
    let mut double_click_opened = false;
    reopen(&mut app, &mut log);
    for key in keys(&app.nav_rows()) {
        if key == run_key() {
            continue;
        }
        reopen(&mut app, &mut log);
        let view = overview::view(&app, layout.main);
        let Some(node) = view.layout.node(&key) else {
            continue;
        };
        let rect = node.rect;
        if rect.y < view.pan.y || rect.x < view.pan.x {
            continue;
        }
        let (x, y) = (
            view.canvas.x + rect.x + rect.width / 2 - view.pan.x,
            view.canvas.y + rect.y + 1 - view.pan.y,
        );
        if !view.canvas.contains((x, y).into()) {
            continue;
        }
        let mut effects = app.on_click(x, y, &layout);
        effects.extend(app.on_click(x, y, &layout));
        double_click_opened |= app.conversation.is_open();
        effects.extend(app.on_scroll(true, x, y, &layout));
        effects.extend(app.on_scroll(false, x, y, &layout));
        effects.extend(app.on_drag(x + 1, y + 1, &layout));
        assert_watch_only(&app, &effects, &format!("mouse on {key:?}"));
    }
    assert!(double_click_opened, "a double click reached a conversation");
    // And every sidebar row.
    for index in 0..app.rows().len() {
        reopen(&mut app, &mut log);
        let (x, y) = (
            layout.sidebar_list.x + 2,
            layout.sidebar_list.y + index as u16,
        );
        let mut effects = app.on_click(x, y, &layout);
        effects.extend(app.on_scroll(true, x, y, &layout));
        assert_watch_only(&app, &effects, &format!("sidebar row {index}"));
    }
    assert_watch_only(&app, &log, "the whole session");
}

/// Review m3 (M33, M34, M37): a planner has ended once it leaves `planning`, a scout
/// once it has reported or failed, whatever `ended_at` says; either with no window id
/// yet says so.
#[test]
fn planner_and_scout_enter_rules() {
    use crate::tree::run_fixtures::{planner, scout};
    use proto::{PlannerState, Runtime, ScoutState};
    let planner_key = NodeKey::Planner {
        run: RUN_ID.into(),
        epic: "A".into(),
    };
    let scout_key = NodeKey::Scout {
        run: RUN_ID.into(),
        id: "s1".into(),
    };
    let cases = [
        (PlannerState::Planning, ScoutState::Working, Some(98)),
        (PlannerState::Finished, ScoutState::Reported, Some(98)),
        (PlannerState::Failed, ScoutState::Failed, Some(98)),
        (PlannerState::Planning, ScoutState::Starting, None),
    ];
    for (planner_state, scout_state, window) in cases {
        let (mut snapshot, windows) = three_task_fixture();
        let mut a = planner("A", "daemon");
        a.state = planner_state;
        a.window_id = window;
        let mut s1 = scout("s1", "where is auth?", Runtime::Claude, 9_000);
        s1.state = scout_state;
        s1.window_id = window.map(|id| id - 1);
        snapshot.runs[0].planners = vec![a];
        snapshot.runs[0].scouts = vec![s1];
        let mut app = app_with_runs(windows, snapshot);
        let _ = app.focus(1);
        open_run_view(&mut app, RUN_ID);

        let ended = planner_state != PlannerState::Planning;
        select_nav(&mut app, planner_key.clone());
        assert!(tap(&mut app, KeyCode::Enter).is_empty());
        let expected = match window {
            None => "planner A has no window yet".to_owned(),
            Some(_) if ended => "planner A has finished and its window is gone".to_owned(),
            Some(id) => format!("window #{id} is not listed yet"),
        };
        assert_eq!(
            app.toast_text(),
            Some(expected.as_str()),
            "{planner_state:?}"
        );

        let ended = matches!(scout_state, ScoutState::Reported | ScoutState::Failed);
        select_nav(&mut app, scout_key.clone());
        assert!(tap(&mut app, KeyCode::Enter).is_empty());
        let expected = match window {
            None => "scout s1 has no window yet".to_owned(),
            Some(_) if ended => "scout s1 has finished and its window is gone".to_owned(),
            Some(id) => format!("window #{} is not listed yet", id - 1),
        };
        assert_eq!(app.toast_text(), Some(expected.as_str()), "{scout_state:?}");
        assert!(!app.conversation.is_open());
        assert_eq!(app.focused, Some(1));
    }

    // A round with no window id yet (the 200-task run has none).
    let mut app = app_with_runs(
        vec![],
        crate::tree::run_fixtures::snapshot(
            10_000,
            vec![crate::tree::run_fixtures::two_hundred_task_run()],
        ),
    );
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, round_key("t0", AgentRole::Worker, 1, 1));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("worker #1 has no window yet"));
}

/// Review m3 (M2): a sub-agent in the run view opens its owning window's conversation,
/// whatever kind of window that is; it never focuses it.
#[test]
fn enter_on_a_sub_agent_in_the_run_view_opens_its_windows_conversation() {
    for kind in [proto::WindowKind::Headless, proto::WindowKind::Pty] {
        let (snapshot, mut windows) = three_task_fixture();
        let worker = windows.iter_mut().find(|w| w.id == 6).expect("window 6");
        worker.kind = kind;
        worker.subagents = vec![proto::SubagentInfo {
            id: "a1".into(),
            parent_id: None,
            kind: "general-purpose".into(),
            label: Some("explore".into()),
            model: None,
            state: proto::SubagentState::Running,
            tool: None,
            started_secs: 5,
            ended_secs: None,
            needs_permission: false,
        }];
        let mut app = app_with_runs(windows, snapshot);
        let _ = app.focus(1);
        open_run_view(&mut app, RUN_ID);
        select_nav(
            &mut app,
            NodeKey::Subagent {
                window_id: 6,
                id: "a1".into(),
            },
        );
        let effects = tap(&mut app, KeyCode::Enter);
        assert_eq!(effects, conversation_of(6), "{kind:?}");
        assert_eq!(app.focused, Some(1), "{kind:?}");
        assert!(app.run_view.is_some());
        assert!(!inputs(&effects));
    }
}
