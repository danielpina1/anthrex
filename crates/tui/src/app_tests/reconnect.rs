//! Task M6.11: the reconnect state machine — link loss, retries, resubscribing the
//! focused window, and the disconnected-command toasts. Split out from `app/tests.rs`
//! from the start per `AGENTS.md` hard rule 8, the same shape `app_tests/lifecycle.rs`
//! already gives task M6.10's tests.

use super::*;

#[test]
fn link_lost_then_reconnected_resubscribes_the_focused_window() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    app.focus(2);
    app.on_daemon(DaemonMsg::Snapshot {
        window_id: 2,
        cols: 80,
        rows: 24,
        bytes: vec![],
    });

    // Whole-branch-review Minor m2: `attempts` starts at 1, not 0. `statusbar.rs`
    // renders it verbatim as `reconnecting (attempt {attempts})`, so a 0 here used to
    // put "reconnecting (attempt 0)" on screen for the first ~2s of every disconnect —
    // the very first thing a user sees when the daemon dies — while decision 34's own
    // mock-up shows a number that reads as "which attempt is this", never zero.
    assert!(app.on_link_lost("x").is_empty());
    assert!(matches!(app.link, Link::Reconnecting { attempts: 1, .. }));
    assert_eq!(app.toast_text(), Some("connection to the daemon lost"));

    assert!(app.on_reconnect_failed("refused", false).is_empty());
    match &app.link {
        Link::Reconnecting { attempts, .. } => assert_eq!(*attempts, 2),
        other => panic!("expected Link::Reconnecting, got {other:?}"),
    }

    let effects = app.on_reconnected(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24,
        })]
    );
    assert!(matches!(app.link, Link::Connected));
    assert_eq!(app.toast_text(), Some("reconnected"));
}

/// The worst defect shape the brief calls out: a reconnect that shows the user a
/// different agent than the one they were watching. `replace_windows`'s own neighbour
/// fallback (already covered by `removed_focused_window_moves_focus_to_a_neighbour`
/// for an ordinary `WindowsChanged`) must fire exactly the same way when the change
/// arrives via `on_reconnected`, and must not also get a second, duplicate `Subscribe`
/// from `on_reconnected`'s own "resubscribe even though already focused" logic.
#[test]
fn reconnect_when_the_focused_window_is_gone_focuses_a_neighbour() {
    let mut app = app_with(vec![
        win(1, "a", Status::Idle),
        win(2, "b", Status::Idle),
        win(3, "c", Status::Idle),
    ]);
    app.focus(2);
    app.on_link_lost("x");
    app.on_reconnect_failed("refused", false);

    let effects = app.on_reconnected(vec![win(1, "a", Status::Idle), win(3, "c", Status::Idle)]);
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 3,
            cols: 80,
            rows: 24,
        })],
        "exactly one Subscribe, for the neighbour that took over focus"
    );
    assert_eq!(app.focused, Some(3));
}

#[test]
fn giving_up_sets_lost_and_c_b_r_retries() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    app.on_link_lost("x");
    assert!(app.on_reconnect_failed("timed out", true).is_empty());
    assert!(matches!(app.link, Link::Lost { .. }));

    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE),
        vec![Effect::Reconnect]
    );
    assert!(matches!(app.link, Link::Reconnecting { .. }));

    // While Connected, C-b r is the pre-existing no-op affirmation instead.
    app.link = Link::Connected;
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('r'), KeyModifiers::NONE).is_empty());
    assert_eq!(app.toast_text(), Some("connected"));
}

