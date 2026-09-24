//! M8a.17, decision 49 in the TUI: a headless run session's window has no terminal, so
//! no path subscribes to it or forwards input to it. The prefix key still works, and the
//! pane points at the conversation view.

use super::*;
use crate::app::Link;

fn headless(id: u32, name: &str, status: Status) -> WindowInfo {
    let mut window = win(id, name, status);
    window.runtime = Runtime::Codex;
    window.kind = proto::WindowKind::Headless;
    window.run = Some(proto::RunRef {
        run_id: "run-1".into(),
        task_id: Some("t1".into()),
        role: proto::AgentRole::Worker,
        session: 1,
    });
    window
}

fn subscribes(effects: &[Effect], id: u32) -> bool {
    effects.iter().any(|effect| {
        matches!(effect, Effect::Send(ClientMsg::Subscribe { window_id, .. }) if *window_id == id)
    })
}

fn inputs(effects: &[Effect]) -> bool {
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
    let layout = crate::ui::layout(area, 34);
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
