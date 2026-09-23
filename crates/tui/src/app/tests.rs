use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DaemonMsg, Runtime, Status};

// These stay under the old `crates/tui/src/app_tests/` directory (task M6.9's `git mv`
// only moves `app.rs` and `app_tests.rs` itself); the path is relative to this file's own
// directory, `crates/tui/src/app/`, hence `../`.
#[path = "../app_tests/tree_mode.rs"]
mod tree_mode;

#[path = "../app_tests/tree_interaction.rs"]
mod tree_interaction;

#[path = "../app_tests/overview.rs"]
mod overview;

#[path = "../app_tests/git.rs"]
mod git;

#[path = "../app_tests/dialog.rs"]
mod dialog;

#[path = "../app_tests/remove.rs"]
mod remove;

#[path = "../app_tests/lifecycle.rs"]
mod lifecycle;

#[path = "../app_tests/settings.rs"]
mod settings_tests;

#[path = "../app_tests/reconnect.rs"]
mod reconnect;

#[path = "../app_tests/conversation.rs"]
mod conversation;

#[path = "../app_tests/conversation_follow.rs"]
mod conversation_follow;

fn win(id: u32, name: &str, status: Status) -> WindowInfo {
    WindowInfo {
        id,
        name: name.into(),
        runtime: Runtime::Shell,
        cwd: "/tmp".into(),
        project: "/tmp".into(),
        worktree: None,
        branch: None,
        status,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
        kind: proto::WindowKind::Pty,
        run: None,
    }
}

fn project_win(id: u32, project: &str) -> WindowInfo {
    let mut window = win(id, &format!("window-{id}"), Status::Idle);
    window.cwd = project.into();
    window.project = project.into();
    window
}

fn project_windows() -> Vec<WindowInfo> {
    vec![
        project_win(1, "/p/b"),
        project_win(2, "/p/a"),
        project_win(3, "/p/b"),
    ]
}

fn app_with(windows: Vec<WindowInfo>) -> App {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    // The renderer reports the size on the first draw; simulate that.
    let _ = app.set_terminal_size(80, 24);
    app
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

fn press(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
    app.on_key(key(code, mods))
}

fn prefix(app: &mut App) {
    assert!(press(app, KeyCode::Char('b'), KeyModifiers::CONTROL).is_empty());
}

#[test]
fn first_size_report_subscribes_to_the_first_window() {
    let mut app = App::new(
        vec![win(4, "a", Status::Idle), win(5, "b", Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    assert_eq!(app.focused, None);
    let effects = app.set_terminal_size(100, 30);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 4,
            cols: 100,
            rows: 30
        })]
    );
    assert_eq!(app.focused, Some(4));
    assert_eq!(app.parser.screen().size(), (30, 100));
}

#[test]
fn focus_requested_before_the_first_size_report_subscribes_once_sized() {
    let mut app = App::new(
        vec![win(4, "a", Status::Idle), win(5, "b", Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    assert!(app.focus(5).is_empty());
    assert_eq!(app.focused, None);
    assert!(
        app.on_daemon(DaemonMsg::Created { window_id: 5 })
            .is_empty()
    );
    assert_eq!(app.focused, None);
    let effects = app.set_terminal_size(100, 30);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 5,
            cols: 100,
            rows: 30
        })]
    );
    assert_eq!(app.focused, Some(5));
}

#[test]
fn later_size_changes_are_debounced_into_a_resize() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert!(app.set_terminal_size(120, 40).is_empty());
    assert!(
        app.on_tick().is_empty(),
        "resize is not sent before the debounce window"
    );
    std::thread::sleep(RESIZE_DEBOUNCE + Duration::from_millis(5));
    assert_eq!(
        app.on_tick(),
        vec![Effect::Send(ClientMsg::Resize {
            window_id: 1,
            cols: 120,
            rows: 40
        })]
    );
    assert!(app.on_tick().is_empty());
}

