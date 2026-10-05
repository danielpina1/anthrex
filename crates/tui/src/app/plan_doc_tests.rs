//! Milestone 9.6 task 18 (decisions 34, 35; DF §6.1): the plan review's Plan doc tab.
//! Tab at a design run's plan gate switches between the task list and `plan.md` (one
//! tagged `ShowDoc`, kept while the version stands); `c` and `b` take their note in the
//! gate screen's editor and send the gate screen's `DocGate` after its pages; the tab
//! follows the gate to a newer version. A run without the flow has no tab.

use crate::app::doc_gate::DocLoad;
use crate::app::plan_review::ReviewTarget;
use crate::app::{App, Effect, Modal, PendingAction};
use crate::doc_note::NoteFor;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture, task};
use crate::ui::doc_gate::tests::{app_with, code, design_run, key, reply, sent, view};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{
    DaemonMsg, DocAuthor, DocGateAction, DocGateKind, DocInfo, DocKind, RunReply, RunRequest, Size,
    TaskState,
};

/// `plan.md` as the engine renders it: a stage, two tasks, and the coverage table with
/// one requirement no task covers.
pub(crate) const PLAN: &str = "\
# Plan: password reset

## Stage 1

### t1 reset token model
Covers: R1
Size: M · Route: claude · Tests: tdd

Files: src/token.rs

### t2 reset endpoint
Covers: none
Size: S · Route: claude · Tests: tdd

## Coverage

| Requirement | Tasks |
|---|---|
| R1 | t1 |
| R2 | none |
";

/// The design run waiting at its plan gate (v`version`), with two tasks and its plan
/// versions listed.
pub(crate) fn plan_run(version: u32) -> proto::RunInfo {
    let mut run = design_run(DocGateKind::Plan);
    if let Some(gate) = run.doc_gate.as_mut() {
        gate.version = version;
        // The plan's review left nothing for the gate: a test that wants its lines
        // sets them (the final fix wave's FW-19 check).
        gate.disputed.clear();
        gate.changes_summary.clear();
    }
    run.tasks = vec![
        task("t1", "reset token model", Size::M, TaskState::Pending),
        task("t2", "reset endpoint", Size::S, TaskState::Pending),
    ];
    for n in 1..=version {
        run.docs.push(DocInfo {
            kind: DocKind::Plan,
            version: n,
            author: DocAuthor::Engine,
            reason: "reviewed".into(),
            bytes: 300,
            requirements: Vec::new(),
        });
    }
    run
}

/// The plan review open on `run`'s gate, on a `w`×`h` terminal.
pub(crate) fn reviewing(run: proto::RunInfo, w: u16, h: u16) -> App {
    let mut app = app_with(run, w, h);
    assert!(
        app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate)
            .is_empty()
    );
    app
}

/// [`reviewing`] on the Plan doc tab with `text` loaded as v`version`.
pub(crate) fn on_tab(run: proto::RunInfo, text: &str, w: u16, h: u16) -> App {
    let version = run.doc_gate.as_ref().map_or(1, |g| g.version);
    let mut app = reviewing(run, w, h);
    let effects = app.on_key(code(KeyCode::Tab));
    let [(id, _)] = sent(&effects)[..] else {
        panic!("one ShowDoc: {effects:?}");
    };
    reply(
        &mut app,
        RunReply::Doc {
            doc: Box::new(view(DocKind::Plan, version, text)),
            request_id: Some(id),
        },
    );
    app
}

fn doc(app: &App) -> Option<&super::PlanDoc> {
    app.plan_review.as_ref().and_then(|r| r.doc.as_ref())
}

fn shown(app: &App) -> bool {
    doc(app).is_some_and(|d| d.shown)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        assert!(sent(&app.on_key(key(c))).is_empty());
    }
}

/// The one tagged `DocGate` among `effects`.
fn gate_sent(effects: &[Effect]) -> (DocGateKind, DocGateAction) {
    match &sent(effects)[..] {
        [(_, RunRequest::DocGate { run, kind, action })] if run == RUN_ID => {
            (*kind, action.clone())
        }
        other => panic!("one DocGate: {other:?}"),
    }
}

#[test]
fn tab_switches_the_plan_review_to_the_plan_doc_and_back() {
    let mut app = reviewing(plan_run(2), 120, 40);
    let effects = app.on_key(code(KeyCode::Tab));
    let requests = sent(&effects);
    let [
        (
            id,
            RunRequest::ShowDoc {
                kind: DocKind::Plan,
                version: Some(2),
                diff: false,
                ..
            },
        ),
    ] = requests[..]
    else {
        panic!("one ShowDoc of plan v2: {requests:?}");
    };
    assert!(shown(&app));
    assert_eq!(doc(&app).map(|d| &d.load), Some(&DocLoad::Loading(id)));
    reply(
        &mut app,
        RunReply::Doc {
            doc: Box::new(view(DocKind::Plan, 2, PLAN)),
            request_id: Some(id),
        },
    );
    assert!(matches!(
        doc(&app).map(|d| &d.load),
        Some(DocLoad::Ready(_))
    ));
    // `j` scrolls the document, not the task list.
    let selected = app.plan_review.as_ref().and_then(|r| r.selected.clone());
    assert!(app.on_key(key('j')).is_empty());
    assert_eq!(doc(&app).map(|d| d.scroll), Some(1));
    assert_eq!(
        app.plan_review.as_ref().and_then(|r| r.selected.clone()),
        selected
    );
    // Tab back to the list, and again to the document, asked once.
    assert!(app.on_key(code(KeyCode::Tab)).is_empty());
    assert!(!shown(&app));
    assert!(app.on_key(key('j')).is_empty());
    assert_eq!(
        app.plan_review.as_ref().and_then(|r| r.selected.as_deref()),
        Some("t2")
    );
    assert!(sent(&app.on_key(code(KeyCode::Tab))).is_empty());
    assert!(shown(&app));
    // Esc closes the review from either tab.
    app.on_key(code(KeyCode::Esc));
    assert!(app.plan_review.is_none());
}

