//! The final review's fixes to the conversation view (milestone 6.5): it survives a
//! reconnect (I1), it follows focus (M3), and the prefix pressed twice inside it
//! reaches no PTY (N3). Split from `app_tests/conversation.rs` to keep both files
//! under the 600-line rule.

use super::conversation::{snapshot, toggle};
use super::*;

/// Only the conversation messages among `effects`, in order.
fn conversation_msgs(effects: &[Effect]) -> Vec<ClientMsg> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Send(
                msg @ (ClientMsg::SubscribeConversation { .. }
                | ClientMsg::UnsubscribeConversation { .. }),
            ) => Some(msg.clone()),
            _ => None,
        })
        .collect()
}

fn subscribe(window_id: u32, agent_id: Option<&str>) -> ClientMsg {
    ClientMsg::SubscribeConversation {
        window_id,
        agent_id: agent_id.map(str::to_owned),
        from_rev: None,
    }
}

fn unsubscribe(window_id: u32, agent_id: Option<&str>) -> ClientMsg {
    ClientMsg::UnsubscribeConversation {
        window_id,
        agent_id: agent_id.map(str::to_owned),
    }
}

fn delta(window_id: u32, agent_id: Option<&str>, from_rev: u64, to_rev: u64) -> DaemonMsg {
    DaemonMsg::ConversationDelta {
        window_id,
        agent_id: agent_id.map(str::to_owned),
        from_rev,
        to_rev,
        turns: vec![],
        session_id: None,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
    }
}

/// Opens the view on window 2 and descends one level, into `agent-of-41`.
fn open_and_descend(app: &mut App) {
    toggle(app);
    app.on_daemon(snapshot(2, None, 41, "root"));
    press(app, KeyCode::Char('G'), KeyModifiers::NONE);
    assert_eq!(
        conversation_msgs(&press(app, KeyCode::Enter, KeyModifiers::NONE)),
        vec![subscribe(2, Some("agent-of-41"))]
    );
    app.on_daemon(snapshot(2, Some("agent-of-41"), 57, "sub"));
    assert_eq!(app.conversation.trail().len(), 1);
    assert_eq!(app.conversation.rev(), Some(57));
}

/// Review I1: a conversation subscription belongs to the connection, so after a
/// reconnect every level of the trail is subscribed again — root first, in trail order,
/// once each, and with `from_rev: None`, because a rev from before a daemon restart
/// means nothing (review M1). Until each snapshot lands, a delta for that level is
/// dropped quietly rather than asking for yet another snapshot.
#[test]
fn a_reconnect_resubscribes_every_level_of_the_trail() {
    let mut app = app_with(project_windows());
    open_and_descend(&mut app);

    assert!(app.on_link_lost("connection closed").is_empty());
    let effects = app.on_reconnected(project_windows());
    assert_eq!(
        conversation_msgs(&effects),
        vec![subscribe(2, None), subscribe(2, Some("agent-of-41"))]
    );
    assert!(app.conversation.is_open());
    assert_eq!(app.conversation.trail().len(), 1);

    // Deltas the new daemon sends before the snapshots are dropped, with no second
    // subscribe for either level.
    assert!(app.on_daemon(delta(2, None, 3, 4)).is_empty());
    assert!(
        app.on_daemon(delta(2, Some("agent-of-41"), 2, 3))
            .is_empty()
    );

    // The snapshots apply, on the level each belongs to.
    assert!(
        app.on_daemon(snapshot(2, Some("agent-of-41"), 2, "sub again"))
            .is_empty()
    );
    assert_eq!(app.conversation.rev(), Some(2));
    assert!(app.on_daemon(snapshot(2, None, 3, "root again")).is_empty());
    // And a delta that follows a snapshot applies as usual.
    assert!(app.on_daemon(delta(2, None, 3, 4)).is_empty());
    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.conversation.trail().is_empty());
    assert_eq!(app.conversation.rev(), Some(4));
}

/// Review I1: while the link is down the view's subscriptions are already gone, so
/// closing it sends nothing, and a reconnect with the view closed subscribes nothing.
#[test]
fn a_view_closed_while_disconnected_sends_nothing() {
    let mut app = app_with(project_windows());
    open_and_descend(&mut app);
    app.on_link_lost("connection closed");

    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE).is_empty());
    assert!(!app.conversation.is_open());
    assert!(conversation_msgs(&app.on_reconnected(project_windows())).is_empty());

    // Connected again, the view subscribes and unsubscribes as before.
    assert_eq!(
        conversation_msgs(&toggle(&mut app)),
        vec![subscribe(2, None)]
    );
    assert_eq!(
        conversation_msgs(&toggle(&mut app)),
        vec![unsubscribe(2, None)]
    );
}
