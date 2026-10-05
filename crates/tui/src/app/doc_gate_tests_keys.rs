//! Task M9.6.17, fix round 1: the approve names the version its page showed (ruling
//! T17-1), a closed gate leaves only `q` and Esc (T17-2), the client's own refusals and
//! the brainstorm's approve page, Enter on a gate alert through the action menu, and an
//! expired `ShowDoc` reported once (T17-4).

use super::*;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
use crate::ui::doc_gate::tests::{
    app_with, code, design_run, key, opened, reply, rows, screen, sent,
};
use crossterm::event::KeyCode;
use proto::{ActionInfo, ActionKind, DaemonMsg, RunReply, RunRequest};
use std::time::{Duration, Instant};

/// The daemon's snapshot now holds `run` as the one run.
fn deliver(app: &mut App, run: RunInfo) {
    let (mut snap, _) = gate_fixture();
    snap.runs = vec![run];
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
}

/// The one tagged `DocGate` among `effects`.
fn gate_action(effects: &[Effect]) -> DocGateAction {
    match &sent(effects)[..] {
        [(_, RunRequest::DocGate { action, .. })] => action.clone(),
        other => panic!("one DocGate: {other:?}"),
    }
}

fn confirm_message(app: &App) -> String {
    match &app.modal {
        Some(Modal::Confirm { message, .. }) => message.clone(),
        other => panic!("a confirm page: {other:?}"),
    }
}

/// Ruling T17-1: the approve carries the version its page showed. An `edit-doc` from
/// elsewhere opens v3 while the page for v2 is open: `y` still sends v2, which the
/// daemon refuses in its own words, shown on the message line. The screen has moved to
/// v3 meanwhile, and the next `a` asks for and sends v3.
#[test]
fn an_approve_names_the_version_its_page_showed() {
    let mut app = opened(DocGateKind::Spec, 120, 40);
    app.on_key(key('a'));
    assert_eq!(
        confirm_message(&app),
        "Approve spec v2? Planning starts next."
    );
    let mut edited = design_run(DocGateKind::Spec);
    edited.doc_gate.as_mut().unwrap().version = 3;
    deliver(&mut app, edited);
    app.screens_tick(Instant::now());
    assert_eq!(screen(&app).version, 3, "the screen follows the gate");
    let effects = app.on_key(key('y'));
    assert_eq!(
        gate_action(&effects),
        DocGateAction::Approve { version: Some(2) }
    );
    let [(id, _)] = &sent(&effects)[..] else {
        unreachable!()
    };
    let refusal = "the spec is now v3; review it before approving";
    reply(
        &mut app,
        RunReply::Refused {
            request: proto::run_wire::request::DOC_GATE.into(),
            message: refusal.into(),
            request_id: Some(*id),
        },
    );
    assert_eq!(
        screen(&app).message,
        Some((Tone::Refused, refusal.to_owned()))
    );
    app.on_key(key('a'));
    assert_eq!(
        confirm_message(&app),
        "Approve spec v3? Planning starts next."
    );
    assert_eq!(
        gate_action(&app.on_key(key('y'))),
        DocGateAction::Approve { version: Some(3) }
    );
}

/// Ruling T17-2: once the gate the screen shows has closed (approved or rejected
/// elsewhere) or the run waits at another gate, the screen says
/// `the <kind> gate is closed`, and every key but `q` and Esc does nothing.
#[test]
fn a_closed_gate_leaves_only_q_and_esc() {
    let mut planning = design_run(DocGateKind::Spec);
    planning.doc_gate = None;
    planning.state = proto::RunState::Planning;
    let mut at_plan = design_run(DocGateKind::Plan);
    at_plan.doc_gate.as_mut().unwrap().version = 1;
    for (what, run) in [("closed", planning), ("another gate", at_plan)] {
        let mut app = opened(DocGateKind::Spec, 120, 40);
        deliver(&mut app, run);
        assert!(
            sent(&app.screens_tick(Instant::now())).is_empty(),
            "{what}: nothing reloads"
        );
        for c in ['a', 'x', 'c', 'e', 'r', 'b', 'd', 'f', 'g', 'j'] {
            let effects = app.on_key(key(c));
            assert!(effects.is_empty(), "{what}: {c}");
            assert!(app.modal.is_none(), "{what}: {c}");
            assert_eq!(screen(&app).pane, DocPane::Document, "{what}: {c}");
            assert_eq!(screen(&app).scroll, 0, "{what}: {c}");
        }
        assert_eq!(
            app.doc_gate_closed().as_deref(),
            Some("the spec gate is closed"),
            "{what}"
        );
        let rows = rows(&app, 120, 40);
        assert!(
            rows[37].starts_with("│the spec gate is closed "),
            "{what}: {rows:#?}"
        );
        let bar = rows[39].trim_end();
        assert!(
            bar.starts_with(" REVIEW ") && bar.ends_with("  esc close"),
            "{what}: {bar:?}"
        );
        assert!(
            !bar.contains("approve") && !bar.contains("scroll"),
            "{bar:?}"
        );
        app.on_key(key('q'));
        assert!(app.screen.is_none(), "{what}: q closes");
    }
    // Esc closes it too.
    let mut app = opened(DocGateKind::Spec, 120, 40);
    let mut gone = design_run(DocGateKind::Spec);
    gone.doc_gate = None;
    deliver(&mut app, gone);
    app.on_key(code(KeyCode::Esc));
    assert!(app.screen.is_none());
}

