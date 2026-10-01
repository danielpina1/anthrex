//! Milestone 9.0.6 task 11, decision 17: Enter on an alert opens the action menu on
//! its node with the alert's action selected.

use super::actions::{action, flow};
use super::alerts::every_app;
use super::runs::deliver;
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::AlertKey;
use crate::tree::alert_fixtures::every_source;
use proto::{ActionKind, RunsSnapshot};

/// `every_source` with the daemon's entries on each node an alert points at.
fn with_actions() -> RunsSnapshot {
    let (mut snap, _) = every_source();
    for run in &mut snap.runs {
        match run.run_id.as_str() {
            "b-gate" => {
                run.actions = vec![
                    action(ActionKind::Approve, "approve", None),
                    action(ActionKind::Reject, "reject", None),
                ];
            }
            "c-held" => {
                run.actions = vec![
                    action(ActionKind::Pause, "pause", None),
                    action(
                        ActionKind::RejectHold {
                            hold: "epic:ui".into(),
                        },
                        "reject hold epic:ui",
                        None,
                    ),
                    action(
                        ActionKind::ApproveHold {
                            hold: "epic:ui".into(),
                        },
                        "approve hold epic:ui",
                        None,
                    ),
                ];
                for task in &mut run.tasks {
                    task.actions = vec![
                        action(ActionKind::Answer, "answer", None),
                        action(ActionKind::Retry, "retry", None),
                    ];
                }
            }
            "d-bare" => {
                run.tasks[0].actions = vec![
                    action(ActionKind::Retry, "retry", None),
                    action(ActionKind::Answer, "answer", None),
                ];
            }
            "e-halt" => {
                run.actions = vec![
                    action(ActionKind::Cancel, "cancel run", None),
                    action(ActionKind::Resume, "resume halted run", None),
                ];
            }
            "f-done" => {
                run.actions = vec![
                    action(ActionKind::Discard, "discard", None),
                    action(ActionKind::Accept, "accept", None),
                ];
            }
            _ => {}
        }
    }
    snap
}

fn app_with_actions() -> App {
    let mut app = every_app();
    deliver(&mut app, with_actions());
    app
}

fn enter_on(app: &mut App, key: &AlertKey) -> Vec<Effect> {
    prefix(app);
    tap(app, KeyCode::Char('a'));
    for _ in 0..20 {
        if app
            .alerts_focus
            .as_ref()
            .and_then(|f| f.selected.clone())
            .as_ref()
            == Some(key)
        {
            return tap(app, KeyCode::Enter);
        }
        tap(app, KeyCode::Char('j'));
    }
    panic!("{key:?} is not listed");
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

fn selected_kind(app: &App) -> ActionKind {
    let f = flow(app);
    f.items[f.selected].kind.clone()
}

#[test]
fn enter_on_each_alert_preselects_its_action() {
    let cases: Vec<(AlertKey, &str, ActionTarget, ActionKind)> = vec![
        (
            AlertKey::Accept("f-done".into()),
            "f-done",
            ActionTarget::Run,
            ActionKind::Accept,
        ),
        (
            AlertKey::Blocked {
                run: "d-bare".into(),
                task: "t1".into(),
            },
            "d-bare",
            ActionTarget::Task("t1".into()),
            ActionKind::Answer,
        ),
        (
            AlertKey::Blocked {
                run: "c-held".into(),
                task: "t1".into(),
            },
            "c-held",
            ActionTarget::Task("t1".into()),
            ActionKind::Retry,
        ),
        (
            AlertKey::Halted("e-halt".into()),
            "e-halt",
            ActionTarget::Run,
            ActionKind::Resume,
        ),
        (
            AlertKey::Gate("b-gate".into()),
            "b-gate",
            ActionTarget::Run,
            ActionKind::ReviewPlan,
        ),
        (
            AlertKey::Hold {
                run: "c-held".into(),
                hold: "epic:ui".into(),
            },
            "c-held",
            ActionTarget::Run,
            ActionKind::ApproveHold {
                hold: "epic:ui".into(),
            },
        ),
    ];
    for (key, run, target, kind) in cases {
        let mut app = app_with_actions();
        assert!(enter_on(&mut app, &key).is_empty());
        let f = flow(&app);
        assert_eq!((f.run_id.as_str(), &f.target), (run, &target), "{key:?}");
        assert_eq!(selected_kind(&app), kind, "{key:?}");
        assert_eq!(app.alerts_focus, None, "{key:?}: the focus is left");
        assert!(!app.keymap.alerts_mode(), "{key:?}");
    }
}

/// P1 keeps its meaning (decision 17); a proposal opens the Profile screen (task 13,
/// F26: `profile_screen.rs::enter_on_a_proposal_alert_opens_the_profile_screen`).
#[test]
fn the_orchestrator_and_proposal_alerts_open_no_menu() {
    let mut app = app_with_actions();
    let effects = enter_on(&mut app, &AlertKey::Orchestrator("a-attn".into()));
    assert!(
        !effects.is_empty(),
        "the orchestrator's window is subscribed"
    );
    assert!(app.modal.is_none());
    assert_eq!(app.focused, Some(11));

    let mut app = app_with_actions();
    assert!(!enter_on(&mut app, &AlertKey::Proposal("/r/shop".into())).is_empty());
    assert!(app.modal.is_none());
    assert!(app.screen.is_some());
    assert_eq!(app.toast_text(), None);
}

#[test]
fn a_preselection_that_left_the_list_selects_the_first_entry() {
    let mut app = app_with_actions();
    let mut snap = with_actions();
    let halted = snap.runs.iter_mut().find(|r| r.run_id == "e-halt").unwrap();
    halted.actions = vec![action(ActionKind::Cancel, "cancel run", None)];
    deliver(&mut app, snap);
    assert!(enter_on(&mut app, &AlertKey::Halted("e-halt".into())).is_empty());
    assert_eq!(flow(&app).selected, 0);
    assert_eq!(selected_kind(&app), ActionKind::Cancel);
}