#[test]
fn snapshot_and_output_feed_the_focused_parser_only() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    app.on_daemon(DaemonMsg::Snapshot {
        window_id: 1,
        cols: 80,
        rows: 24,
        bytes: b"hello".to_vec(),
    });
    assert!(app.parser.screen().contents().starts_with("hello"));
    app.on_daemon(DaemonMsg::Output {
        window_id: 2,
        bytes: b"IGNORED".to_vec(),
    });
    assert!(!app.parser.screen().contents().contains("IGNORED"));
    app.on_daemon(DaemonMsg::Output {
        window_id: 1,
        bytes: b" world".to_vec(),
    });
    assert!(app.parser.screen().contents().starts_with("hello world"));
}

#[test]
fn keys_go_to_the_focused_window_and_prefix_switches() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    assert_eq!(
        press(&mut app, KeyCode::Char('l'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"l".to_vec()
        })]
    );
    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24
        })]
    );
    assert_eq!(app.focused, Some(2));
    prefix(&mut app);
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(1), "next wraps around");
    prefix(&mut app);
    press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(2));
    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('d'), KeyModifiers::NONE),
        vec![Effect::Quit]
    );
}

#[test]
fn next_and_previous_follow_the_tree_order() {
    let mut app = app_with(project_windows());
    app.focus(2);

    for expected in [1, 3, 2] {
        prefix(&mut app);
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
        assert_eq!(app.focused, Some(expected));
    }

    prefix(&mut app);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(3));
}

#[test]
fn number_keys_use_visible_positions() {
    let mut app = app_with(project_windows());

    prefix(&mut app);
    press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(2));
    prefix(&mut app);
    press(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(3));

    assert!(app.tree.toggle(&tree::NodeKey::Project("/p/a".into())));
    prefix(&mut app);
    press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(1));
}

#[test]
fn first_focus_is_the_first_visible_window() {
    let mut app = App::new(project_windows(), "/tmp".into(), UiSettings::default());

    assert_eq!(
        app.set_terminal_size(100, 30),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 100,
            rows: 30,
        })]
    );
    assert_eq!(app.focused, Some(2));
}

#[test]
fn next_from_a_hidden_focused_window_goes_forward() {
    let mut app = app_with(project_windows());
    app.focus(1);
    assert!(app.tree.toggle(&tree::NodeKey::Project("/p/b".into())));

    prefix(&mut app);
    press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(2));

    assert!(app.tree.toggle(&tree::NodeKey::Project("/p/b".into())));
    app.focus(3);
    assert!(app.tree.toggle(&tree::NodeKey::Project("/p/b".into())));
    prefix(&mut app);
    press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
    assert_eq!(app.focused, Some(2));

    assert!(app.tree.toggle(&tree::NodeKey::Project("/p/a".into())));
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE).is_empty());
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.focused, Some(2));
}

/// I8: re-focusing the focused window must not resubscribe or reset the parser.
#[test]
fn focusing_the_already_focused_window_does_nothing() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    assert_eq!(app.focused, Some(1));
    app.on_daemon(DaemonMsg::Snapshot {
        window_id: 1,
        cols: 80,
        rows: 24,
        bytes: b"kept".to_vec(),
    });
    assert!(
        app.focus(1).is_empty(),
        "no Subscribe for the window we are already on"
    );
    assert!(
        app.parser.screen().contents().starts_with("kept"),
        "the screen must survive"
    );
    // Only a real change still subscribes.
    assert_eq!(
        app.focus(2),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24
        })]
    );
    // C-b 2 on the window already focused is likewise a no-op.
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('2'), KeyModifiers::NONE).is_empty());
}

#[test]
fn kill_asks_for_confirmation_first() {
    let mut app = app_with(vec![win(1, "a", Status::Working)]);
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm {
            action: PendingAction::Kill(1),
            ..
        })
    ));
    assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());
    prefix(&mut app);
    press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Kill { window_id: 1 })]
    );
    // Task M6.10, decision 36: `C-b Q` no longer quits immediately on `y`/`Enter` — see
    // `lifecycle::stop_daemon_waits_for_confirmation` for the full three-path coverage of
    // what it does instead.
    prefix(&mut app);
    press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Shutdown)]
    );
}

