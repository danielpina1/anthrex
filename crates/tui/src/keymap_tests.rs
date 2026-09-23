use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn plain_and_control_characters() {
    assert_eq!(
        encode_key(key(KeyCode::Char('a'), KeyModifiers::NONE), false),
        Some(b"a".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('é'), KeyModifiers::NONE), false),
        Some("é".as_bytes().to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
        Some(vec![0x03])
    );
    assert_eq!(
        encode_key(
            key(
                KeyCode::Char('C'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT
            ),
            false
        ),
        Some(vec![0x03])
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('['), KeyModifiers::CONTROL), false),
        Some(vec![0x1b])
    );
    assert_eq!(
        encode_key(key(KeyCode::Char(' '), KeyModifiers::CONTROL), false),
        Some(vec![0x00])
    );
    assert_eq!(
        encode_key(key(KeyCode::Char('x'), KeyModifiers::ALT), false),
        Some(vec![0x1b, b'x'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Enter, KeyModifiers::NONE), false),
        Some(vec![b'\r'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Backspace, KeyModifiers::NONE), false),
        Some(vec![0x7f])
    );
    assert_eq!(
        encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
        Some(vec![0x1b])
    );
    assert_eq!(
        encode_key(key(KeyCode::Tab, KeyModifiers::NONE), false),
        Some(vec![b'\t'])
    );
    assert_eq!(
        encode_key(key(KeyCode::BackTab, KeyModifiers::SHIFT), false),
        Some(b"\x1b[Z".to_vec())
    );
}

/// I3: Alt+Enter is how claude and codex insert a newline without submitting; it must
/// reach them as ESC CR, not as a bare CR that submits the prompt.
#[test]
fn alt_prefixes_enter_tab_escape_and_backspace_with_escape() {
    assert_eq!(
        encode_key(key(KeyCode::Enter, KeyModifiers::ALT), false),
        Some(vec![0x1b, b'\r'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Tab, KeyModifiers::ALT), false),
        Some(vec![0x1b, b'\t'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Esc, KeyModifiers::ALT), false),
        Some(vec![0x1b, 0x1b])
    );
    assert_eq!(
        encode_key(key(KeyCode::Backspace, KeyModifiers::ALT), false),
        Some(vec![0x1b, 0x7f])
    );
    // Without Alt they are unchanged.
    assert_eq!(
        encode_key(key(KeyCode::Enter, KeyModifiers::NONE), false),
        Some(vec![b'\r'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Tab, KeyModifiers::NONE), false),
        Some(vec![b'\t'])
    );
    assert_eq!(
        encode_key(key(KeyCode::Esc, KeyModifiers::NONE), false),
        Some(vec![0x1b])
    );
}

#[test]
fn cursor_keys_honour_application_mode_and_modifiers() {
    assert_eq!(
        encode_key(key(KeyCode::Up, KeyModifiers::NONE), false),
        Some(b"\x1b[A".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Up, KeyModifiers::NONE), true),
        Some(b"\x1bOA".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Left, KeyModifiers::SHIFT), false),
        Some(b"\x1b[1;2D".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Right, KeyModifiers::CONTROL), true),
        Some(b"\x1b[1;5C".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Home, KeyModifiers::NONE), true),
        Some(b"\x1bOH".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::Delete, KeyModifiers::NONE), false),
        Some(b"\x1b[3~".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::PageUp, KeyModifiers::SHIFT), false),
        Some(b"\x1b[5;2~".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::F(1), KeyModifiers::NONE), false),
        Some(b"\x1bOP".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::F(5), KeyModifiers::NONE), false),
        Some(b"\x1b[15~".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::F(12), KeyModifiers::NONE), false),
        Some(b"\x1b[24~".to_vec())
    );
    assert_eq!(
        encode_key(key(KeyCode::CapsLock, KeyModifiers::NONE), false),
        None
    );
}

#[test]
fn prefix_then_command() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert!(km.pending());
    assert_eq!(
        km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::NextWindow)
    );
    assert!(!km.pending());
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('3'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::FocusIndex(2))
    );
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('X'), KeyModifiers::SHIFT), false),
        KeyAction::Run(Command::RemoveWindow)
    );
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('?'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::Help)
    );
}

