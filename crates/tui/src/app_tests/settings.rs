//! Task M6.9's `UiSettings` end to end: the configured prefix, scrollback limit, bell
//! toggles, default runtime, and the startup config-problems notice. Split out of
//! `app/tests.rs` per `AGENTS.md` hard rule 8 (task M6.10's file-size finding B) — the
//! same shape `app_tests/git.rs` and `app_tests/remove.rs` already give their own
//! tasks' tests.

use super::*;

/// Task M6.9: a configured prefix drives the keymap end to end. The easy half is that
/// the new prefix works; the half that matters to a user is that the *old* one, `C-b`,
/// stops being swallowed and reaches the PTY as the raw byte a shell expects for its own
/// `C-b` readline binding.
#[test]
fn custom_prefix_drives_the_keymap() {
    let settings = UiSettings {
        prefix: (KeyCode::Char('a'), KeyModifiers::CONTROL),
        prefix_label: "C-a".into(),
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)],
        "/tmp".into(),
        settings,
    );
    let _ = app.set_terminal_size(80, 24);

    assert!(press(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL).is_empty());
    assert_eq!(
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24
        })]
    );
    assert_eq!(app.focused, Some(2), "C-a j must switch windows");

    assert_eq!(
        press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 2,
            bytes: vec![0x02]
        })],
        "C-b is no longer the prefix and must reach the PTY as 0x02"
    );
}

#[test]
fn scrollback_follows_settings() {
    let settings = UiSettings {
        scrollback_lines: 10,
        ..UiSettings::default()
    };
    let mut app = App::new(vec![win(1, "a", Status::Idle)], "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);
    for i in 0..50 {
        app.parser.process(format!("line {i}\r\n").as_bytes());
    }
    let area = ratatui::layout::Rect::new(0, 0, 80, 24);
    let mut layout = crate::ui::layout(area, 0);
    layout.main_inner = area;
    // 34 wheel ticks of 3 lines each ask for 102 lines of scrollback; with
    // `scrollback_lines = 10` the parser can only ever retain 10.
    for _ in 0..34 {
        app.on_scroll(true, 5, 5, &layout);
    }
    assert_eq!(app.scroll_offset, 10);
}

/// `bell_attention` and `bell_done` gate `Effect::Bell` independently, and neither ever
/// rings for the focused window (matching the toast right beside it).
#[test]
fn bells_follow_settings() {
    let settings = UiSettings {
        bell_attention: true,
        bell_done: false,
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![win(1, "a", Status::Idle), win(2, "b", Status::Working)],
        "/tmp".into(),
        settings,
    );
    let _ = app.set_terminal_size(80, 24);
    assert_eq!(app.focused, Some(1));

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(1, "a", Status::Idle), win(2, "b", Status::Attention)],
    });
    assert!(
        effects.contains(&Effect::Bell),
        "a background window entering Attention must ring: {effects:?}"
    );

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(1, "a", Status::Idle), win(2, "b", Status::Done)],
    });
    assert!(
        !effects.contains(&Effect::Bell),
        "bell_done is false, so Done must not ring: {effects:?}"
    );

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(1, "a", Status::Attention), win(2, "b", Status::Done)],
    });
    assert!(
        !effects.contains(&Effect::Bell),
        "the focused window must never ring: {effects:?}"
    );
}

/// Decision 37's own last sentence: "It sends at most one bell per `WindowsChanged`."
/// Whole-branch-review Minor m1: `replace_windows` pushed one `Effect::Bell` per
/// transitioning background window inside its loop, so two background windows
/// transitioning in the same `WindowsChanged` rang twice — two `0x07` bytes written to
/// the real terminal for what decision 34's status line still shows as a single event.
/// `bells_follow_settings` above only ever asserts `contains(&Effect::Bell)`, which
/// passes identically whether one bell or five were queued, so it could not have caught
/// this.
#[test]
fn at_most_one_bell_per_windows_changed() {
    let settings = UiSettings {
        bell_attention: true,
        bell_done: true,
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![
            win(1, "focused", Status::Idle),
            win(2, "b", Status::Idle),
            win(3, "c", Status::Idle),
        ],
        "/tmp".into(),
        settings,
    );
    let _ = app.set_terminal_size(80, 24);
    assert_eq!(app.focused, Some(1));

    // Two background windows transition in the same batch: one to Attention, one to
    // Done. Both individually qualify for a bell (decision 4), but decision 37 caps
    // the *event*, not the window count.
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![
            win(1, "focused", Status::Idle),
            win(2, "b", Status::Attention),
            win(3, "c", Status::Done),
        ],
    });
    let bells = effects.iter().filter(|e| **e == Effect::Bell).count();
    assert_eq!(
        bells, 1,
        "at most one bell per WindowsChanged (decision 37): {effects:?}"
    );
}

#[test]
fn new_window_uses_the_default_runtime() {
    let settings = UiSettings {
        default_runtime: Runtime::Claude,
        ..UiSettings::default()
    };
    let mut app = App::new(vec![], "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);

    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE).is_empty());
    match &app.modal {
        Some(Modal::NewAgent(form)) => assert_eq!(form.runtime, Runtime::Claude),
        other => panic!("expected Modal::NewAgent, got {other:?}"),
    }
}

#[test]
fn config_problems_open_a_notice() {
    let mut app = App::new(
        vec![win(1, "a", Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    let _ = app.set_terminal_size(80, 24);
    app.report_config_problems(vec![
        "prefix: expected C- followed by a letter (using C-b)".into(),
    ]);
    assert!(matches!(app.modal, Some(Modal::Notice { .. })));

    let effects = press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
    assert!(effects.is_empty(), "{effects:?}");
    assert!(app.modal.is_none());

    let mut clean = App::new(vec![], "/tmp".into(), UiSettings::default());
    clean.report_config_problems(vec![]);
    assert!(clean.modal.is_none(), "no problems opens nothing");
}
