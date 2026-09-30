//! M9.0.5.8: the Alerts box's focus (decision 21): `C-b a`, `j`/`k`, Enter on each
//! priority and `Esc`. Split from `alerts.rs`, which computes the alerts.

use super::alerts::{every_app, every_source};
use super::runs::{app_with_runs, deliver};
use super::*;
use crate::app::{AlertKey, ReviewTarget, TreeInput};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::snapshot;
use proto::RunState;

#[test]
fn c_b_a_focuses_and_shows_a_hidden_sidebar() {
    let mut app = every_app();
    app.sidebar_visible = false;
    focus(&mut app);
    assert!(app.sidebar_visible);
    assert!(app.keymap.alerts_mode());
    assert_eq!(
        selected(&app),
        Some(AlertKey::Orchestrator("a-attn".into()))
    );
    // With no alert the box still takes the focus, with nothing selected.
    let mut empty = app_with_runs(vec![], snapshot(1, vec![]));
    focus(&mut empty);
    assert_eq!(selected(&empty), None);
    assert!(tap(&mut empty, KeyCode::Enter).is_empty());
    assert_eq!(
        empty.alerts_focus, None,
        "Enter on nothing leaves the focus"
    );
}

#[test]
fn j_k_move_and_selection_follows_identity() {
    let mut app = every_app();
    focus(&mut app);
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert!(tap(&mut app, KeyCode::Down).is_empty());
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(selected(&app), Some(AlertKey::Gate("b-gate".into())));
    assert!(tap(&mut app, KeyCode::Char('k')).is_empty());
    assert!(tap(&mut app, KeyCode::Up).is_empty());
    assert_eq!(
        selected(&app),
        Some(AlertKey::Orchestrator("b-gate".into()))
    );
    // The ends hold.
    for _ in 0..3 {
        tap(&mut app, KeyCode::Char('k'));
    }
    assert_eq!(
        selected(&app),
        Some(AlertKey::Orchestrator("a-attn".into()))
    );
    for _ in 0..20 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(selected(&app), Some(AlertKey::Proposal("/r/shop".into())));

    // A reorder: `a-attn`'s window stops asking, so every alert moves up one; the
    // selection (`b-gate`'s plan) follows its alert.
    let mut app = every_app();
    focus(&mut app);
    for _ in 0..3 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(selected(&app), Some(AlertKey::Gate("b-gate".into())));
    let (_, mut windows) = every_source();
    windows[1].status = Status::Working;
    app.on_daemon(DaemonMsg::WindowsChanged { windows });
    assert_eq!(selected(&app), Some(AlertKey::Gate("b-gate".into())));

    // A resolved selection gives way to the alert now at its position: the gate
    // approved, `c-held`'s hold takes position 2 (now that `a-attn` is gone).
    let (mut snap, _) = every_source();
    let b = snap.runs.iter_mut().find(|r| r.run_id == "b-gate").unwrap();
    b.state = RunState::Running;
    deliver(&mut app, snap);
    assert_eq!(
        selected(&app),
        Some(AlertKey::Hold {
            run: "c-held".into(),
            hold: "epic:ui".into()
        })
    );
    // Everything resolved: nothing is selected, the focus stays.
    deliver(&mut app, snapshot(1, vec![]));
    let (_, windows) = every_source();
    app.on_daemon(DaemonMsg::WindowsChanged {
        windows: windows[..1].to_vec(),
    });
    assert_eq!(selected(&app), None);
    assert!(app.alerts_focus.is_some());
}

/// Moves the focus to the alert `key`.
fn select_alert(app: &mut App, key: &AlertKey) {
    focus(app);
    for _ in 0..20 {
        if selected(app).as_ref() == Some(key) {
            return;
        }
        tap(app, KeyCode::Char('j'));
    }
    panic!("{key:?} is not listed");
}

#[test]
fn enter_on_each_priority() {
    // P1: focus the orchestrator's window, tree mode off.
    let mut app = every_app();
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('t'));
    assert!(app.tree_input.is_some());
    select_alert(&mut app, &AlertKey::Orchestrator("b-gate".into()));
    let effects = tap(&mut app, KeyCode::Enter);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Send(ClientMsg::Subscribe { window_id: 12, .. }), ..]
        ),
        "{effects:?}"
    );
    assert_eq!(app.focused, Some(12));
    assert_eq!(app.tree_input, None);
    assert!(!app.keymap.tree_mode());
    assert_eq!(app.alerts_focus, None);
    assert!(!app.keymap.alerts_mode());

    // P2: the review on that gate, and on that hold.
    let mut app = every_app();
    select_alert(&mut app, &AlertKey::Gate("b-gate".into()));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let review = app.plan_review.as_ref().expect("the review is open");
    assert_eq!(
        (review.run_id.as_str(), &review.target),
        ("b-gate", &ReviewTarget::Gate)
    );
    assert!(app.keymap.review_mode());
    assert!(!app.keymap.alerts_mode());
    let hold_key = AlertKey::Hold {
        run: "c-held".into(),
        hold: "epic:ui".into(),
    };
    let mut app = every_app();
    select_alert(&mut app, &hold_key);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    let review = app.plan_review.as_ref().expect("the review is open");
    assert_eq!(review.target, ReviewTarget::Hold("epic:ui".into()));
    assert_eq!(review.selected.as_deref(), Some("t5"));

    // P3: the run view with the task selected; a halted run's root.
    let mut app = every_app();
    select_alert(
        &mut app,
        &AlertKey::Blocked {
            run: "d-bare".into(),
            task: "t1".into(),
        },
    );
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("d-bare")
    );
    assert_eq!(
        app.tree.selected,
        Some(NodeKey::Task {
            run: "d-bare".into(),
            id: "t1".into()
        })
    );
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    let mut app = every_app();
    select_alert(&mut app, &AlertKey::Halted("e-halt".into()));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("e-halt")
    );
    assert_eq!(app.tree.selected, Some(NodeKey::Run("e-halt".into())));

    // P4: the run view on the run; a proposal toasts.
    let mut app = every_app();
    select_alert(&mut app, &AlertKey::Accept("f-done".into()));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some("f-done")
    );
    assert_eq!(app.tree.selected, Some(NodeKey::Run("f-done".into())));
    let mut app = every_app();
    select_alert(&mut app, &AlertKey::Proposal("/r/shop".into()));
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(
        app.toast_text(),
        Some(
            "profile proposal for shop: run anthrex profile show --proposed, then \
             anthrex profile confirm or reject, in that project"
        )
    );
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.run_view, None);
}

