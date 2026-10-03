//! Milestone 9.2 task M9.2.15: the run view of a `pr`-mode run is watch-only
//! (decision 42): no key merges, approves on the host or replies; the action menu
//! offers what the daemon lists (no accept; discard only once complete, decisions
//! 38–39 and ruling R-1); and the "ready to accept" alert never shows (ruling R-13).

use super::actions::action;
use super::alerts::listed;
use super::runs::{app_with_runs, open_run_view};
use super::*;
use crate::actions_request::ActionTarget;
use crate::tree::NodeKey;
use crate::tree::pr_fixtures::pr_fixture;
use crate::tree::run_fixtures::{RUN_ID, three_task_fixture};
use proto::{ActionKind, FinishAction, PlanEdit, RunRequest, RunState, RunsSnapshot};

/// The `pr` fixture with the actions the daemon lists for it: running, pause and
/// cancel on the run (no accept or discard: `engine/actions/mod.rs`, decisions 38–39);
/// complete, discard alone (ruling R-1). Its stage and tasks carry theirs.
fn pr_snapshot(state: RunState) -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = pr_fixture();
    let run = &mut snap.runs[0];
    run.state = state;
    run.actions = match state {
        RunState::Complete => vec![action(ActionKind::Discard, "discard", None)],
        _ => vec![
            action(ActionKind::Pause, "pause", None),
            action(ActionKind::Cancel, "cancel run", None),
        ],
    };
    for stage in &mut run.stages {
        stage.actions = vec![action(
            ActionKind::MessageStage { stage: stage.n },
            "message stage",
            None,
        )];
    }
    for task in &mut run.tasks {
        task.actions = vec![
            action(ActionKind::Message, "message", None),
            action(ActionKind::CancelTask, "cancel task", None),
        ];
    }
    (snap, windows)
}

/// The run view on the `pr` fixture, `selected` selected.
fn pr_view(state: RunState, selected: &NodeKey) -> App {
    let (snap, windows) = pr_snapshot(state);
    let mut app = app_with_runs(windows, snap);
    open_run_view(&mut app, RUN_ID);
    let rows = crate::app::nav_rows_of(&app.windows, &app.runs, &app.tree, app.run_view.as_ref());
    app.tree.select(&rows, selected.clone());
    app
}

/// A request that would land something on the host or the base, or speak there: an
/// accept (a merge into the base), `run deliver` and `run watch` (host operations the
/// CLI sends), and a `reply_comment` edit (a reply on GitHub).
fn names_a_host_action(effect: &Effect) -> bool {
    let request = match effect {
        Effect::Send(ClientMsg::Run(request))
        | Effect::Send(ClientMsg::RunTagged { request, .. }) => request,
        _ => return false,
    };
    match request {
        RunRequest::Finish { action, .. } => *action == FinishAction::Accept,
        RunRequest::Deliver { .. } | RunRequest::Watch { .. } => true,
        RunRequest::Edit { edits, .. } => edits
            .iter()
            .any(|edit| matches!(edit, PlanEdit::ReplyComment { .. })),
        _ => false,
    }
}

