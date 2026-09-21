//! Task M5.10: the remove-confirm dialog and its dirty-tree force-or-keep follow-up
//! (decisions 35 and 36). The dialogs' rendering is task M5.11's; these tests cover
//! only `App`'s wiring — what each key sends, and what a daemon reply does to the
//! modal and to the pending-worktree-removal tracking.

use super::*;

/// A window this daemon made a worktree for, as `WindowInfo.branch` marks it
/// (decision 35: that field, not `branch_text`, decides whether the checkbox shows).
fn wt_win(id: u32, name: &str, branch: &str) -> WindowInfo {
    let mut window = win(id, name, Status::Idle);
    window.branch = Some(branch.to_string());
    window
}

fn open_remove_confirm(app: &mut App) {
    prefix(app);
    assert!(press(app, KeyCode::Char('X'), KeyModifiers::SHIFT).is_empty());
}

/// Opens the remove-confirm dialog on the focused window, ticks the worktree box,
/// confirms, and feeds back a `remove-dirty` refusal — leaving `Modal::ForceRemove`
/// open for the caller.
fn open_force_remove(app: &mut App, message: &str) {
    open_remove_confirm(app);
    assert!(press(app, KeyCode::Char('w'), KeyModifiers::NONE).is_empty());
    assert!(press(app, KeyCode::Enter, KeyModifiers::NONE).len() == 1);
    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "remove-dirty".into(),
            message: message.into(),
        })
        .is_empty()
    );
}

#[test]
fn remove_of_a_plain_window_sends_a_plain_remove() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    open_remove_confirm(&mut app);
    match &app.modal {
        Some(Modal::Remove(confirm)) => {
            assert_eq!(confirm.window_id, 1);
            assert_eq!(confirm.branch, None);
            assert!(!confirm.remove_worktree);
        }
        other => panic!("expected Modal::Remove, got {other:?}"),
    }

    // Space has nothing to toggle on a window with no worktree.
    assert!(press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE).is_empty());
    match &app.modal {
        Some(Modal::Remove(confirm)) => assert!(!confirm.remove_worktree),
        other => panic!("expected Modal::Remove, got {other:?}"),
    }

    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 1,
            remove_worktree: false,
            force: false,
        })]
    );
    assert!(app.modal.is_none());
}

#[test]
fn remove_confirm_cancel_sends_nothing() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    open_remove_confirm(&mut app);
    assert!(press(&mut app, KeyCode::Char('n'), KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());

    open_remove_confirm(&mut app);
    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());
}

#[test]
fn remove_checkbox_is_offered_and_starts_unticked() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);

    open_remove_confirm(&mut app);
    match &app.modal {
        Some(Modal::Remove(confirm)) => {
            assert_eq!(confirm.branch.as_deref(), Some("feat/api"));
            assert!(!confirm.remove_worktree, "starts unticked");
        }
        other => panic!("expected Modal::Remove, got {other:?}"),
    }
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 1,
            remove_worktree: false,
            force: false,
        })],
        "Enter with the box unticked removes only the window"
    );

    open_remove_confirm(&mut app);
    assert!(press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE).is_empty());
    match &app.modal {
        Some(Modal::Remove(confirm)) => assert!(confirm.remove_worktree, "'w' ticks it"),
        other => panic!("expected Modal::Remove, got {other:?}"),
    }
    // Space toggles the same box: back off, then on again.
    assert!(press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE).is_empty());
    match &app.modal {
        Some(Modal::Remove(confirm)) => assert!(!confirm.remove_worktree),
        other => panic!("expected Modal::Remove, got {other:?}"),
    }
    assert!(press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE).is_empty());

    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 1,
            remove_worktree: true,
            force: false,
        })]
    );
}

#[test]
fn a_dirty_refusal_opens_the_force_prompt() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    open_force_remove(&mut app, "worktree /w has uncommitted or untracked changes");

    match &app.modal {
        Some(Modal::ForceRemove {
            window_id,
            name,
            message,
        }) => {
            assert_eq!(*window_id, 1);
            assert_eq!(name, "api");
            assert_eq!(message, "worktree /w has uncommitted or untracked changes");
        }
        other => panic!("expected Modal::ForceRemove, got {other:?}"),
    }
    assert_eq!(
        app.toast_text(),
        None,
        "the refusal opens a dialog, not a toast"
    );

    // Enter is deliberately not a synonym for either destructive choice.
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert!(
        matches!(app.modal, Some(Modal::ForceRemove { .. })),
        "Enter must leave the force prompt open"
    );

    assert_eq!(
        press(&mut app, KeyCode::Char('f'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 1,
            remove_worktree: true,
            force: true,
        })]
    );
    assert!(app.modal.is_none());
}

#[test]
fn keep_removes_only_the_window() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    open_force_remove(&mut app, "worktree /w has uncommitted or untracked changes");

    assert_eq!(
        press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Remove {
            window_id: 1,
            remove_worktree: false,
            force: false,
        })]
    );
    assert!(app.modal.is_none());
}

#[test]
fn cancel_leaves_everything() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    open_force_remove(&mut app, "worktree /w has uncommitted or untracked changes");

    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());
}

#[test]
fn remove_dirty_without_a_pending_removal_is_a_toast() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "remove-dirty".into(),
            message: "worktree /w has uncommitted or untracked changes".into(),
        })
        .is_empty()
    );
    assert_eq!(
        app.toast_text(),
        Some("worktree /w has uncommitted or untracked changes")
    );
    assert!(app.modal.is_none());
}

#[test]
fn ack_clears_the_pending_removal() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    open_remove_confirm(&mut app);
    press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(
        app.on_daemon(DaemonMsg::Ack {
            request: "remove".into(),
        })
        .is_empty()
    );

    // With the pending removal cleared, an unrelated later `remove-dirty` (a stale
    // reply, or one for some other window entirely) is only a toast.
    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "remove-dirty".into(),
            message: "worktree /w has uncommitted or untracked changes".into(),
        })
        .is_empty()
    );
    assert_eq!(
        app.toast_text(),
        Some("worktree /w has uncommitted or untracked changes")
    );
    assert!(
        app.modal.is_none(),
        "the Ack must have cleared the pending removal"
    );
}

#[test]
fn a_remove_error_also_clears_the_pending_removal() {
    let mut app = app_with(vec![wt_win(1, "api", "feat/api")]);
    open_remove_confirm(&mut app);
    press(&mut app, KeyCode::Char('w'), KeyModifiers::NONE);
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "remove".into(),
            message: "no window with id 1".into(),
        })
        .is_empty()
    );
    assert_eq!(app.toast_text(), Some("no window with id 1"));

    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "remove-dirty".into(),
            message: "worktree /w has uncommitted or untracked changes".into(),
        })
        .is_empty()
    );
    assert_eq!(
        app.toast_text(),
        Some("worktree /w has uncommitted or untracked changes")
    );
    assert!(
        app.modal.is_none(),
        "the remove error must have cleared the pending removal too"
    );
}
