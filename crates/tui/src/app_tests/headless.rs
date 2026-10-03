//! M8a.17, decision 49 in the TUI: a headless run session's window has no terminal, so
//! no path subscribes to it or forwards input to it. The prefix key still works, and the
//! pane points at the conversation view.

use super::*;
use crate::app::Link;

pub(super) fn headless(id: u32, name: &str, status: Status) -> WindowInfo {
    let mut window = win(id, name, status);
    window.runtime = Runtime::Codex;
    window.kind = proto::WindowKind::Headless;
    window.run = Some(proto::RunRef {
        run_id: "run-1".into(),
        task_id: Some("t1".into()),
        role: proto::AgentRole::Worker,
        session: 1,
        lane: None,
    });
    window
}

pub(super) fn subscribes(effects: &[Effect], id: u32) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Send(ClientMsg::Subscribe { window_id, .. }) if *window_id == id)
    })
}

pub(super) fn inputs(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|effect| matches!(effect, Effect::Send(ClientMsg::Input { .. })))
}

#[test]
fn focusing_a_headless_window_sends_no_subscribe() {
    let mut app = app_with(vec![
        win(1, "pty", Status::Idle),
        headless(2, "worker", Status::Working),
    ]);
    assert_eq!(app.focused, Some(1));
    let effects = app.focus(2);
    assert_eq!(app.focused, Some(2));
    assert!(!subscribes(&effects, 2), "{effects:?}");
    // The PTY window it left stops streaming to this client.
    assert!(
        effects.contains(&Effect::Send(ClientMsg::Unsubscribe)),
        "{effects:?}"
    );

    // Neither the retry of a dropped `Subscribe`...
    assert!(!subscribes(&app.on_tick(), 2));
    // ...nor a reconnect sends one.
    assert!(app.on_link_lost("x").is_empty());
    assert!(matches!(app.link, Link::Reconnecting { .. }));
    let effects = app.on_reconnected(vec![
        win(1, "pty", Status::Idle),
        headless(2, "worker", Status::Working),
    ]);
    assert!(!subscribes(&effects, 2), "{effects:?}");
    assert!(!subscribes(&app.on_tick(), 2));

    // Back on the PTY window, it subscribes as ever.
    assert!(subscribes(&app.focus(1), 1));
}

#[test]
fn a_headless_window_first_in_the_list_is_focused_without_a_subscribe() {
    let mut app = App::new(
        vec![headless(3, "worker", Status::Working)],
        "/tmp".into(),
        UiSettings::default(),
    );
    let effects = app.set_terminal_size(80, 24);
    assert_eq!(app.focused, Some(3));
    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.on_tick().is_empty());
}

#[test]
fn keys_are_not_forwarded_to_a_headless_window() {
    let mut app = app_with(vec![
        win(1, "pty", Status::Idle),
        headless(2, "worker", Status::Idle),
    ]);
    let effects = press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(
        inputs(&effects),
        "a PTY window still gets its keys: {effects:?}"
    );

    app.focus(2);
    for code in [KeyCode::Char('x'), KeyCode::Enter, KeyCode::Char('c')] {
        let effects = press(&mut app, code, KeyModifiers::NONE);
        assert!(!inputs(&effects), "{code:?}: {effects:?}");
    }
    let effects = press(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert!(!inputs(&effects), "{effects:?}");
    assert!(app.on_paste("rm -rf /".into()).is_empty());
}

#[test]
fn mouse_input_is_not_forwarded_to_a_headless_window() {
    let area = ratatui::layout::Rect::new(0, 0, 120, 30);
    let layout = crate::ui::layout(area, 34, 0);
    let mut app = app_with(vec![
        win(1, "pty", Status::Idle),
        headless(2, "worker", Status::Idle),
    ]);
    let (x, y) = (layout.main_inner.x + 1, layout.main_inner.y + 1);
    // A PTY window that asked for mouse reports gets the wheel.
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    assert!(inputs(&app.on_scroll(true, x, y, &layout)));

    app.focus(2);
    // Whatever a previous screen left in the parser, nothing reaches the session.
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    assert!(!inputs(&app.on_scroll(true, x, y, &layout)));
    assert!(!inputs(&app.on_scroll(false, x, y, &layout)));
}

#[test]
fn prefix_commands_still_work_on_a_headless_window() {
    let mut app = app_with(vec![headless(2, "worker", Status::Idle)]);
    assert_eq!(app.focused, Some(2));
    let effects = super::conversation::toggle(&mut app);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::SubscribeConversation {
            window_id: 2,
            agent_id: None,
            from_rev: None,
        })]
    );
    assert!(app.conversation.is_open());
}

#[test]
fn the_headless_placeholder_names_the_conversation_key() {
    let app = app_with(vec![headless(2, "worker", Status::Working)]);
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 20)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, &app);
        })
        .unwrap();
    let screen = terminal.backend().to_string();
    assert!(
        screen.contains("headless session · codex · working · C-b m shows its conversation"),
        "{screen}"
    );
}