#[test]
fn a_run_without_the_flow_has_no_plan_doc_tab() {
    let (snap, windows) = gate_fixture();
    let mut app = App::new(
        windows,
        "/tmp".into(),
        crate::settings::UiSettings::default(),
    );
    let _ = app.set_terminal_size(120, 40);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app.open_plan_review(RUN_ID.into(), ReviewTarget::Gate);
    for c in [code(KeyCode::Tab), key('c'), key('b')] {
        assert!(app.on_key(c).is_empty());
    }
    assert!(doc(&app).is_none() && app.modal.is_none());
}

#[test]
fn c_and_b_send_the_gate_screens_requests_after_its_pages() {
    for on_doc in [false, true] {
        let mut app = if on_doc {
            on_tab(plan_run(2), PLAN, 120, 40)
        } else {
            reviewing(plan_run(2), 120, 40)
        };
        // `c`: the note editor, sent on its Ctrl-S, review again left at no.
        assert!(app.on_key(key('c')).is_empty());
        let Some(Modal::DocNote(form)) = &app.modal else {
            panic!("the note editor: {:?}", app.modal);
        };
        assert_eq!(
            (form.purpose, form.kind, form.version),
            (NoteFor::Changes, DocGateKind::Plan, 2)
        );
        type_text(&mut app, "split t1");
        let (kind, action) = gate_sent(&app.on_key(ctrl('s')));
        assert_eq!(kind, DocGateKind::Plan);
        assert_eq!(
            action,
            DocGateAction::Changes {
                note: "split t1".into(),
                review: false
            }
        );
        app.modal = None;
        // `b`: the note, then the gate screen's page, then `y`.
        assert!(app.on_key(key('b')).is_empty());
        type_text(&mut app, "R2 is wrong");
        assert!(sent(&app.on_key(ctrl('s'))).is_empty());
        let Some(Modal::Confirm { message, action }) = &app.modal else {
            panic!("the back page: {:?}", app.modal);
        };
        assert_eq!(message, "Go back to the spec? The plan is set aside.");
        let back = DocGateAction::Back {
            note: "R2 is wrong".into(),
        };
        assert_eq!(
            *action,
            PendingAction::DocGate {
                run_id: RUN_ID.into(),
                kind: DocGateKind::Plan,
                action: back.clone(),
            }
        );
        assert_eq!(gate_sent(&app.on_key(key('y'))), (DocGateKind::Plan, back));
    }
}

/// Final fix wave FW-75 (WB-D m1): the client's refusal is the daemon's text, naming
/// the gate's version as the daemon does (`plan v2` while v2 is revised).
#[test]
fn while_the_plan_is_revised_c_and_b_say_so() {
    let mut run = plan_run(2);
    if let Some(gate) = run.doc_gate.as_mut() {
        gate.revising = Some("split t1".into());
    }
    let mut app = reviewing(run, 120, 40);
    for c in ['c', 'b'] {
        assert!(app.on_key(key(c)).is_empty());
        assert!(app.modal.is_none(), "{c}");
        assert_eq!(
            app.toast_text(),
            Some("the orchestrator is revising plan v2; wait for it")
        );
    }
}

#[test]
fn the_tab_follows_the_gate_to_a_newer_version() {
    let mut app = on_tab(plan_run(1), PLAN, 120, 40);
    assert!(app.screens_tick(std::time::Instant::now()).is_empty());
    let (mut snap, _) = gate_fixture();
    snap.runs = vec![plan_run(2)];
    snap.revision += 1;
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    let effects = app.screens_tick(std::time::Instant::now());
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
    assert_eq!(doc(&app).map(|d| d.version), Some(2));
    assert_eq!(doc(&app).map(|d| d.scroll), Some(0));
}

#[test]
fn an_expired_load_says_so_once() {
    let mut app = reviewing(plan_run(1), 120, 40);
    let effects = app.on_key(code(KeyCode::Tab));
    let [(id, _)] = sent(&effects)[..] else {
        panic!("{effects:?}");
    };
    assert!(app.doc_show_failed(id, crate::app::replies::NO_REPLY));
    assert_eq!(
        doc(&app).map(|d| &d.load),
        Some(&DocLoad::Failed(crate::app::replies::NO_REPLY.into()))
    );
}

/// Review m2: on the Plan doc tab `e` and `d` would edit or drop a task the tab hides,
/// so they are refused there with a line naming the tasks tab; on the task list they
/// are the review's own again.
#[test]
fn e_and_d_on_the_tab_point_to_the_tasks_tab() {
    let mut app = on_tab(plan_run(2), PLAN, 120, 40);
    for c in ['e', 'd'] {
        assert!(app.on_key(key(c)).is_empty(), "{c}");
        assert!(app.modal.is_none(), "{c}: {:?}", app.modal);
        assert_eq!(app.toast_text(), Some(super::TASKS_TAB_TEXT));
        assert!(shown(&app), "{c} leaves the tab shown");
    }
    assert!(app.on_key(code(KeyCode::Tab)).is_empty());
    assert!(!shown(&app));
    app.on_key(key('e'));
    assert!(app.modal.is_some(), "e edits a task on the task list");
}
