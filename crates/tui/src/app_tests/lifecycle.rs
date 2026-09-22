//! Task M6.10: the rename prompt, the restart confirmation and the three ways
//! `C-b Q`'s wait can end. Split out from `app/tests.rs` from the start (rather than
//! grown there and split later) per `AGENTS.md` hard rule 8 — the same shape
//! `app_tests/git.rs` and `app_tests/remove.rs` already give their own tasks' tests.

use super::*;
use std::time::{Duration, Instant};

fn rename_prompt(app: &App) -> &crate::app::prompt::RenamePrompt {
    match &app.modal {
        Some(Modal::Rename(prompt)) => prompt,
        other => panic!("expected Modal::Rename, got {other:?}"),
    }
}

#[test]
fn rename_prompt_edits_and_sends() {
    let mut app = app_with(vec![win(1, "api-worker", Status::Idle)]);

    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char(','), KeyModifiers::NONE).is_empty());
    let prompt = rename_prompt(&app);
    assert_eq!(prompt.window_id, 1);
    assert_eq!(prompt.input.text(), "api-worker");

    // Backspace removes a character.
    assert!(press(&mut app, KeyCode::Backspace, KeyModifiers::NONE).is_empty());
    assert_eq!(rename_prompt(&app).input.text(), "api-worke");

    // Typed characters append.
    for c in "r-2".chars() {
        assert!(press(&mut app, KeyCode::Char(c), KeyModifiers::NONE).is_empty());
    }
    assert_eq!(rename_prompt(&app).input.text(), "api-worker-2");

    // Enter sends Rename and closes the modal.
    assert_eq!(
        press(&mut app, KeyCode::Enter, KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Rename {
            window_id: 1,
            name: "api-worker-2".into(),
        })]
    );
    assert!(app.modal.is_none());

    // Esc closes it and sends nothing.
    prefix(&mut app);
    press(&mut app, KeyCode::Char(','), KeyModifiers::NONE);
    assert!(press(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty());
    assert!(app.modal.is_none());
}

#[test]
fn rename_prompt_checks_before_sending() {
    let mut app = app_with(vec![
        win(1, "api-worker", Status::Idle),
        win(2, "api", Status::Idle),
    ]);

    // Renaming to another window's name.
    prefix(&mut app);
    press(&mut app, KeyCode::Char(','), KeyModifiers::NONE);
    for _ in "-worker".chars() {
        press(&mut app, KeyCode::Backspace, KeyModifiers::NONE);
    }
    assert_eq!(rename_prompt(&app).input.text(), "api");
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert!(matches!(app.modal, Some(Modal::Rename(_))));
    assert_eq!(
        rename_prompt(&app).error.as_deref(),
        Some("a window named 'api' exists")
    );

    // Renaming to spaces.
    press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    press(&mut app, KeyCode::Char(' '), KeyModifiers::NONE);
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert!(matches!(app.modal, Some(Modal::Rename(_))));
    assert_eq!(
        rename_prompt(&app).error.as_deref(),
        Some("name must not be empty")
    );

    // Renaming to 65 characters.
    press(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
    for _ in 0..65 {
        press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    }
    assert!(press(&mut app, KeyCode::Enter, KeyModifiers::NONE).is_empty());
    assert!(matches!(app.modal, Some(Modal::Rename(_))), "modal stays open");
    assert_eq!(
        rename_prompt(&app).error.as_deref(),
        Some("name must be at most 64 characters")
    );
}

#[test]
fn restart_of_an_exited_window_is_immediate() {
    let mut app = app_with(vec![win(1, "api", Status::Exited)]);
    prefix(&mut app);
    assert_eq!(
        press(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT),
        vec![Effect::Send(ClientMsg::Restart { window_id: 1 })]
    );
    assert_eq!(app.toast_text(), Some("restarting api"));
    assert!(app.modal.is_none());
}

/// The dialog-acting-on-the-wrong-window hazard the brief calls out: the confirm modal
/// must carry the id in `PendingAction::Restart`, not recover it from `self.focused` at
/// `y`-time, because focus can move while the dialog is open.
#[test]
fn restart_of_a_live_window_asks_first() {
    let mut app = app_with(vec![win(1, "api", Status::Working), win(2, "web", Status::Idle)]);
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT).is_empty());
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm {
            action: PendingAction::Restart(1),
            ..
        })
    ));

    // Focus moves away while the dialog is open (e.g. a window list update); the
    // pending action must still name window 1, not "whatever is focused now".
    app.focus(2);
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Restart { window_id: 1 })]
    );
}

#[test]
fn restart_confirmation_names_the_window() {
    let mut app = app_with(vec![win(1, "api", Status::Working)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('R'), KeyModifiers::SHIFT);
    match &app.modal {
        Some(Modal::Confirm { message, .. }) => assert_eq!(
            message,
            "Restart 'api'? It is running and will be stopped first."
        ),
        other => panic!("expected Modal::Confirm, got {other:?}"),
    }
}

/// The three failure paths the brief singles out. `on_daemon(Bye)` alone must not quit
/// — the daemon's own confirmation and the link actually closing are two different
/// events, and only the second one is `on_link_lost`.
#[test]
fn stop_daemon_waits_for_confirmation() {
    let mut app = app_with(vec![win(1, "api", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
    assert_eq!(
        press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE),
        vec![Effect::Send(ClientMsg::Shutdown)]
    );

    assert!(
        app.on_daemon(DaemonMsg::Bye {
            reason: "shutting down".into()
        })
        .is_empty(),
        "Bye alone must not quit"
    );

    assert_eq!(app.on_link_lost(), vec![Effect::Quit]);
    // A second C-b Q must be possible: the wait ended, so `stopping` cannot still be set.
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT).is_empty());
    assert!(matches!(
        app.modal,
        Some(Modal::Confirm {
            action: PendingAction::StopDaemon,
            ..
        })
    ));
}

#[test]
fn stop_daemon_send_failure_is_reported() {
    let mut app = app_with(vec![win(1, "api", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);

    assert!(app.on_send_failed(&ClientMsg::Shutdown).is_empty());
    assert_eq!(
        app.toast_text(),
        Some("could not reach the daemon; run anthrex daemon stop")
    );

    // The wait already ended, so a later link loss must not also quit.
    assert!(app.on_link_lost().is_empty());
}

#[test]
fn stop_daemon_times_out() {
    let mut app = app_with(vec![win(1, "api", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('Q'), KeyModifiers::SHIFT);
    press(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);

    app.stopping = Some(Instant::now() - Duration::from_secs(6));
    assert!(app.on_tick().is_empty());
    assert_eq!(
        app.toast_text(),
        Some("the daemon did not confirm the shutdown; run anthrex daemon stop")
    );

    // A second C-b Q is possible once the timeout has cleared `stopping`.
    assert!(app.on_link_lost().is_empty(), "the timeout already ended the wait");
}

/// The other half of finding A's fix (`lib.rs`'s disconnect toast was hard-coded to
/// `C-b`, not `app.settings.prefix_label`).
#[test]
fn link_lost_toast_uses_the_configured_prefix() {
    let settings = UiSettings {
        prefix_label: "C-a".into(),
        ..UiSettings::default()
    };
    let mut app = App::new(vec![win(1, "a", Status::Idle)], "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);
    assert!(app.on_link_lost().is_empty());
    assert_eq!(
        app.toast_text(),
        Some("connection to daemon lost; C-a d to exit")
    );
}
