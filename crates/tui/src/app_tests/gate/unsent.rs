//! Whole-branch review M2: an edit form whose `Edit` the connection refused, or whose
//! reply a lost link took with it, stops submitting and says so inline, so `Enter`
//! retries instead of the form staying "saving…" until it is closed.

use super::*;

const NOT_SENT: &str = "the edit was not sent; press Enter to retry";

fn the_edit() -> Vec<Effect> {
    form_edit(vec![amend(None, Some(Size::S))])
}

fn sent_edit() -> ClientMsg {
    match the_edit().remove(0) {
        Effect::Send(msg) => msg,
        other => panic!("not a send: {other:?}"),
    }
}

/// The form is free again, the error inline, and `Enter` sends the same edit under a
/// new id.
fn retryable(app: &mut App) {
    assert!(!form(app).submitting);
    assert_eq!(form(app).error.as_deref(), Some(NOT_SENT));
    assert_eq!(
        tap(app, KeyCode::Enter),
        form_edit_as(2, vec![amend(None, Some(Size::S))])
    );
    assert!(form(app).submitting);
    assert_eq!(form(app).error, None);
}

#[test]
fn a_refused_edit_send_frees_the_form_to_retry() {
    let mut app = gate();
    assert_eq!(submit_a_size_change(&mut app), the_edit());
    assert!(form(&app).submitting);
    assert!(app.on_send_failed(&sent_edit()).is_empty());
    retryable(&mut app);
}

#[test]
fn another_refused_send_leaves_a_submitting_form_alone() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let before = form(&app).clone();
    let approve = ClientMsg::Run(RunRequest::Approve {
        run_id: RUN_ID.into(),
    });
    assert!(app.on_send_failed(&approve).is_empty());
    assert_eq!(form(&app), &before);
}

#[test]
fn a_lost_link_frees_a_submitting_form_to_retry() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    assert!(app.on_link_lost("connection closed").is_empty());
    assert!(!app.connected());
    retryable(&mut app);
}

#[test]
fn a_reconnect_frees_a_submitting_form_to_retry() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let windows = app.windows.clone();
    app.on_reconnected(windows);
    retryable(&mut app);
}

#[test]
fn a_lost_link_leaves_a_form_that_is_not_submitting_alone() {
    let mut app = gate();
    open_form(&mut app, "t1");
    let before = form(&app).clone();
    app.on_link_lost("connection closed");
    let windows = app.windows.clone();
    app.on_reconnected(windows);
    assert_eq!(form(&app), &before);
}
