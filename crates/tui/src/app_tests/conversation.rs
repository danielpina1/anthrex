//! Task M6.5.12: `C-b m` and the conversation view's keys, through `App::on_key`.

use super::*;

fn toggle(app: &mut App) -> Vec<Effect> {
    prefix(app);
    press(app, KeyCode::Char('m'), KeyModifiers::NONE)
}

#[test]
fn toggling_the_view_subscribes_and_unsubscribes() {
    let mut app = app_with(project_windows());
    assert_eq!(app.focused, Some(2));

    assert_eq!(
        toggle(&mut app),
        vec![Effect::Send(ClientMsg::SubscribeConversation {
            window_id: 2,
            agent_id: None,
            from_rev: None,
        })]
    );
    assert!(app.keymap.conversation_mode());
    assert!(app.conversation.is_open());

    assert_eq!(
        toggle(&mut app),
        vec![Effect::Send(ClientMsg::UnsubscribeConversation {
            window_id: 2,
            agent_id: None,
        })]
    );
    assert!(!app.keymap.conversation_mode());
    assert!(!app.conversation.is_open());
}

#[test]
fn the_view_does_not_open_without_a_focused_window() {
    let mut app = app_with(vec![]);
    assert_eq!(app.focused, None);
    assert!(toggle(&mut app).is_empty());
    assert_eq!(app.toast_text(), Some("no window focused"));
    assert!(!app.keymap.conversation_mode());
    assert!(!app.conversation.is_open());
}

#[test]
fn bare_keys_go_to_the_view_and_q_leaves_conversation_mode() {
    let mut app = app_with(project_windows());
    toggle(&mut app);
    // `j` moves the view's cursor; nothing reaches the PTY.
    assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE).is_empty());
    assert_eq!(
        press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::UnsubscribeConversation {
            window_id: 2,
            agent_id: None,
        })]
    );
    assert!(!app.keymap.conversation_mode());
    assert!(!app.conversation.is_open());
    // Keys reach the focused window again.
    assert_eq!(
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 2,
            bytes: b"j".to_vec(),
        })]
    );
}