/// Every key the run view's keymap reads, and the keys a menu or a page reads after
/// it: each printable character, the named keys, and the prefix.
fn every_key() -> Vec<KeyEvent> {
    let mut keys: Vec<KeyEvent> = (0x20u8..=0x7e)
        .map(|b| key(KeyCode::Char(char::from(b)), KeyModifiers::NONE))
        .collect();
    for code in [
        KeyCode::Enter,
        KeyCode::Esc,
        KeyCode::Tab,
        KeyCode::BackTab,
        KeyCode::Up,
        KeyCode::Down,
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Home,
        KeyCode::End,
        KeyCode::Backspace,
        KeyCode::Delete,
    ] {
        keys.push(key(code, KeyModifiers::NONE));
    }
    keys.push(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    keys.push(key(KeyCode::Char('j'), KeyModifiers::CONTROL));
    keys
}

/// Decision 42: the view is watch-only. Every key, from the run, a stage, a task and a
/// fix task, followed by the keys that take a menu's first entry or move to the next and
/// confirm (`⏎`, `j`, `y`), running and complete: no effect names a host action.
#[test]
fn no_key_merges_approves_or_replies() {
    let nodes = [
        NodeKey::Run(RUN_ID.into()),
        NodeKey::Stage {
            run: RUN_ID.into(),
            n: 2,
        },
        NodeKey::Task {
            run: RUN_ID.into(),
            id: "t2".into(),
        },
        NodeKey::Task {
            run: RUN_ID.into(),
            id: "fix3".into(),
        },
    ];
    let then = [
        vec![],
        vec![KeyCode::Enter, KeyCode::Char('y')],
        vec![KeyCode::Char('j'), KeyCode::Enter, KeyCode::Char('y')],
        vec![KeyCode::Down, KeyCode::Down, KeyCode::Enter, KeyCode::Enter],
    ];
    let mut tried = 0;
    for state in [RunState::Running, RunState::Complete] {
        for node in &nodes {
            for first in every_key() {
                for rest in &then {
                    let mut app = pr_view(state, node);
                    let mut effects = app.on_key(first);
                    for code in rest {
                        effects.extend(app.on_key(key(*code, KeyModifiers::NONE)));
                    }
                    let host: Vec<&Effect> =
                        effects.iter().filter(|e| names_a_host_action(e)).collect();
                    assert!(
                        host.is_empty(),
                        "{state:?} {node:?} {first:?} then {rest:?}: {host:?}"
                    );
                    tried += 1;
                }
            }
        }
    }
    assert_eq!(tried, 2 * 4 * every_key().len() * 4);
}

/// Decisions 38–39 and ruling R-1, through the client: the menu lists what the daemon
/// computed, so a running `pr` run offers neither accept nor discard, and a complete one
/// discard alone, which confirms on `y` and sends `run discard`.
#[test]
fn a_pr_runs_menu_has_no_accept_and_discard_only_once_complete() {
    let run = NodeKey::Run(RUN_ID.into());
    let kinds = |app: &App| match &app.modal {
        Some(Modal::Action(flow)) => {
            assert_eq!(flow.target, ActionTarget::Run);
            flow.items
                .iter()
                .map(|a| a.kind.clone())
                .collect::<Vec<_>>()
        }
        other => panic!("no menu: {other:?}"),
    };
    let mut app = pr_view(RunState::Running, &run);
    assert!(press(&mut app, KeyCode::Char('.'), KeyModifiers::NONE).is_empty());
    let listed = kinds(&app);
    assert!(!listed.contains(&ActionKind::Accept), "{listed:?}");
    assert!(!listed.contains(&ActionKind::Discard), "{listed:?}");
    let mut app = pr_view(RunState::Complete, &run);
    assert!(press(&mut app, KeyCode::Char('.'), KeyModifiers::NONE).is_empty());
    let listed = kinds(&app);
    assert!(!listed.contains(&ActionKind::Accept), "{listed:?}");
    assert!(listed.contains(&ActionKind::Discard), "{listed:?}");
    let at = listed
        .iter()
        .position(|k| *k == ActionKind::Discard)
        .unwrap();
    for _ in 0..at {
        press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
    }
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    // Destructive: bare Enter never confirms it.
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    let effects = press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
    let sent: Vec<&RunRequest> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { request, .. }) => Some(request),
            _ => None,
        })
        .collect();
    assert!(
        matches!(
            sent.as_slice(),
            [RunRequest::Finish {
                action: FinishAction::Discard,
                ..
            }]
        ),
        "{effects:?}"
    );
}

/// Ruling R-13: a complete `pr` run raises no "ready to accept" alert (its pull
/// requests are merged on GitHub, never accepted); a complete local run still does.
#[test]
fn a_complete_pr_run_is_never_ready_to_accept() {
    let (snap, windows) = pr_snapshot(RunState::Complete);
    let app = app_with_runs(windows, snap);
    let shown = listed(&app);
    assert!(
        shown.iter().all(|(_, _, text)| !text.contains("accept")),
        "{shown:?}"
    );
    let (mut snap, windows) = three_task_fixture();
    snap.runs[0].state = RunState::Complete;
    let app = app_with_runs(windows, snap);
    let shown = listed(&app);
    assert!(
        shown
            .iter()
            .any(|(p, _, text)| *p == 4 && text.starts_with("ready to accept")),
        "{shown:?}"
    );
}
