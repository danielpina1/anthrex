//! Task M5.9: opening, submitting and closing the new-agent form from `App`. The form's
//! own field logic (focus order, validation, key handling) is tested in `dialog.rs`
//! itself; these tests only cover the wiring decisions 26, 30, 31, 33 and 34 describe.

use super::*;
use crate::dialog::TextInput;
use proto::{GitState, Head};

fn git_state() -> GitState {
    GitState {
        head: Head::Branch("main".into()),
        upstream: None,
        ahead: 0,
        behind: 0,
        dirty: 0,
        untracked: 0,
        conflicts: 0,
        operation: None,
        stale: false,
    }
}

/// Panics with a useful message unless the modal is the new-agent form, and hands back
/// a reference to it.
fn expect_new_agent(app: &App) -> &crate::dialog::NewAgentForm {
    match &app.modal {
        Some(Modal::NewAgent(form)) => form,
        other => panic!("expected Modal::NewAgent, got {other:?}"),
    }
}

#[test]
fn new_window_opens_the_form_and_submits_on_enter() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    assert!(press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE).is_empty());
    assert_eq!(
        expect_new_agent(&app).focus,
        crate::dialog::FormField::Runtime
    );

    press(&mut app, KeyCode::Char('3'), KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    match &effects[..] {
        [
            Effect::Send(ClientMsg::CreateWindow {
                spec,
                cols: 80,
                rows: 24,
            }),
        ] => {
            assert_eq!(spec.runtime, Runtime::Shell);
            assert_eq!(spec.cwd, PathBuf::from("/tmp"));
        }
        other => panic!("unexpected effects {other:?}"),
    }
    assert!(expect_new_agent(&app).submitting);

    // The daemon may announce the window list before or after Created; both orders focus it.
    assert!(
        app.on_daemon(DaemonMsg::Created { window_id: 9 })
            .is_empty()
    );
    assert!(app.modal.is_none());
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(9, "shell-9", Status::Starting)],
    });
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 9,
            cols: 80,
            rows: 24
        })]
    );
    assert_eq!(app.focused, Some(9));
}

#[test]
fn a_create_error_is_shown_inline_not_as_a_toast() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    // Defaults (Claude, dir "/tmp") validate cleanly on their own.
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(effects.len(), 1);
    assert!(expect_new_agent(&app).submitting);

    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "create".into(),
            message: "not a git repository: /x".into(),
        })
        .is_empty()
    );

    let form = expect_new_agent(&app);
    assert_eq!(form.error.as_deref(), Some("not a git repository: /x"));
    assert!(!form.submitting);
    assert_eq!(app.toast_text(), None);
}

#[test]
fn an_unrelated_error_while_the_form_is_open_still_toasts() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(!expect_new_agent(&app).submitting);

    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "kill".into(),
            message: "no window with id 7".into(),
        })
        .is_empty()
    );
    assert_eq!(app.toast_text(), Some("no window with id 7"));
    assert!(expect_new_agent(&app).error.is_none());
}

/// The inline-vs-toast split turns on the request kind, not merely on `submitting`: a
/// `kill` error that happens to land while this form's own `create` is still in flight
/// is still none of the form's business.
#[test]
fn an_unrelated_error_while_submitting_still_toasts_and_leaves_submitting_set() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(effects.len(), 1);
    assert!(expect_new_agent(&app).submitting);

    assert!(
        app.on_daemon(DaemonMsg::Error {
            request: "kill".into(),
            message: "no window with id 7".into(),
        })
        .is_empty()
    );
    assert_eq!(app.toast_text(), Some("no window with id 7"));
    let form = expect_new_agent(&app);
    assert!(form.error.is_none());
    assert!(form.submitting, "unrelated to this form's own create");
}

#[test]
fn keys_and_pastes_go_to_the_form_not_the_pty() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);

    assert!(press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE).is_empty());
    assert!(app.on_paste("hello".into()).is_empty());

    press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(app.modal.is_none());
    prefix(&mut app);
    press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::Help)));
    assert!(app.on_paste("hello".into()).is_empty());
}

#[test]
fn the_form_remembers_the_last_accepted_values() {
    let mut app = app_with(vec![]);
    app.home_dir = Some(PathBuf::from("/home/me"));

    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    if let Some(Modal::NewAgent(form)) = &mut app.modal {
        form.runtime = Runtime::Codex;
        form.dir = TextInput::new("~/p");
        form.model = TextInput::new("m1");
    } else {
        panic!("form did not open");
    }
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(effects.len(), 1, "a valid submit sends exactly one effect");

    assert!(
        app.on_daemon(DaemonMsg::Created { window_id: 9 })
            .is_empty()
    );
    assert!(app.modal.is_none());
    assert_eq!(app.form_defaults.runtime, Runtime::Codex);
    assert_eq!(app.form_defaults.dir, "~/p");
    assert_eq!(app.form_defaults.model, "m1");

    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    let form = expect_new_agent(&app);
    assert_eq!(form.runtime, Runtime::Codex);
    assert_eq!(form.dir.text(), "~/p");
    assert_eq!(form.model.text(), "m1");
    assert_eq!(form.name.text(), "");
    assert_eq!(form.branch.text(), "");
    assert_eq!(form.prompt.text(), "");
    assert!(!form.worktree);

    // A submit the daemon rejects must not change the remembered defaults.
    if let Some(Modal::NewAgent(form)) = &mut app.modal {
        form.dir = TextInput::new("~/other");
    }
    press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    app.on_daemon(DaemonMsg::Error {
        request: "create".into(),
        message: "boom".into(),
    });
    assert_eq!(
        app.form_defaults.dir, "~/p",
        "a rejected submit must not overwrite the remembered defaults"
    );
}

#[test]
fn escape_while_submitting_still_focuses_the_new_window() {
    let mut app = app_with(vec![]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    let effects = press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(effects.len(), 1);

    let effects = press(&mut app, KeyCode::Esc, KeyModifiers::NONE);
    assert!(effects.is_empty());
    assert!(app.modal.is_none());

    assert!(
        app.on_daemon(DaemonMsg::Created { window_id: 9 })
            .is_empty()
    );
    let effects = app.on_daemon(DaemonMsg::WindowsChanged {
        windows: vec![win(9, "shell-9", Status::Starting)],
    });
    assert_eq!(
        effects,
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 9,
            cols: 80,
            rows: 24
        })]
    );
    assert_eq!(app.focused, Some(9));
}

#[test]
fn first_open_shows_the_default_dir_with_tilde() {
    let mut app = App::new(vec![], "/home/me/code".into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    app.home_dir = Some(PathBuf::from("/home/me"));

    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert_eq!(expect_new_agent(&app).dir.text(), "~/code");
}

#[test]
fn a_git_message_still_updates_state_while_the_form_is_open() {
    let mut app = app_with(vec![win(1, "a", Status::Idle)]);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('c'), KeyModifiers::NONE);
    assert!(matches!(app.modal, Some(Modal::NewAgent(_))));

    let root = PathBuf::from("/repo");
    assert!(
        app.on_daemon(DaemonMsg::Git {
            root: root.clone(),
            state: Some(git_state()),
        })
        .is_empty()
    );
    assert!(app.git.contains_key(&root));
    assert!(matches!(app.modal, Some(Modal::NewAgent(_))));
}
