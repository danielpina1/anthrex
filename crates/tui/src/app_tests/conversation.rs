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

fn empty_conversation(
    window_id: u32,
    agent_id: Option<&str>,
    rev: u64,
    text: &str,
) -> proto::Conversation {
    proto::Conversation {
        window_id,
        agent_id: agent_id.map(str::to_owned),
        session_id: None,
        runtime: Runtime::Shell,
        rev,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
        turns: vec![proto::Turn {
            id: rev * 10,
            role: proto::Role::Assistant,
            at_unix_secs: 0,
            state: proto::TurnState::Complete,
            blocks: vec![
                proto::Block::Text { text: text.into() },
                proto::Block::SubagentSpawn {
                    agent_id: format!("agent-of-{rev}"),
                    kind: "Explore".into(),
                    label: format!("label-{rev}"),
                    model: None,
                },
            ],
        }],
    }
}

fn snapshot(window_id: u32, agent_id: Option<&str>, rev: u64, text: &str) -> DaemonMsg {
    DaemonMsg::ConversationSnapshot {
        window_id,
        agent_id: agent_id.map(str::to_owned),
        conversation: empty_conversation(window_id, agent_id, rev, text),
    }
}

/// Task M6.5.13: the daemon's `ConversationSnapshot` reaches the view through
/// `App::on_daemon`; one for another window is ignored.
#[test]
fn a_snapshot_for_the_open_view_is_applied() {
    let mut app = app_with(project_windows());
    toggle(&mut app);
    assert_eq!(app.conversation.rev(), None);

    assert!(
        app.on_daemon(snapshot(3, None, 99, "window three's"))
            .is_empty()
    );
    assert_eq!(
        app.conversation.rev(),
        None,
        "another window's snapshot applied"
    );

    assert!(
        app.on_daemon(snapshot(2, None, 41, "window two's"))
            .is_empty()
    );
    assert_eq!(app.conversation.rev(), Some(41));
    assert_eq!(
        app.conversation.conversation().unwrap().turns[0].blocks[0],
        proto::Block::Text {
            text: "window two's".into()
        }
    );

    // A delta for the view's key applies too, and moves `rev`.
    assert!(
        app.on_daemon(DaemonMsg::ConversationDelta {
            window_id: 2,
            agent_id: None,
            from_rev: 41,
            to_rev: 42,
            turns: vec![],
            session_id: Some("sess-two".into()),
            degraded: Some(proto::DegradeReason::TooLarge),
            dropped_turns: 0,
            dropped_by: None,
        })
        .is_empty()
    );
    assert_eq!(app.conversation.rev(), Some(42));
    let conversation = app.conversation.conversation().unwrap();
    assert_eq!(conversation.session_id.as_deref(), Some("sess-two"));
    assert_eq!(conversation.degraded, Some(proto::DegradeReason::TooLarge));
}

#[test]
fn conversation_gone_pops_or_closes() {
    let mut app = app_with(project_windows());
    toggle(&mut app);
    app.on_daemon(snapshot(2, None, 41, "root"));
    // Descend into the spawn (the last row): Enter on it.
    press(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::SubscribeConversation {
            window_id: 2,
            agent_id: Some("agent-of-41".into()),
            from_rev: None,
        })]
    );
    app.on_daemon(snapshot(2, Some("agent-of-41"), 57, "sub"));
    assert_eq!(app.conversation.trail().len(), 1);

    // Gone for another window's sub-agent: nothing.
    assert!(
        app.on_daemon(DaemonMsg::ConversationGone {
            window_id: 3,
            agent_id: Some("agent-of-41".into()),
            reason: proto::conversation::GONE_WINDOW_REMOVED.into(),
        })
        .is_empty()
    );
    assert_eq!(app.conversation.trail().len(), 1);
    assert_eq!(app.toast_text(), None);

    // Gone for the sub-agent on screen: the crumb pops, the reason is toasted, the view
    // stays open on the root.
    assert!(
        app.on_daemon(DaemonMsg::ConversationGone {
            window_id: 2,
            agent_id: Some("agent-of-41".into()),
            reason: proto::conversation::GONE_SUBAGENT_UNKNOWN.into(),
        })
        .is_empty()
    );
    assert!(app.conversation.trail().is_empty());
    assert!(app.conversation.is_open());
    assert_eq!(app.conversation.rev(), Some(41));
    assert_eq!(app.toast_text(), Some("no such sub-agent in this window"));
    assert!(app.keymap.conversation_mode());

    // Gone for the root: the view closes, the reason is toasted, and bare keys go back
    // to the PTY.
    app.on_daemon(DaemonMsg::ConversationGone {
        window_id: 2,
        agent_id: None,
        reason: proto::conversation::GONE_WINDOW_REMOVED.into(),
    });
    assert!(!app.conversation.is_open());
    assert_eq!(app.toast_text(), Some("window removed"));
    assert!(!app.keymap.conversation_mode());
    assert_eq!(
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 2,
            bytes: b"j".to_vec(),
        })]
    );
}

/// The daemon's "too large to send" ending is toasted like any other reason.
#[test]
fn a_conversation_too_large_to_send_is_toasted() {
    let mut app = app_with(project_windows());
    toggle(&mut app);
    app.on_daemon(DaemonMsg::ConversationGone {
        window_id: 2,
        agent_id: None,
        reason: "conversation too large to send".into(),
    });
    assert_eq!(app.toast_text(), Some("conversation too large to send"));
    assert!(!app.conversation.is_open());
    assert!(!app.keymap.conversation_mode());
}
