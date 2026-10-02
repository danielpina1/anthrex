//! M8c.9: the edit form's replies, pastes and snapshots while it is open (decisions
//! 33 and 34), and the gate's no-input guard. Split from `gate.rs` (rule 8).

use super::*;

#[test]
fn a_refused_edit_shows_inline_and_a_done_edit_closes() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    app.toast("");
    reply(
        &mut app,
        to(
            1,
            refused(
                proto::run_wire::request::EDIT,
                "task t1: size: below the floor",
            ),
        ),
    );
    assert_eq!(
        form(&app).error.as_deref(),
        Some("task t1: size: below the floor")
    );
    assert!(!form(&app).submitting);
    assert_eq!(app.toast_text(), Some(""), "shown inline, not toasted");

    // Resubmitting works once the refusal cleared `submitting`.
    assert_eq!(
        tap(&mut app, KeyCode::Enter),
        form_edit_as(2, vec![amend(None, Some(Size::S))])
    );
    reply(
        &mut app,
        to(
            2,
            refused(
                proto::run_wire::request::EDIT,
                "task t1: size: one\ntask t1: route: two\n\ntask t1: brief: three\n",
            ),
        ),
    );
    assert_eq!(
        form(&app).error.as_deref(),
        Some("task t1: size: one (+2 more)")
    );

    tap(&mut app, KeyCode::Enter);
    reply(
        &mut app,
        to(3, done(proto::run_wire::request::EDIT, "applied 1 edit(s)")),
    );
    assert_eq!(app.modal, None);
    assert_eq!(app.toast_text(), Some("applied 1 edit(s)"));
}

#[test]
fn a_long_refusal_is_capped_inline() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let long = "x".repeat(1_000);
    reply(
        &mut app,
        to(1, refused(proto::run_wire::request::EDIT, &long)),
    );
    let error = form(&app).error.clone().expect("an error");
    assert_eq!(error.chars().count(), 301);
    assert!(error.ends_with('…'));
}

#[test]
fn replies_for_other_requests_leave_a_submitting_form_alone() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let before = form(&app).clone();
    reply(
        &mut app,
        done(
            proto::run_wire::request::APPROVE,
            "run add-reset-3f9a approved",
        ),
    );
    assert_eq!(
        form(&app),
        &before,
        "a different request's Done keeps the form"
    );
    assert_eq!(app.toast_text(), Some("run add-reset-3f9a approved"));
    reply(
        &mut app,
        refused(proto::run_wire::request::REJECT, "no\nmore"),
    );
    assert_eq!(form(&app), &before);
    assert_eq!(app.toast_text(), Some("no (+1 more)"));
}

#[test]
fn an_edit_reply_with_no_form_open_toasts() {
    let mut app = gate();
    select(&mut app, task_key("t2"));
    tap(&mut app, KeyCode::Char('d'));
    tap(&mut app, KeyCode::Char('y'));
    reply(
        &mut app,
        refused(proto::run_wire::request::EDIT, "task t2: gone\nand more"),
    );
    assert_eq!(app.toast_text(), Some("task t2: gone (+1 more)"));
    assert_eq!(app.modal, None);
    reply(
        &mut app,
        done(proto::run_wire::request::EDIT, "applied 1 edit(s)"),
    );
    assert_eq!(app.toast_text(), Some("applied 1 edit(s)"));
}

#[test]
fn a_form_that_is_not_submitting_ignores_edit_replies() {
    let mut app = gate();
    open_form(&mut app, "t1");
    let before = form(&app).clone();
    reply(
        &mut app,
        done(proto::run_wire::request::EDIT, "applied 1 edit(s)"),
    );
    assert_eq!(
        form(&app),
        &before,
        "an older edit's reply does not close it"
    );
    reply(&mut app, refused(proto::run_wire::request::EDIT, "nope"));
    assert_eq!(form(&app), &before, "nor fills its error");
    assert_eq!(app.toast_text(), Some("nope"));
}

#[test]
fn a_double_enter_sends_once() {
    let mut app = gate();
    assert_eq!(submit_a_size_change(&mut app).len(), 1);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(form(&app).submitting);
}