/// M8c.6, decision 26: `C-b x`, `C-b X` and `C-b R` on a focused headless window open no
/// dialog and send nothing; they toast the daemon's own refusal.
#[test]
fn control_commands_on_a_headless_window_open_nothing() {
    use crate::tree::run_fixtures::three_task_fixture;
    let (_, windows) = three_task_fixture();
    // No snapshot: window 6 is a plain row, reachable by `C-b <n>`.
    let mut app = app_with(windows);
    let index = tree::agent_order(&app.rows())
        .iter()
        .position(|id| *id == 6)
        .expect("window 6 has a position");
    prefix(&mut app);
    let digit = char::from_digit(index as u32 + 1, 10).expect("a digit");
    press(&mut app, KeyCode::Char(digit), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(6));

    let refusal = "window 6 is a headless session of run add-reset-3f9a; only the engine drives it. Use anthrex run cancel to stop it";
    let control = |app: &mut App, c: char, what: &str| {
        app.toast = None;
        prefix(app);
        let effects = press(app, KeyCode::Char(c), KeyModifiers::NONE);
        assert!(effects.is_empty(), "{what}: {effects:?}");
        assert_eq!(app.modal, None, "{what}");
        app.toast_text().map(str::to_owned)
    };
    for (c, what) in [('x', "kill"), ('X', "remove"), ('R', "restart")] {
        assert_eq!(
            control(&mut app, c, what).as_deref(),
            Some(refusal),
            "{what}"
        );
    }
    let mut exited = app.windows.clone();
    exited
        .iter_mut()
        .filter(|w| w.id == 6)
        .for_each(|w| w.status = Status::Exited);
    let _ = app.on_daemon(DaemonMsg::WindowsChanged { windows: exited });
    assert_eq!(app.focused, Some(6));
    assert_eq!(
        control(&mut app, 'R', "restart, exited").as_deref(),
        Some(refusal)
    );

    // A run-less headless window (M8b's onboarding scout) gets the scout variant.
    let mut scout = headless(7, "scout", Status::Working);
    scout.run = None;
    let mut app = app_with(vec![scout]);
    assert_eq!(app.focused, Some(7));
    let scout_refusal = "window 7 is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it";
    for (c, what) in [('x', "kill"), ('X', "remove"), ('R', "restart")] {
        assert_eq!(
            control(&mut app, c, what).as_deref(),
            Some(scout_refusal),
            "{what}"
        );
    }

    // A PTY window still gets its dialogs.
    let mut app = app_with(vec![win(1, "pty", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
    app.modal = None;
    prefix(&mut app);
    press(&mut app, KeyCode::Char('X'), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::Remove(_))));
    app.modal = None;
    prefix(&mut app);
    press(&mut app, KeyCode::Char('R'), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
}

/// Whole-branch review M1: a sub-agent of a headless window listed as a plain window
/// (no run in the snapshot names it) opens that window's conversation on Enter. It
/// never focuses the window or leaves tree mode.
#[test]
fn enter_on_a_plain_listed_headless_windows_sub_agent_opens_its_conversation() {
    let mut worker = headless(2, "worker", Status::Working);
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
    let mut app = app_with(vec![win(1, "pty", Status::Idle), worker]);
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.run_view, None);
    app.enter_tree();
    let key = crate::tree::NodeKey::Subagent {
        window_id: 2,
        id: "a1".into(),
    };
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, key.clone());
    assert_eq!(app.tree.selected, Some(key));

    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::SubscribeConversation {
            window_id: 2,
            agent_id: None,
            from_rev: None,
        })]
    );
    assert_eq!(app.focused, Some(1), "Enter focused the headless window");
    assert!(app.conversation.is_open());
    assert!(!subscribes(&effects, 2));
    assert!(!inputs(&effects));
}

/// Milestone 9 decision 11: `C-b x` and `C-b X` on the orchestrator of a run this client
/// shows as not terminal open nothing and toast the daemon's refusal; `C-b R` still
/// opens its dialog. Once the run is terminal, or when the client does not show the
/// run, the dialogs open as for any PTY window.
#[test]
fn kill_and_remove_of_a_live_orchestrator_open_nothing() {
    use super::runs::{app_with_runs, deliver, run_info, snapshot};
    let mut orchestrator = win(5, "7a2c/orchestrator", Status::Idle);
    orchestrator.run = Some(proto::RunRef {
        run_id: "r-7a2c".into(),
        task_id: None,
        role: proto::AgentRole::Orchestrator,
        session: 1,
        lane: None,
    });
    let mut app = app_with_runs(
        vec![orchestrator.clone()],
        snapshot(1, 0, vec![run_info("r-7a2c")]),
    );
    assert_eq!(app.focused, Some(5));
    let refusal = "window 5 is the orchestrator of run r-7a2c; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart 5";
    let control = |app: &mut App, c: char| {
        app.toast = None;
        app.modal = None;
        prefix(app);
        let effects = press(app, KeyCode::Char(c), KeyModifiers::NONE);
        (effects, app.toast_text().map(str::to_owned))
    };
    for c in ['x', 'X'] {
        let (effects, toast) = control(&mut app, c);
        assert!(effects.is_empty(), "{c}: {effects:?}");
        assert_eq!(app.modal, None, "{c}");
        assert_eq!(toast.as_deref(), Some(refusal), "{c}");
    }
    control(&mut app, 'R');
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })), "C-b R");

    // Terminal: an ordinary PTY window again.
    let mut done = run_info("r-7a2c");
    done.state = proto::RunState::Accepted;
    deliver(&mut app, snapshot(2, 0, vec![done]));
    control(&mut app, 'x');
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })), "C-b x");
    control(&mut app, 'X');
    assert!(matches!(app.modal, Some(Modal::Remove(_))), "C-b X");

    // A run the client does not show: the daemon decides.
    let mut app = app_with(vec![orchestrator]);
    control(&mut app, 'x');
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })));
}
