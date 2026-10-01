//! Milestone 9.0.5's two bare-key modes, the plan review's and the Alerts box's, and
//! their precedence (Risks 1). Split from `keymap_tests.rs` to keep it under 600 lines.

use super::*;

/// Milestone 9.0.5 decision 13: in review mode bare keys go to the review, never to a
/// PTY, and the prefix (and `C-b a`) still works.
#[test]
fn review_mode_routes_bare_keys_to_the_review() {
    let mut km = Keymap::new(Keymap::default_prefix());
    km.set_review_mode(true);
    assert!(km.review_mode());
    for code in [
        KeyCode::Char('j'),
        KeyCode::Char('a'),
        KeyCode::Esc,
        KeyCode::PageDown,
        KeyCode::Enter,
    ] {
        let k = key(code, KeyModifiers::NONE);
        assert_eq!(km.handle(k, false), KeyAction::Review(k));
    }
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('a'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::FocusAlerts)
    );
    // The prefix twice sends no literal byte: the review is not a PTY.
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(km.handle(prefix, false), KeyAction::Nothing);
    km.set_review_mode(false);
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Send(b"j".to_vec()));
}

/// Risks 1: the review wins over the conversation view and the tree.
#[test]
fn review_mode_wins_over_conversation_and_tree_modes() {
    let mut km = Keymap::new(Keymap::default_prefix());
    km.set_tree_mode(true);
    km.set_conversation_mode(true);
    km.set_review_mode(true);
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Review(j));
}

/// `C-b a` is `FocusAlerts` in every mode.
#[test]
fn prefix_a_focuses_the_alerts() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('a'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::FocusAlerts)
    );
}

/// Milestone 9.0.5 decision 21: in alerts mode bare keys go to the box; the prefix still
/// works, and the prefix twice sends nothing.
#[test]
fn alerts_mode_routes_bare_keys_to_the_box() {
    let mut km = Keymap::new(Keymap::default_prefix());
    km.set_alerts_mode(true);
    assert!(km.alerts_mode());
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Alerts(j));
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('t'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::ToggleTree)
    );
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(km.handle(prefix, false), KeyAction::Nothing);
}

/// Risks 1: review, then alerts, then conversation, then tree.
#[test]
fn alerts_mode_sits_between_the_review_and_the_conversation() {
    let mut km = Keymap::new(Keymap::default_prefix());
    km.set_tree_mode(true);
    km.set_conversation_mode(true);
    km.set_alerts_mode(true);
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Alerts(j));
    km.set_review_mode(true);
    assert_eq!(km.handle(j, false), KeyAction::Review(j));
}

/// Milestone 9.0.6 decision 33: `C-b P` opens the Profile screen; in screen mode bare
/// keys go to the screen, which wins over every other mode; the prefix still works and
/// twice sends nothing.
#[test]
fn screen_mode_routes_bare_keys_to_the_screen() {
    let mut km = Keymap::new(Keymap::default_prefix());
    let prefix = key(KeyCode::Char('b'), KeyModifiers::CONTROL);
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('P'), KeyModifiers::SHIFT), false),
        KeyAction::Run(Command::OpenProfile)
    );
    km.set_tree_mode(true);
    km.set_conversation_mode(true);
    km.set_alerts_mode(true);
    km.set_review_mode(true);
    km.set_screen_mode(true);
    assert!(km.screen_mode());
    let j = key(KeyCode::Char('j'), KeyModifiers::NONE);
    assert_eq!(km.handle(j, false), KeyAction::Screen(j));
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(
        km.handle(key(KeyCode::Char('t'), KeyModifiers::NONE), false),
        KeyAction::Run(Command::ToggleTree)
    );
    assert_eq!(km.handle(prefix, false), KeyAction::AwaitPrefix);
    assert_eq!(km.handle(prefix, false), KeyAction::Nothing);
    km.set_screen_mode(false);
    assert_eq!(km.handle(j, false), KeyAction::Review(j));
}