#[test]
fn prefix_twice_sends_literal_and_escape_cancels() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    km.handle(prefix, false);
    assert_eq!(km.handle(prefix, false), KeyAction::Send(vec![0x02]));
    km.handle(prefix, false);
    assert_eq!(
        km.handle(key(KeyCode::Esc, KeyModifiers::NONE), false),
        KeyAction::Cancel
    );
    assert!(!km.pending());
    km.handle(prefix, false);
    assert_eq!(
        km.handle(key(KeyCode::Char('z'), KeyModifiers::NONE), false),
        KeyAction::Nothing
    );
}

#[test]
fn ordinary_keys_pass_through_and_releases_are_ignored() {
    let mut km = Keymap::new(Keymap::default_prefix());
    assert_eq!(
        km.handle(key(KeyCode::Char('q'), KeyModifiers::NONE), false),
        KeyAction::Send(b"q".to_vec())
    );
    let mut release = key(KeyCode::Char('q'), KeyModifiers::NONE);
    release.kind = crossterm::event::KeyEventKind::Release;
    assert_eq!(km.handle(release, false), KeyAction::Nothing);
}

#[test]
fn held_prefix_auto_repeat_keeps_waiting() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert!(km.pending());
    let mut repeat = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    repeat.kind = crossterm::event::KeyEventKind::Repeat;
    assert_eq!(km.handle(repeat, false), KeyAction::AwaitPrefix);
    assert!(km.pending());
    assert_eq!(
        km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::NextWindow)
    );
    assert!(!km.pending());
}

#[test]
fn prefix_opens_tree_overview_and_width_commands() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);

    for (key, command) in [
        (
            key(KeyCode::Char('t'), KeyModifiers::NONE),
            Command::ToggleTree,
        ),
        (
            key(KeyCode::Char('T'), KeyModifiers::NONE),
            Command::ToggleOverview,
        ),
        (
            key(KeyCode::Char('T'), KeyModifiers::SHIFT),
            Command::ToggleOverview,
        ),
        (
            key(KeyCode::Char('<'), KeyModifiers::NONE),
            Command::NarrowSidebar,
        ),
        (
            key(KeyCode::Char('>'), KeyModifiers::NONE),
            Command::WidenSidebar,
        ),
    ] {
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(key, false), KeyAction::Run(command));
    }
}

/// Task M6.10, decision 23: `,` renames, `R` restarts, `r` reconnects. `R` must
/// match whether or not crossterm also reports the shift modifier (compare
/// `prefix_opens_tree_overview_and_width_commands`'s `T` cases above).
#[test]
fn rename_restart_and_reconnect_keys() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);

    for (k, command) in [
        (
            key(KeyCode::Char(','), KeyModifiers::NONE),
            Command::RenameWindow,
        ),
        (
            key(KeyCode::Char('R'), KeyModifiers::SHIFT),
            Command::RestartWindow,
        ),
        (
            key(KeyCode::Char('R'), KeyModifiers::NONE),
            Command::RestartWindow,
        ),
        (
            key(KeyCode::Char('r'), KeyModifiers::NONE),
            Command::Reconnect,
        ),
    ] {
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(km.handle(k, false), KeyAction::Run(command));
    }
}

#[test]
fn tree_mode_returns_keys_instead_of_bytes() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    km.set_tree_mode(true);

    for tree_key in [
        key(KeyCode::Char('j'), KeyModifiers::NONE),
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Esc, KeyModifiers::NONE),
        key(KeyCode::Char('/'), KeyModifiers::NONE),
    ] {
        assert_eq!(km.handle(tree_key, false), KeyAction::Tree(tree_key));
    }

    let mut release = key(KeyCode::Char('j'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert_eq!(km.handle(release, false), KeyAction::Nothing);

    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::NextWindow)
    );
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(km.handle(prefix, false), KeyAction::Send(vec![0x02]));

    km.set_tree_mode(false);
    assert_eq!(
        km.handle(key(KeyCode::Char('j'), KeyModifiers::NONE), false),
        KeyAction::Send(b"j".to_vec())
    );
}

#[test]
fn the_conversation_key_is_m() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('m'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::ToggleConversation)
    );
}

