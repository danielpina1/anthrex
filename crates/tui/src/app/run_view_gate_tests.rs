//! Milestone 9.6 task 18 (DF §6.1, rulings T1-O1 and T17-1): the run view's gate keys in
//! a design run. At a brainstorm or spec gate `a` (and `p`) open the gate screen, which a
//! plain approve could not do (T1-O1 refuses it), and `x` asks on the gate screen's own
//! reject page. At a design run's plan gate `a` asks to approve the version the run view
//! shows, and its `y` sends that version (T17-1); the plan review's `a` is the same key.

use super::*;
use crate::app::doc_gate::DocLoad;
use crate::app::screens::Screen;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::doc_gate::tests::{app_with, design_run, key, rows, sent};
use proto::{DocGateAction, DocGateKind};

fn in_run_view(kind: DocGateKind) -> App {
    let mut app = app_with(design_run(kind), 120, 40);
    app.open_run_view(RUN_ID.into());
    app
}

fn confirm_of(app: &App) -> (String, PendingAction) {
    match &app.modal {
        Some(Modal::Confirm { message, action }) => (message.clone(), action.clone()),
        other => panic!("a confirm page: {other:?}"),
    }
}

#[test]
fn a_at_a_brainstorm_or_spec_gate_opens_the_gate_screen() {
    for kind in [DocGateKind::Brainstorm, DocGateKind::Spec] {
        for c in ['a', 'p'] {
            let mut app = in_run_view(kind);
            let effects = app.on_run_view_key(key(c)).expect("a run view key");
            let Some(Screen::DocGate(s)) = &app.screen else {
                panic!(
                    "{c} at the {kind:?} gate opens the gate screen: {:?}",
                    app.screen
                );
            };
            assert_eq!((s.kind, s.run_id.as_str()), (kind, RUN_ID));
            assert!(matches!(s.doc, DocLoad::Loading(_)));
            let requests = sent(&effects);
            assert!(
                matches!(&requests[..], [(_, RunRequest::ShowDoc { kind: k, .. })]
                    if *k == crate::app::doc_gate::gate_doc(kind)),
                "{requests:?}"
            );
            assert!(app.modal.is_none() && app.plan_review.is_none());
        }
    }
}

#[test]
fn x_at_a_spec_gate_asks_on_the_gate_reject_page() {
    let mut app = in_run_view(DocGateKind::Spec);
    assert!(sent(&app.on_run_view_key(key('x')).unwrap()).is_empty());
    let (message, action) = confirm_of(&app);
    assert_eq!(message, "Reject the spec? This discards run 3f9a.");
    assert_eq!(
        action,
        PendingAction::DocGate {
            run_id: RUN_ID.into(),
            kind: DocGateKind::Spec,
            action: DocGateAction::Reject,
        }
    );
}

#[test]
fn a_at_a_design_plan_gate_approves_the_version_shown() {
    let mut run = design_run(DocGateKind::Plan);
    if let Some(gate) = run.doc_gate.as_mut() {
        gate.version = 3;
    }
    let mut app = app_with(run, 120, 40);
    app.open_run_view(RUN_ID.into());
    assert!(sent(&app.on_run_view_key(key('a')).unwrap()).is_empty());
    let (message, action) = confirm_of(&app);
    assert_eq!(message, "Approve plan v3? The run starts next.");
    let want = DocGateAction::Approve { version: Some(3) };
    assert_eq!(
        action,
        PendingAction::DocGate {
            run_id: RUN_ID.into(),
            kind: DocGateKind::Plan,
            action: want.clone(),
        }
    );
    let effects = app.on_key(key('y'));
    assert!(
        matches!(&sent(&effects)[..], [(_, RunRequest::DocGate { kind: DocGateKind::Plan, action, .. })]
            if *action == want),
        "{effects:?}"
    );
    // The plan review's `a` is the same key.
    let mut app = app_with(design_run(DocGateKind::Plan), 120, 40);
    app.open_run_view(RUN_ID.into());
    assert!(app.on_run_view_key(key('p')).is_some());
    assert!(app.plan_review.is_some());
    app.on_key(key('a'));
    // `design_run`'s plan gate is at v2.
    assert_eq!(confirm_of(&app).0, "Approve plan v2? The run starts next.");
}

#[test]
fn the_run_view_hints_at_a_spec_gate_name_its_keys() {
    let app = in_run_view(DocGateKind::Spec);
    let bar = rows(&app, 120, 40)[39].clone();
    assert_eq!(
        bar,
        " RUN  ⏸ spec v2  a review  x reject  ⏎ open  . actions  f filter: all  esc back"
    );
}

/// Ruling T18-1: the run view draws one node an agent, its glyph its state's (failed
/// `✗`, running live `●` while its window is not listed, done `✓`), and the selected agent's panel names its state,
/// runtime, sessions and, for a reviewer, its document.
#[test]
fn the_run_view_draws_the_design_agents() {
    let mut app = app_with(crate::tree::run_rows::design_tests::with_agents(), 120, 40);
    app.open_run_view(RUN_ID.into());
    let drawn = rows(&app, 120, 40);
    // A node is cut at the canvas's widest box; the inspector says the rest.
    for text in [
        "✗ brainstormer claude",
        "● brainstormer codex  2 s…",
        "✓ doc reviewer spec-r2 co…",
    ] {
        assert!(drawn.iter().any(|r| r.contains(text)), "{text}: {drawn:#?}");
    }
    let key = |label: &str| NodeKey::DesignAgent {
        run: RUN_ID.into(),
        label: label.into(),
    };
    let nav = crate::app::nav_rows_of(&app.windows, &app.runs, &app.tree, app.run_view.as_ref());
    app.tree.select(&nav, key("codex"));
    let drawn = rows(&app, 120, 40).join("\n");
    for text in ["state", "running", "runtime", "codex", "sessions  2"] {
        assert!(drawn.contains(text), "{text}: {drawn}");
    }
    app.tree.select(&nav, key("spec-r2"));
    let drawn = rows(&app, 120, 40).join("\n");
    assert!(drawn.contains("spec, review 2"), "{drawn}");
}

/// Enter on a design agent opens its conversation, as on a scout: here its window is
/// not listed, so the run view says why.
#[test]
fn enter_on_a_design_agent_says_why_there_is_no_conversation() {
    let mut app = app_with(crate::tree::run_rows::design_tests::with_agents(), 120, 40);
    app.open_run_view(RUN_ID.into());
    let key = |label: &str| NodeKey::DesignAgent {
        run: RUN_ID.into(),
        label: label.into(),
    };
    assert!(app.activate_run_node(key("codex")).is_empty());
    assert_eq!(app.toast_text(), Some("window #41 is not listed yet"));
    assert!(app.activate_run_node(key("claude")).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("brainstormer claude has no window yet")
    );
}

/// Ruling T18-3: from round 2 a brainstorm or spec gate's reject page names the round it
/// drops, from the run view and from the gate screen alike.
#[test]
fn a_later_rounds_reject_page_drops_the_round() {
    let mut run = design_run(DocGateKind::Spec);
    run.round = 2;
    let mut app = app_with(run.clone(), 120, 40);
    app.open_run_view(RUN_ID.into());
    app.on_run_view_key(key('x'));
    assert_eq!(
        confirm_of(&app).0,
        "Reject the spec? This drops round 2 of run 3f9a."
    );
    let mut app = app_with(run, 120, 40);
    app.open_doc_gate(RUN_ID);
    app.on_key(key('x'));
    assert_eq!(
        confirm_of(&app).0,
        "Reject the spec? This drops round 2 of run 3f9a."
    );
}