/// Decision 35: `App::focus`'s early return for the already-focused window now also
/// requires `subscribed == Some(id)`, so a refused `Subscribe` is retried instead of
/// being permanently masked by that early return.
#[test]
fn a_refused_subscribe_is_retried_on_tick() {
    let mut app = app_with(vec![win(1, "a", Status::Idle), win(2, "b", Status::Idle)]);
    let subscribe = ClientMsg::Subscribe {
        window_id: 2,
        cols: 80,
        rows: 24,
    };
    assert_eq!(app.focus(2), vec![Effect::Send(subscribe.clone())]);
    assert_eq!(app.subscribed, Some(2));

    assert!(app.on_send_failed(&subscribe).is_empty());
    assert_eq!(app.subscribed, None);

    assert_eq!(app.on_tick(), vec![Effect::Send(subscribe.clone())]);
    assert_eq!(app.subscribed, Some(2));
    assert!(
        app.on_tick().is_empty(),
        "already resubscribed; nothing more to retry"
    );

    // The early return in `focus` no longer blocks a retry once the subscription it
    // was counting on has since been refused.
    assert!(app.on_send_failed(&subscribe).is_empty());
    assert_eq!(app.focus(2), vec![Effect::Send(subscribe)]);
}

#[test]
fn commands_while_disconnected_toast() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    app.link = Link::Reconnecting {
        attempts: 1,
        reason: "x".into(),
    };

    assert!(
        app.on_send_failed(&ClientMsg::Kill { window_id: 1 })
            .is_empty()
    );
    assert_eq!(app.toast_text(), Some("not connected; C-b r to reconnect"));

    app.toast = None;
    assert!(
        app.on_send_failed(&ClientMsg::Input {
            window_id: 1,
            bytes: vec![],
        })
        .is_empty()
    );
    assert_eq!(app.toast_text(), None, "an Input refusal stays silent");
}

/// Decision 33: a reconnect applies the new window list through `replace_windows` and
/// resubscribes, and touches nothing else. The client process did not die, so the user
/// must come back to the view they left — the tree viewport and selection, what is
/// collapsed, the filter, the overview, milestone 4.7's inspector panel and the graph
/// pan.
///
/// Written while auditing whether this actually held (it does, and has since milestone
/// 6). It is here because nothing asserted it: the decision is spread across
/// `on_reconnected`, `replace_windows` and `repair_selection`, and a later change to any
/// of the three could reset the view with every other reconnect test still passing.
#[test]
fn a_reconnect_leaves_the_view_where_the_user_left_it() {
    let windows: Vec<_> = (1..=12)
        .map(|id| win(id, &format!("w{id}"), Status::Idle))
        .collect();
    let mut app = app_with(windows.clone());
    app.focus(7);
    app.overview = true;
    app.inspector_visible = true;
    app.sidebar_width = 41;
    app.tree.filter = "w1".into();
    app.tree.collapsed.insert(crate::tree::NodeKey::Window(3));
    app.tree.selected = Some(crate::tree::NodeKey::Window(11));
    app.tree.sidebar.top = 4;
    app.tree.overview.top = 6;
    app.graph_pan = crate::graph::Pan { x: 3, y: 2 };

    let before = (
        app.overview,
        app.inspector_visible,
        app.sidebar_width,
        app.tree.filter.clone(),
        app.tree.collapsed.clone(),
        app.tree.selected.clone(),
        app.tree.sidebar.top,
        app.tree.overview.top,
        app.graph_pan,
        app.focused,
    );

    app.on_link_lost("x");
    app.on_reconnect_failed("refused", false);
    app.on_reconnected(windows);

    assert_eq!(app.overview, before.0, "overview");
    assert_eq!(app.inspector_visible, before.1, "inspector_visible");
    assert_eq!(app.sidebar_width, before.2, "sidebar_width");
    assert_eq!(app.tree.filter, before.3, "tree filter");
    assert_eq!(app.tree.collapsed, before.4, "collapsed set");
    assert_eq!(app.tree.selected, before.5, "tree selection");
    assert_eq!(app.tree.sidebar.top, before.6, "sidebar viewport top");
    assert_eq!(app.tree.overview.top, before.7, "overview viewport top");
    assert_eq!(app.graph_pan, before.8, "graph pan");
    assert_eq!(app.focused, before.9, "focused window");
}
