//! Milestone 9.6 task 17: the document gate screen's keys, requests and replies.

use super::*;
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::doc_gate::tests::{
    REPORT, app_with, code, design_run, key, opened, opened_with, reply, screen, sent,
};
use crossterm::event::KeyCode;
use proto::{ActionKind, RunReply, RunRequest};

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn confirm_of(app: &App) -> Option<(String, PendingAction)> {
    match &app.modal {
        Some(Modal::Confirm { message, action }) => Some((message.clone(), action.clone())),
        _ => None,
    }
}

/// The one tagged `DocGate` among `effects`.
fn gate_sent(effects: &[Effect]) -> (u64, DocGateAction) {
    match &sent(effects)[..] {
        [
            (
                id,
                RunRequest::DocGate {
                    run,
                    kind: _,
                    action,
                },
            ),
        ] if run == RUN_ID => (*id, action.clone()),
        other => panic!("one DocGate: {other:?}"),
    }
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        app.on_key(key(c));
    }
}

/// The brief's test: `a`, `x`, `r` and `b` send their `DocGate` only after their
/// confirm page's `y`; `c` and `e` only on their editor's Ctrl-S. Nothing is sent by
/// the key itself, nor by a page left with `n`.
#[test]
fn each_key_sends_its_request_after_its_confirm() {
    // a: approve.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    assert!(sent(&app.on_key(key('a'))).is_empty());
    let (message, _) = confirm_of(&app).expect("a confirm page");
    assert_eq!(message, "Approve spec v2? Planning starts next.");
    assert!(sent(&app.on_key(key('n'))).is_empty(), "n leaves the page");
    app.on_key(key('a'));
    assert_eq!(
        gate_sent(&app.on_key(key('y'))).1,
        DocGateAction::Approve { version: Some(2) }
    );

    // x: reject, destructive (Enter does not confirm it).
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('x'));
    let (message, action) = confirm_of(&app).unwrap();
    assert_eq!(message, "Reject the spec? This discards run 3f9a.");
    assert!(action.destructive());
    assert!(sent(&app.on_key(code(KeyCode::Enter))).is_empty());
    assert_eq!(gate_sent(&app.on_key(key('y'))).1, DocGateAction::Reject);

    // r: rethink, a note then its page.
    let mut app = opened(DocGateKind::Brainstorm, 120, 40);
    assert!(sent(&app.on_key(key('r'))).is_empty());
    assert!(matches!(app.modal, Some(Modal::DocNote(_))));
    type_text(&mut app, "think smaller");
    assert!(sent(&app.on_key(ctrl('s'))).is_empty());
    let (message, _) = confirm_of(&app).unwrap();
    assert_eq!(
        message,
        "Rethink the brainstorm? Both brainstormers run again."
    );
    let note = "think smaller".to_owned();
    assert_eq!(
        gate_sent(&app.on_key(key('y'))).1,
        DocGateAction::Rethink { note }
    );

    // b: back, a note then its page.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('b'));
    type_text(&mut app, "again");
    assert!(sent(&app.on_key(ctrl('s'))).is_empty());
    let (message, _) = confirm_of(&app).unwrap();
    assert_eq!(message, "Go back to the brainstorm? The spec is set aside.");
    let note = "again".to_owned();
    assert_eq!(
        gate_sent(&app.on_key(key('y'))).1,
        DocGateAction::Back { note }
    );

    // c: changes, sent on Ctrl-S, review no by default, Tab toggles it.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('c'));
    type_text(&mut app, "split R1");
    let effects = app.on_key(ctrl('s'));
    let changes = DocGateAction::Changes {
        note: "split R1".into(),
        review: false,
    };
    assert_eq!(gate_sent(&effects).1, changes);
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('c'));
    type_text(&mut app, "split R1");
    app.on_key(code(KeyCode::Tab));
    let (_, action) = gate_sent(&app.on_key(ctrl('s')));
    assert!(
        matches!(action, DocGateAction::Changes { review: true, .. }),
        "{action:?}"
    );

    // e: the document in the editor, its text sent on Ctrl-S.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('e'));
    let Some(Modal::DocNote(form)) = &app.modal else {
        panic!("the editor");
    };
    assert_eq!(form.text.text(), crate::ui::doc_gate::tests::SPEC);
    type_text(&mut app, "R3 x");
    let (_, action) = gate_sent(&app.on_key(ctrl('s')));
    let DocGateAction::Edit { text } = action else {
        panic!("{action:?}");
    };
    assert!(
        text.starts_with("# Password reset") && text.ends_with("R3 x"),
        "{text:?}"
    );
}

/// DF §6.1: while the orchestrator revises, only `x` and `q` act; every other gate key
/// says so on the message line and sends nothing.
#[test]
fn revising_disables_all_but_x_and_q() {
    let mut run = design_run(DocGateKind::Spec);
    run.doc_gate.as_mut().unwrap().revising = Some("split R1".into());
    let mut app = opened_with(run, 120, 40);
    for c in ['a', 'c', 'e', 'r', 'b', 'd', 'f', 'g'] {
        let effects = app.on_key(key(c));
        assert!(sent(&effects).is_empty(), "{c}");
        assert!(app.modal.is_none(), "{c}");
        assert_eq!(screen(&app).pane, DocPane::Document, "{c}");
        let (tone, text) = screen(&app).message.clone().unwrap();
        assert_eq!(tone, Tone::Note);
        assert_eq!(
            text,
            "the orchestrator is revising v3; only x and q work now"
        );
    }
    app.on_key(key('x'));
    assert!(confirm_of(&app).is_some(), "x still asks");
    app.on_key(key('n'));
    app.on_key(key('q'));
    assert!(app.screen.is_none(), "q closes");
}