/// Review m3: the keys the client refuses itself say why on the message line, send
/// nothing and open nothing: `r` away from the brainstorm gate, `b` at it, `g` away
/// from it; and `e` before the document has loaded.
#[test]
fn the_client_refuses_r_b_g_and_an_early_e_in_words() {
    let cases = [
        (
            DocGateKind::Spec,
            'r',
            "rethink is only for the brainstorm gate",
        ),
        (
            DocGateKind::Brainstorm,
            'b',
            "back is only for the spec and plan gates",
        ),
        (
            DocGateKind::Spec,
            'g',
            "the drafts are the brainstorm gate's",
        ),
    ];
    for (kind, c, text) in cases {
        let mut app = opened(kind, 120, 40);
        assert!(sent(&app.on_key(key(c))).is_empty(), "{c}");
        assert!(app.modal.is_none(), "{c}");
        assert_eq!(screen(&app).pane, DocPane::Document, "{c}");
        assert_eq!(
            screen(&app).message,
            Some((Tone::Note, text.to_owned())),
            "{c}"
        );
        let rows = rows(&app, 120, 40);
        assert!(rows[37].starts_with(&format!("│{text} ")), "{c}: {rows:#?}");
    }
    let mut app = app_with(design_run(DocGateKind::Spec), 120, 40);
    app.open_doc_gate(RUN_ID);
    app.on_key(key('e'));
    assert!(app.modal.is_none());
    assert_eq!(
        screen(&app).message,
        Some((
            Tone::Note,
            "the document is not loaded; e edits it once it is".to_owned()
        ))
    );
}

/// Review m3: the brainstorm gate's approve page names what starts next, and the
/// approve sent names v1.
#[test]
fn the_brainstorm_approve_page_names_the_spec() {
    let mut app = opened(DocGateKind::Brainstorm, 120, 40);
    app.on_key(key('a'));
    assert_eq!(
        confirm_message(&app),
        "Approve brainstorm v1? Spec starts next."
    );
    assert_eq!(
        gate_action(&app.on_key(key('y'))),
        DocGateAction::Approve { version: Some(1) }
    );
}

/// Review m3: Enter on the gate's alert opens the action menu with `review document`
/// selected, and its Enter opens the screen, asking for the gate's version.
#[test]
fn enter_on_a_gate_alert_opens_the_screen_through_the_menu() {
    let mut run = design_run(DocGateKind::Spec);
    let info = |kind: ActionKind, label: &str| ActionInfo {
        needs: kind.needs(),
        destructive: kind.destructive(),
        effect: format!("{label}: the effect"),
        label: label.into(),
        refused_why: None,
        kind,
    };
    run.actions = vec![
        info(ActionKind::ReviewDoc, "review document"),
        info(ActionKind::Reject, "reject"),
    ];
    let mut app = app_with(run, 120, 40);
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(key('a'));
    assert!(app.alerts_focus.is_some(), "the alerts view");
    assert!(sent(&app.on_key(code(KeyCode::Enter))).is_empty());
    let Some(Modal::Action(flow)) = &app.modal else {
        panic!("the action menu: {:?}", app.modal);
    };
    assert_eq!(flow.items[flow.selected].kind, ActionKind::ReviewDoc);
    let effects = app.on_key(code(KeyCode::Enter));
    assert!(
        matches!(
            &sent(&effects)[..],
            [(
                _,
                RunRequest::ShowDoc {
                    version: Some(2),
                    ..
                }
            )]
        ),
        "{effects:?}"
    );
    assert_eq!(screen(&app).kind, DocGateKind::Spec);
    assert!(app.modal.is_none());
}

/// Review m6: a `ShowDoc` that expires is reported once, by the pane that waited on it,
/// with no `no reply from daemon` toast besides.
#[test]
fn an_expired_show_doc_is_reported_once() {
    let mut app = app_with(design_run(DocGateKind::Spec), 120, 40);
    let effects = app.open_doc_gate(RUN_ID);
    let [(id, _)] = &sent(&effects)[..] else {
        panic!("{effects:?}");
    };
    app.set_reply_sent_at(*id, Instant::now() - Duration::from_secs(31));
    app.on_tick();
    assert_eq!(
        screen(&app).doc,
        DocLoad::Failed(crate::app::replies::NO_REPLY.to_owned())
    );
    assert_eq!(app.toast_text(), None, "no second report");
}