#[test]
fn a_paste_goes_to_the_focused_text_field() {
    let mut app = gate();
    open_form(&mut app, "t1");
    focus(&mut app, EditField::Brief);
    assert!(app.on_paste(" three\r\nfour\nfive".into()).is_empty());
    assert_eq!(
        form(&app).brief.text(),
        "Line one\nLine two three\nfour\nfive"
    );
    focus(&mut app, EditField::Model);
    assert!(app.on_paste("gpt-\n5\r\n".into()).is_empty());
    assert_eq!(form(&app).model.text(), "gpt-5");
    // A megabyte is bounded in a one-line field (`run_edit::TEXT_MAX_CHARS`); the
    // brief takes up to `run_edit::BRIEF_MAX_CHARS` (decision 35).
    app.on_paste("y".repeat(1_000_000));
    assert_eq!(
        form(&app).model.text().chars().count(),
        crate::run_edit::TEXT_MAX_CHARS
    );
    // Final fix wave: the cap is one million characters; a paste up to it goes in
    // whole, and the next stops there.
    focus(&mut app, EditField::Brief);
    let cap = crate::run_edit::BRIEF_MAX_CHARS;
    let before = form(&app).brief.text().chars().count();
    app.on_paste("y".repeat(cap - before));
    assert_eq!(form(&app).brief.text().chars().count(), cap);
    app.on_paste("y".repeat(10));
    assert_eq!(form(&app).brief.text().chars().count(), cap);
}

#[test]
fn a_snapshot_that_removes_the_task_closes_the_form() {
    let mut app = gate();
    open_form(&mut app, "t1");
    let (mut snap, _) = gate_fixture();
    snap.runs[0].tasks.remove(0);
    deliver(&mut app, snap);
    assert_eq!(app.modal, None);
    assert_eq!(
        app.toast_text(),
        Some("t1 is no longer in run add-reset-3f9a's plan")
    );
    assert!(app.run_view.is_some());
}

#[test]
fn a_snapshot_that_closes_the_gate_closes_the_form() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let (mut snap, _) = gate_fixture();
    snap.runs[0].state = RunState::Running;
    deliver(&mut app, snap);
    assert_eq!(app.modal, None);
    assert_eq!(
        app.toast_text(),
        Some("the plan gate is closed: run add-reset-3f9a is running")
    );
}

#[test]
fn a_snapshot_that_drops_the_run_closes_the_form() {
    let mut app = gate();
    open_form(&mut app, "t1");
    deliver(&mut app, snapshot(10_001, vec![]));
    assert_eq!(app.modal, None);
    assert_eq!(app.run_view, None);
    assert_eq!(
        app.toast_text(),
        Some("the plan gate is closed: run add-reset-3f9a is gone")
    );
}

#[test]
fn a_snapshot_that_keeps_the_gate_keeps_the_form() {
    let mut app = gate();
    open_form(&mut app, "t1");
    tap(&mut app, KeyCode::Right);
    let before = form(&app).clone();
    let (mut snap, _) = gate_fixture();
    snap.runs[0].tasks[0] = edit_fixture_task();
    snap.runs[0]
        .tasks
        .push(task("t3", "new", Size::S, TaskState::Pending));
    deliver(&mut app, snap);
    assert_eq!(form(&app), &before);
}

#[test]
fn no_gate_path_sends_input() {
    let mut app = gate();
    let mut effects = vec![];
    for c in ['a', 'y', 'x', 'y', 'd', 'e'] {
        effects.extend(tap(&mut app, KeyCode::Char(c)));
    }
    select(&mut app, task_key("t1"));
    effects.extend(tap(&mut app, KeyCode::Char('e')));
    effects.extend(tap(&mut app, KeyCode::Right));
    effects.extend(tap(&mut app, KeyCode::Enter));
    effects.extend(app.on_paste("pasted".into()));
    assert!(
        effects.iter().all(|e| matches!(
            e,
            Effect::Send(ClientMsg::Run(_) | ClientMsg::RunTagged { .. })
        )),
        "{effects:?}"
    );
}

/// Milestone 9 decision 2: the form is matched by its request's id, never by the
/// request's name, so another `run edit`'s reply (the `d` confirm's, or an older
/// form's) leaves it submitting.
#[test]
fn an_edit_reply_for_another_request_leaves_the_form_submitting() {
    let mut app = gate();
    submit_a_size_change(&mut app);
    let before = form(&app).clone();
    reply(
        &mut app,
        refused(proto::run_wire::request::EDIT, "untagged"),
    );
    assert_eq!(form(&app), &before);
    assert_eq!(app.toast_text(), Some("untagged"));
    reply(
        &mut app,
        to(7, done(proto::run_wire::request::EDIT, "applied 1 edit(s)")),
    );
    assert_eq!(form(&app), &before);
    reply(
        &mut app,
        to(1, done(proto::run_wire::request::EDIT, "applied 1 edit(s)")),
    );
    assert_eq!(app.modal, None);
}
