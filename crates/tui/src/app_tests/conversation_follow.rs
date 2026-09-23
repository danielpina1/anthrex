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

/// Review N3 (spec decision 11): the prefix pressed twice sends a literal prefix byte to
/// the PTY — but not while the view is open, which reaches no PTY at all.
#[test]
fn the_prefix_twice_inside_the_view_reaches_no_pty() {
    let mut app = app_with(project_windows());
    toggle(&mut app);
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL).is_empty());
    assert!(app.conversation.is_open());
    assert!(!app.keymap.pending());

    // Closed, the same two keys send the prefix byte, as before.
    toggle(&mut app);
    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 2,
            bytes: vec![0x02],
        })]
    );
}

/// Three windows in three projects, so the tree order is 7, 8, 9.
fn three_windows() -> Vec<WindowInfo> {
    vec![
        project_win(7, "/p/a"),
        project_win(8, "/p/b"),
        project_win(9, "/p/c"),
    ]
}

/// Opens the view on window 7 and descends into `agent-of-41`.
fn open_on_seven(app: &mut App) {
    assert_eq!(app.focused, Some(7));
    toggle(app);
    app.on_daemon(snapshot(7, None, 41, "seven"));
    press(app, KeyCode::Char('G'), KeyModifiers::NONE);
    press(app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(
        app.conversation.key(),
        Some((7, Some("agent-of-41".into())))
    );
}

fn pty_subscribe(window_id: u32) -> Effect {
    Effect::Send(ClientMsg::Subscribe {
        window_id,
        cols: 80,
        rows: 24,
    })
}

/// Review M3, spec §6: the view shows the focused window's conversation. A focus key
/// moves the PTY and the view together: the whole trail and the root are unsubscribed,
/// deepest first, and the view opens on the new window's root.
#[test]
fn a_focus_key_moves_the_view_to_the_new_window() {
    let mut app = app_with(three_windows());
    open_on_seven(&mut app);

    prefix(&mut app);
    let effects = press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(effects[0], pty_subscribe(8));
    assert_eq!(
        conversation_msgs(&effects),
        vec![
            unsubscribe(7, Some("agent-of-41")),
            unsubscribe(7, None),
            subscribe(8, None),
        ]
    );
    assert_eq!(app.conversation.key(), Some((8, None)));
    assert!(app.conversation.trail().is_empty());
    assert!(app.keymap.conversation_mode());

    // The old window's messages no longer reach the view; the new one's do.
    app.on_daemon(snapshot(7, None, 50, "seven again"));
    assert_eq!(app.conversation.rev(), None);
    app.on_daemon(snapshot(8, None, 60, "eight"));
    assert_eq!(app.conversation.rev(), Some(60));
}

/// Review M3: a click on another window in the sidebar moves the view too.
#[test]
fn a_sidebar_click_moves_the_view_to_the_clicked_window() {
    let mut app = app_with(three_windows());
    open_on_seven(&mut app);

    let layout = crate::ui::layout(ratatui::layout::Rect::new(0, 0, 120, 30), 34);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    let rows = crate::tree::build(&app.windows, &app.tree);
    let index = rows
        .iter()
        .position(|row| row.key == crate::tree::NodeKey::Window(9))
        .expect("window 9 has a row");
    let effects = app.on_click(
        layout.sidebar_list.x + 2,
        layout.sidebar_list.y + index as u16,
        &layout,
    );
    assert_eq!(app.focused, Some(9));
    assert_eq!(
        conversation_msgs(&effects),
        vec![
            unsubscribe(7, Some("agent-of-41")),
            unsubscribe(7, None),
            subscribe(9, None),
        ]
    );
    assert_eq!(app.conversation.key(), Some((9, None)));
}

/// Review M3: the focused window removed, with the window list arriving before the
/// daemon's `ConversationGone` — the view follows focus to the neighbour, and the late
/// `Gone` for the removed window changes nothing.
#[test]
fn a_removed_window_moves_the_view_to_the_neighbour_list_first() {
    let mut app = app_with(three_windows());
    open_on_seven(&mut app);

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![project_win(8, "/p/b"), project_win(9, "/p/c")],
    });
    assert_eq!(app.focused, Some(8));
    assert_eq!(
        conversation_msgs(&effects),
        vec![
            unsubscribe(7, Some("agent-of-41")),
            unsubscribe(7, None),
            subscribe(8, None),
        ]
    );
    // Final re-review m1: the switch still says why, whichever message came first.
    assert_eq!(app.toast_text(), Some("window removed"));
    assert!(
        app.on_daemon(DaemonMsg::ConversationGone {
            window_id: 7,
            agent_id: None,
            reason: proto::conversation::GONE_WINDOW_REMOVED.into(),
        })
        .is_empty()
    );
    assert_eq!(app.conversation.key(), Some((8, None)));
    assert!(app.keymap.conversation_mode());
}