#[test]
fn esc_leaves_the_focus() {
    let mut app = every_app();
    focus(&mut app);
    tap(&mut app, KeyCode::Char('j'));
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.alerts_focus, None);
    assert!(!app.keymap.alerts_mode());
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.run_view, None);
    // Bare keys reach the terminal again.
    assert_eq!(
        tap(&mut app, KeyCode::Char('j')),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"j".to_vec()
        })]
    );
}

#[test]
fn c_b_a_under_the_review_keeps_the_review() {
    let mut app = every_app();
    app.open_plan_review("b-gate".into(), ReviewTarget::Gate);
    focus_attempt(&mut app);
    assert_eq!(app.alerts_focus, None);
    assert!(!app.keymap.alerts_mode());
    assert_eq!(app.toast_text(), Some("leave the plan review first (esc)"));
}

fn focus_attempt(app: &mut App) {
    prefix(app);
    assert!(tap(app, KeyCode::Char('a')).is_empty());
}

#[test]
fn alert_keys_send_no_input() {
    let mut app = every_app();
    let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
    codes.extend([
        KeyCode::Tab,
        KeyCode::Backspace,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Delete,
        KeyCode::F(1),
    ]);
    for code in codes {
        for mods in [KeyModifiers::NONE, KeyModifiers::SHIFT, KeyModifiers::ALT] {
            focus(&mut app);
            let effects = press(&mut app, code, mods);
            assert!(
                !effects
                    .iter()
                    .any(|e| matches!(e, Effect::Send(ClientMsg::Input { .. }))),
                "{code:?} {mods:?}: {effects:?}"
            );
            // Leave whatever that key opened, for the next one.
            app.modal = None;
            app.close_plan_review();
            app.alerts_focus = None;
            app.keymap.set_alerts_mode(false);
        }
    }
    // The prefix twice sends nothing either.
    focus(&mut app);
    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL),
        vec![]
    );
}

fn focus(app: &mut App) {
    prefix(app);
    assert!(tap(app, KeyCode::Char('a')).is_empty());
    assert!(app.alerts_focus.is_some(), "C-b a focused the alerts");
}

fn selected(app: &App) -> Option<AlertKey> {
    app.alerts_focus.as_ref().and_then(|f| f.selected.clone())
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// Review: a paste while the box has the keys reaches nothing under it, over the
/// terminal or over the conversation's search.
#[test]
fn a_paste_while_the_alerts_are_focused_sends_nothing() {
    let mut app = every_app();
    focus(&mut app);
    assert_eq!(app.on_paste("rm -rf x\n".into()), vec![]);
    assert!(app.alerts_focus.is_some());
    tap(&mut app, KeyCode::Esc);
    assert_eq!(
        app.on_paste("ls\n".into()),
        vec![Effect::Send(ClientMsg::Input {
            window_id: 1,
            bytes: b"ls\r".to_vec()
        })]
    );
}

#[test]
fn a_paste_while_the_alerts_are_focused_over_the_conversation_changes_nothing() {
    let mut app = every_app();
    let _ = app.open_conversation(1);
    assert!(app.conversation.is_open());
    tap(&mut app, KeyCode::Char('/'));
    let search = |app: &App| {
        app.conversation
            .search()
            .map(|s| (s.query.clone(), s.typing))
    };
    assert_eq!(search(&app), Some((String::new(), true)));
    focus(&mut app);
    assert_eq!(app.on_paste("secret".into()), vec![]);
    assert_eq!(
        search(&app),
        Some((String::new(), true)),
        "the hidden search"
    );
}

/// Review: every alerts key repairs the selection first, so a window list that has
/// not been through `replace_windows` (the repair's other caller) cannot leave `j`
/// stepping from a resolved alert's stale position.
#[test]
fn an_alerts_key_repairs_the_selection_first() {
    let mut app = every_app();
    focus(&mut app);
    for _ in 0..3 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert_eq!(selected(&app), Some(AlertKey::Gate("b-gate".into())));
    // `a-attn`'s window stops asking, behind the repair's back: every alert moves up
    // one, so `b-gate`'s plan is now at position 2, not 3.
    app.windows[1].status = Status::Working;
    assert!(tap(&mut app, KeyCode::Char('j')).is_empty());
    assert_eq!(
        selected(&app),
        Some(AlertKey::Hold {
            run: "c-held".into(),
            hold: "epic:ui".into()
        }),
        "`j` stepped from the repaired position"
    );
}