#[test]
fn removed_focused_window_moves_focus_to_a_neighbour() {
    let mut app = app_with(vec![
        win(1, "a", Status::Idle),
        win(2, "b", Status::Idle),
        win(3, "c", Status::Idle),
    ]);
    app.focus(2);
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(1, "a", Status::Idle), win(3, "c", Status::Idle)],
    });
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 3,
            cols: 80,
            rows: 24
        })]
    );
    let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
    assert!(effects.is_empty());
    assert_eq!(app.focused, None);
}

#[test]
fn removing_the_focused_window_focuses_the_same_position() {
    let mut app = app_with(project_windows());
    app.focus(1);
    assert!(app.tree.toggle(&tree::NodeKey::Window(1)));
    let rows = tree::build(&app.windows, &app.tree);
    app.tree.select(&rows, tree::NodeKey::Window(1));

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![project_win(2, "/p/a"), project_win(3, "/p/b")],
    });
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 3,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(3));
    assert!(!app.tree.collapsed.contains(&tree::NodeKey::Window(1)));
    assert_eq!(app.tree.selected, Some(tree::NodeKey::Window(3)));

    let mut app = app_with(project_windows());
    app.focus(3);
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![project_win(1, "/p/b"), project_win(2, "/p/a")],
    });
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 1,
            cols: 80,
            rows: 24,
        })]
    );
    assert_eq!(app.focused, Some(1));
}

#[test]
fn background_status_changes_raise_toasts() {
    let mut app = app_with(vec![
        win(1, "a", Status::Working),
        win(2, "b", Status::Working),
    ]);
    app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![
            win(1, "a", Status::Attention),
            win(2, "b", Status::Attention),
        ],
    });
    assert_eq!(
        app.toast_text(),
        Some("b needs attention"),
        "the focused window (1) never toasts"
    );
    app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(1, "a", Status::Attention), win(2, "b", Status::Done)],
    });
    assert_eq!(app.toast_text(), Some("b finished"));
    app.on_daemon(DaemonMsg::Error {
        request: "kill".into(),
        message: "no window with id 7".into(),
    });
    assert_eq!(app.toast_text(), Some("no window with id 7"));
}

#[test]
fn paste_uses_bracketed_mode_when_the_program_asked_for_it() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert_eq!(
        app.on_paste("ab\ncd".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ab\rcd".to_vec()
        })]
    );
    app.parser.process(b"\x1b[?2004h");
    assert_eq!(
        app.on_paste("x".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~x\x1b[201~".to_vec()
        })]
    );
}

#[test]
fn paste_strips_bracketed_paste_markers() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert_eq!(
        app.on_paste("a\x1b[201~b\x1b[200~c".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"abc".to_vec()
        })]
    );
    assert_eq!(
        app.on_paste("a\x1b[20\x1b[201~1~b".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ab".to_vec()
        })],
        "removing an embedded marker must not manufacture a new end marker"
    );

    app.parser.process(b"\x1b[?2004h");
    assert_eq!(
        app.on_paste("a\x1b[201~b\x1b[200~c".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~abc\x1b[201~".to_vec()
        })]
    );
    assert_eq!(
        app.on_paste("a\x1b[20\x1b[201~1~b".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[200~ab\x1b[201~".to_vec()
        })]
    );
}

#[test]
fn wheel_scrolls_the_local_scrollback_and_any_key_snaps_back() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    for i in 0..40 {
        app.parser.process(format!("line {i}\r\n").as_bytes());
    }
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let mut layout = crate::ui::layout(area, 0);
    layout.main_inner = area;
    assert!(app.on_scroll(true, 5, 5, &layout).is_empty());
    assert_eq!(app.scroll_offset, 3);
    press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);
    assert_eq!(app.scroll_offset, 0);
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    let effects = app.on_scroll(false, 5, 5, &layout);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[<65;6;6M".to_vec()
        })]
    );
}

// Task M6.9's settings tests (custom prefix, scrollback, bells, default runtime, the
// startup config-problems notice) live in `app_tests/settings.rs`; task M6.10's rename,
// restart and quit tests live in `app_tests/lifecycle.rs` (both declared above, per
// `AGENTS.md` hard rule 8).
