//! Milestone 9.5 decision 44 (FU-F7): a placeholder headless window (milestone 9
//! decision 11a, restored from a record that did not parse) can be killed and removed
//! from the TUI, as the daemon already allows.

use super::headless::headless;
use super::*;

/// A run-less headless window, as a placeholder or an onboarding scout lists.
fn run_less(id: u32, placeholder: bool) -> WindowInfo {
    let mut window = headless(id, "lost", Status::Exited);
    window.run = None;
    window.placeholder = placeholder;
    window
}

#[test]
fn a_placeholder_window_can_be_closed() {
    let mut app = app_with(vec![run_less(7, true)]);
    assert_eq!(app.focused, Some(7));
    // `C-b x`: the kill dialog opens and nothing is toasted; `y` sends the kill.
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('x'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.toast_text(), None);
    assert!(matches!(app.modal, Some(Modal::Confirm { .. })), "C-b x");
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Kill { window_id: 7 })]
    );
    // `C-b X`: the remove dialog opens and `y` sends the removal.
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('X'), KeyModifiers::SHIFT).is_empty());
    assert_eq!(app.toast_text(), None);
    assert!(matches!(app.modal, Some(Modal::Remove(_))), "C-b X");
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 7,
            remove_worktree: false,
            force: false,
        })]
    );

    // Without `placeholder` the scout refusal stays, and nothing is sent.
    let mut app = app_with(vec![run_less(7, false)]);
    let scout_refusal = "window 7 is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it";
    for c in ['x', 'X'] {
        app.toast = None;
        prefix(&mut app);
        assert!(press(&mut app, KeyCode::Char(c), KeyModifiers::NONE).is_empty());
        assert_eq!(app.modal, None, "{c}");
        assert_eq!(app.toast_text(), Some(scout_refusal), "{c}");
    }
}
