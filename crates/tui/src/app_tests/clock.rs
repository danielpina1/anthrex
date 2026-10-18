//! Milestone 9.11 decision 23 (C9): the client's clock is injectable. The resize
//! debounce, the toast's time to live and the double-click window are measured
//! against the `now` a caller passes, so their bounds are tested exactly, with no sleep.

use super::*;

const MS: Duration = Duration::from_millis(1);

#[test]
fn a_resize_waits_exactly_the_debounce() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let t0 = Instant::now();
    assert!(app.set_terminal_size_at(120, 40, t0).is_empty());
    assert!(
        app.on_tick_at(t0 + RESIZE_DEBOUNCE - MS, 0).is_empty(),
        "one millisecond short of the debounce sends nothing"
    );
    assert_eq!(
        app.on_tick_at(t0 + RESIZE_DEBOUNCE, 0),
        vec![Effect::Send(ClientMsg::Resize {
            window_id: 1,
            cols: 120,
            rows: 40
        })],
        "the debounce's own instant sends the resize"
    );
}

#[test]
fn a_later_size_change_restarts_the_debounce() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let t0 = Instant::now();
    assert!(app.set_terminal_size_at(120, 40, t0).is_empty());
    let t1 = t0 + RESIZE_DEBOUNCE - MS;
    assert!(app.set_terminal_size_at(100, 30, t1).is_empty());
    assert!(app.on_tick_at(t0 + RESIZE_DEBOUNCE, 0).is_empty());
    assert_eq!(
        app.on_tick_at(t1 + RESIZE_DEBOUNCE, 0),
        vec![Effect::Send(ClientMsg::Resize {
            window_id: 1,
            cols: 100,
            rows: 30
        })]
    );
}

#[test]
fn a_toast_expires_at_its_ttl() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    app.toast("hello");
    let at = app.toast.as_ref().expect("the toast is shown").at;
    app.on_tick_at(at + TOAST_TTL - MS, 0);
    assert_eq!(app.toast_text(), Some("hello"), "still shown just before");
    app.on_tick_at(at + TOAST_TTL, 0);
    assert_eq!(app.toast_text(), None, "gone at its time to live");
}

#[test]
fn the_tick_stamps_the_given_now() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    let later = Instant::now() + Duration::from_secs(3600);
    app.on_tick_at(later, 0);
    assert_eq!(app.ticked_at, later);
}

#[test]
fn a_double_click_is_measured_against_the_given_now() {
    let mut mouse = crate::mouse::MouseState::default();
    let t0 = Instant::now();
    assert!(!mouse.press(5, 6, t0), "a first press is a single click");
    assert!(
        mouse.press(5, 6, t0 + crate::mouse::DOUBLE_CLICK - MS),
        "a second press inside the window is a double click"
    );
    assert!(!mouse.press(5, 6, t0), "the gesture was consumed");
    assert!(
        !mouse.press(5, 6, t0 + crate::mouse::DOUBLE_CLICK),
        "a second press at the window's end is a new single click"
    );
}

/// Milestone 9.19 decision 11: a finished sub-agent hides once its age at the tick,
/// `age_secs(ended)`, passes `keep_finished_secs`, before the next window list comes.
#[test]
fn a_finished_subagent_hides_on_time() {
    let finished = proto::SubagentInfo {
        id: "s1".into(),
        parent_id: None,
        kind: "agent".into(),
        label: None,
        model: None,
        state: proto::SubagentState::Done,
        tool: None,
        started_secs: 200,
        ended_secs: Some(100),
        needs_permission: false,
    };
    let window = WindowInfo {
        subagents: vec![finished],
        ..win(1, "a", Status::Idle)
    };
    let mut app = app_with(vec![window]);
    app.tree.keep_finished_secs = 120;
    let t0 = Instant::now();
    app.set_windows_received_at(t0);
    let shown = |app: &App| {
        let key = crate::tree::NodeKey::Subagent {
            window_id: 1,
            id: "s1".into(),
        };
        app.rows().iter().any(|row| row.key == key)
    };
    app.on_tick_at(t0 + Duration::from_secs(20), 0);
    assert!(shown(&app), "120 s finished is not past the setting");
    app.on_tick_at(t0 + Duration::from_secs(21), 0);
    assert!(!shown(&app), "121 s finished is past it");
}