/// Review M3: the same removal with the daemon's `ConversationGone` first. It closes the
/// view with its reason, and the window list that follows opens it on the neighbour.
#[test]
fn a_removed_window_moves_the_view_to_the_neighbour_gone_first() {
    let mut app = app_with(three_windows());
    open_on_seven(&mut app);

    for agent_id in [Some("agent-of-41"), None] {
        app.on_daemon(DaemonMsg::ConversationGone {
            window_id: 7,
            agent_id: agent_id.map(str::to_owned),
            reason: proto::conversation::GONE_WINDOW_REMOVED.into(),
        });
    }
    assert!(!app.conversation.is_open());
    assert_eq!(app.toast_text(), Some("window removed"));

    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![project_win(8, "/p/b"), project_win(9, "/p/c")],
    });
    assert_eq!(app.focused, Some(8));
    assert_eq!(conversation_msgs(&effects), vec![subscribe(8, None)]);
    assert_eq!(app.conversation.key(), Some((8, None)));
    assert!(app.keymap.conversation_mode());
}

/// Review M3: focus moving to no window at all closes the view, with the same toast
/// `C-b m` shows when no window is focused.
#[test]
fn focus_moving_to_no_window_closes_the_view() {
    let mut app = app_with(vec![project_win(7, "/p/a")]);
    open_on_seven(&mut app);

    let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
    assert_eq!(app.focused, None);
    assert_eq!(
        conversation_msgs(&effects),
        vec![unsubscribe(7, Some("agent-of-41")), unsubscribe(7, None)]
    );
    assert!(!app.conversation.is_open());
    assert!(!app.keymap.conversation_mode());
    assert_eq!(app.toast_text(), Some("no window focused"));
}

/// Review M3: the view closed by its window's removal, when that was the last window,
/// says so too once the empty list arrives.
#[test]
fn the_last_window_removed_gone_first_leaves_the_view_closed() {
    let mut app = app_with(vec![project_win(7, "/p/a")]);
    open_on_seven(&mut app);
    app.on_daemon(DaemonMsg::ConversationGone {
        window_id: 7,
        agent_id: None,
        reason: proto::conversation::GONE_WINDOW_REMOVED.into(),
    });
    let effects = app.on_daemon(DaemonMsg::WindowsChanged { windows: vec![] });
    assert!(conversation_msgs(&effects).is_empty());
    assert!(!app.conversation.is_open());
    assert_eq!(app.toast_text(), Some("no window focused"));

    // A window created later is not a reason to open the view on its own.
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![project_win(8, "/p/b")],
    });
    assert!(conversation_msgs(&effects).is_empty());
    assert!(!app.conversation.is_open());
}

/// Reviews I1 and M3 together: the focused window vanished while the link was down.
/// The reconnect's window list moves focus to the neighbour, and the view goes with it,
/// subscribing the neighbour's root once and nothing for the window that is gone.
#[test]
fn a_reconnect_that_moves_focus_moves_the_view_once() {
    let mut app = app_with(three_windows());
    open_on_seven(&mut app);
    app.on_link_lost("connection closed");

    let effects = app.on_reconnected(vec![project_win(8, "/p/b"), project_win(9, "/p/c")]);
    assert_eq!(app.focused, Some(8));
    assert_eq!(conversation_msgs(&effects), vec![subscribe(8, None)]);
    assert_eq!(app.conversation.key(), Some((8, None)));
}