/// The brief's test: `c` at the brainstorm gate opens with the report's questions.
#[test]
fn changes_at_the_brainstorm_gate_prefills_the_questions() {
    let mut app = opened(DocGateKind::Brainstorm, 120, 40);
    app.on_key(key('c'));
    let Some(Modal::DocNote(form)) = &app.modal else {
        panic!("the note editor");
    };
    assert_eq!(
        form.text.text(),
        "- Which mailer?\n- How long do tokens live?"
    );
    assert_eq!(crate::doc_note::report_questions(REPORT), form.text.text());
    // At the spec gate the note opens empty.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('c'));
    let Some(Modal::DocNote(form)) = &app.modal else {
        panic!("the note editor");
    };
    assert_eq!(form.text.text(), "");
}

/// Daemon refusals are shown verbatim on the message line (rulings T1-O1, T11-3, T10-6
/// are the daemon's texts); a refused edit keeps its editor and text.
#[test]
fn a_refusal_is_shown_verbatim_on_the_message_line() {
    let refusal = "the spec is missing the section \"## Requirements\"";
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('a'));
    let (id, _) = gate_sent(&app.on_key(key('y')));
    reply(
        &mut app,
        RunReply::Refused {
            request: proto::run_wire::request::DOC_GATE.into(),
            message: refusal.into(),
            request_id: Some(id),
        },
    );
    assert_eq!(
        screen(&app).message,
        Some((Tone::Refused, refusal.to_owned()))
    );

    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('e'));
    let (id, _) = gate_sent(&app.on_key(ctrl('s')));
    reply(
        &mut app,
        RunReply::Refused {
            request: proto::run_wire::request::DOC_GATE.into(),
            message: refusal.into(),
            request_id: Some(id),
        },
    );
    let Some(Modal::DocNote(form)) = &app.modal else {
        panic!("the editor stays");
    };
    assert_eq!(form.error.as_deref(), Some(refusal));
    assert!(!form.submitting);
    assert_eq!(screen(&app).message, Some((Tone::Refused, refusal.into())));

    // A taken approve closes the screen with the daemon's text.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('a'));
    let (id, _) = gate_sent(&app.on_key(key('y')));
    let done = "run add-reset-3f9a: the spec v2 is approved; planning";
    reply(
        &mut app,
        RunReply::Done {
            request: proto::run_wire::request::DOC_GATE.into(),
            message: done.into(),
            request_id: Some(id),
        },
    );
    assert!(app.screen.is_none());
    assert_eq!(app.toast_text(), Some(done));
}

/// The menu's `review document` opens the screen on a brainstorm or spec gate; a design
/// run's plan gate opens the plan review; an Alerts row's Enter preselects it.
#[test]
fn review_doc_opens_the_gate_screen() {
    let mut app = app_with(design_run(DocGateKind::Spec), 120, 40);
    let effects = app.open_doc_gate(RUN_ID);
    assert!(matches!(
        &sent(&effects)[..],
        [(
            _,
            RunRequest::ShowDoc {
                version: Some(2),
                findings: true,
                diff: false,
                ..
            }
        )]
    ));
    assert_eq!(screen(&app).version, 2);
    assert_eq!(app.key_region(), super::super::region::KeyRegion::Screen);

    let mut plan = design_run(DocGateKind::Plan);
    plan.tasks = crate::tree::run_fixtures::gate_fixture().0.runs[0]
        .tasks
        .clone();
    let mut app = app_with(plan, 120, 40);
    app.open_doc_gate(RUN_ID);
    assert!(app.screen.is_none());
    assert!(app.plan_review.is_some(), "the plan gate is today's review");

    let app = app_with(design_run(DocGateKind::Brainstorm), 120, 40);
    let alert = crate::app::alerts(&app).remove(0);
    assert_eq!(
        crate::app::alerts_view::preselected(&app, &alert.key),
        Some(ActionKind::ReviewDoc)
    );
}

/// The screen follows its gate to a newer version (the orchestrator's revision, the
/// user's edit), asking for it afresh.
#[test]
fn the_screen_follows_its_gate_to_a_newer_version() {
    let mut app = opened(DocGateKind::Spec, 120, 40);
    let mut run = design_run(DocGateKind::Spec);
    run.doc_gate.as_mut().unwrap().version = 3;
    let (mut snap, _) = crate::tree::run_fixtures::gate_fixture();
    snap.runs = vec![run];
    app.on_daemon(proto::DaemonMsg::Run(RunReply::Snapshot(snap)));
    let effects = app.screens_tick(std::time::Instant::now());
    assert!(matches!(
        &sent(&effects)[..],
        [(
            _,
            RunRequest::ShowDoc {
                version: Some(3),
                ..
            }
        )]
    ));
    assert_eq!(screen(&app).version, 3);
    assert!(matches!(screen(&app).doc, DocLoad::Loading(_)));
}

#[path = "doc_gate_tests_keys.rs"]
mod keys;