#[test]
fn conversation_mode_routes_keys_to_the_view() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    km.set_conversation_mode(true);
    assert!(km.conversation_mode());

    for view_key in [
        key(KeyCode::Char('j'), KeyModifiers::NONE),
        key(KeyCode::Enter, KeyModifiers::NONE),
        key(KeyCode::Esc, KeyModifiers::NONE),
        key(KeyCode::Char('q'), KeyModifiers::NONE),
    ] {
        assert_eq!(
            km.handle(view_key, false),
            KeyAction::Conversation(view_key)
        );
    }
    // It wins over tree mode: the view covers the main area the keys would act on.
    km.set_tree_mode(true);
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Conversation(j));

    let mut release = j;
    release.kind = KeyEventKind::Release;
    assert_eq!(km.handle(release, false), KeyAction::Nothing);

    // The prefix still works, and a prefixed `m` still toggles.
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('m'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::ToggleConversation)
    );

    km.set_tree_mode(false);
    km.set_conversation_mode(false);
    assert!(!km.conversation_mode());
    assert_eq!(km.handle(j, false), KeyAction::Send(b"j".to_vec()));
}

/// Every key bound after the prefix, and what it produces. The spec's list of free
/// letters was stale when it was written (`C-b k` was already taken); this table is the
/// truth, so a new binding that shadowed an old one fails here.
#[test]
fn the_conversation_key_does_not_collide() {
    let bound: Vec<(KeyCode, KeyAction)> = vec![
        (KeyCode::Char('j'), KeyAction::Run(Command::NextWindow)),
        (KeyCode::Char('n'), KeyAction::Run(Command::NextWindow)),
        (KeyCode::Char('k'), KeyAction::Run(Command::PrevWindow)),
        (KeyCode::Char('p'), KeyAction::Run(Command::PrevWindow)),
        (KeyCode::Char('1'), KeyAction::Run(Command::FocusIndex(0))),
        (KeyCode::Char('5'), KeyAction::Run(Command::FocusIndex(4))),
        (KeyCode::Char('9'), KeyAction::Run(Command::FocusIndex(8))),
        (KeyCode::Char('c'), KeyAction::Run(Command::NewWindow)),
        (KeyCode::Char('x'), KeyAction::Run(Command::KillWindow)),
        (KeyCode::Char('X'), KeyAction::Run(Command::RemoveWindow)),
        (KeyCode::Char('s'), KeyAction::Run(Command::ToggleSidebar)),
        (KeyCode::Char('t'), KeyAction::Run(Command::ToggleTree)),
        (KeyCode::Char('T'), KeyAction::Run(Command::ToggleOverview)),
        (KeyCode::Char('<'), KeyAction::Run(Command::NarrowSidebar)),
        (KeyCode::Char('>'), KeyAction::Run(Command::WidenSidebar)),
        (KeyCode::Char('d'), KeyAction::Run(Command::Detach)),
        (KeyCode::Char('Q'), KeyAction::Run(Command::StopDaemon)),
        (KeyCode::Char(','), KeyAction::Run(Command::RenameWindow)),
        (KeyCode::Char('R'), KeyAction::Run(Command::RestartWindow)),
        (KeyCode::Char('r'), KeyAction::Run(Command::Reconnect)),
        (KeyCode::Char('?'), KeyAction::Run(Command::Help)),
        (
            KeyCode::Char('m'),
            KeyAction::Run(Command::ToggleConversation),
        ),
        (KeyCode::Esc, KeyAction::Cancel),
    ];
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    for (code, expected) in &bound {
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        assert_eq!(
            km.handle(key(*code, KeyModifiers::NONE), false),
            *expected,
            "prefix then {code:?}"
        );
    }

    // `m` is the only key that toggles the conversation: sweep every printable ASCII
    // character and every other code a terminal commonly sends.
    let mut codes: Vec<KeyCode> = (0x20u8..0x7f).map(|b| KeyCode::Char(b as char)).collect();
    codes.extend([
        KeyCode::Enter,
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Delete,
    ]);
    for code in codes {
        assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
        let action = km.handle(key(code, KeyModifiers::NONE), false);
        assert_eq!(
            action == KeyAction::Run(Command::ToggleConversation),
            code == KeyCode::Char('m'),
            "prefix then {code:?} gave {action:?}"
        );
        if let Some((_, expected)) = bound.iter().find(|(c, _)| *c == code) {
            assert_eq!(action, *expected, "prefix then {code:?}");
        }
    }
}
