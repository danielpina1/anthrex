//! Milestone 9.0.7 decision 37, end to end (the user's 9.0.6 try-out): a TUI started on
//! an empty session (no window, no run) in a directory that is not a Git repository.
//! `C-b g` opens the goal form on that start directory, its `Enter` sends the exact
//! `StartGoal` the TUI builds to a real daemon (`RunHarness`, `fake-agent` as both
//! runtimes), and the daemon's own refusal is the form's error row; the client probes
//! nothing. `C-b P`'s requests get the same refusal from the daemon.

mod support;

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{ClientMsg, DaemonMsg, ProfileReply, RunReply, RunRequest};
use tui::app::{App, Effect, Modal};
use tui::settings::UiSettings;

use support::run_harness::{REQUEST_WAIT, RunHarness};

fn key(app: &mut App, code: KeyCode, mods: KeyModifiers) -> Vec<Effect> {
    app.on_key(KeyEvent::new(code, mods))
}

fn prefixed(app: &mut App, c: char) -> Vec<Effect> {
    assert!(key(app, KeyCode::Char('b'), KeyModifiers::CONTROL).is_empty());
    key(app, KeyCode::Char(c), KeyModifiers::NONE)
}

fn tagged(effects: &[Effect]) -> Vec<(u64, RunRequest)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged { id, request }) => Some((*id, request.clone())),
            _ => None,
        })
        .collect()
}

/// A plain directory beside the harness's repository: not a Git repository, nor inside
/// one (`/tmp` is none), canonical as the CLI hands it to the TUI.
fn plain_dir(h: &RunHarness) -> PathBuf {
    let dir = h.dir.path().join("plain");
    std::fs::create_dir_all(&dir).unwrap();
    dir.canonicalize().unwrap()
}

fn empty_session(dir: &Path) -> App {
    let mut app = App::new(vec![], dir.to_path_buf(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    app
}

#[test]
fn an_empty_session_reaches_the_goal_form_and_shows_the_daemons_refusal() {
    let h = RunHarness::new("");
    let dir = plain_dir(&h);
    let mut app = empty_session(&dir);
    assert!(prefixed(&mut app, 'g').is_empty());
    match &app.modal {
        Some(Modal::StartGoal(form)) => assert_eq!(form.project, dir),
        other => panic!("no goal form: {other:?}"),
    }
    for c in "add a readme".chars() {
        key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    let sent = tagged(&key(&mut app, KeyCode::Enter, KeyModifiers::NONE));
    let [(id, request)] = &sent[..] else {
        panic!("one StartGoal: {sent:?}");
    };
    assert!(
        matches!(request, RunRequest::StartGoal { dir: d, .. } if *d == dir),
        "{request:?}"
    );
    let reply = h.tagged(*id, request.clone(), REQUEST_WAIT);
    let want = format!("not a git repository: {}", dir.display());
    assert!(
        matches!(&reply, RunReply::Refused { message, .. } if *message == want),
        "{reply:?}"
    );
    app.on_daemon(DaemonMsg::Run(reply));
    match &app.modal {
        Some(Modal::StartGoal(form)) => {
            assert_eq!(form.error.as_deref(), Some(want.as_str()));
            assert!(!form.submitting);
        }
        other => panic!("the form closed: {other:?}"),
    }
    assert!(
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE).is_empty() && app.modal.is_none(),
        "esc closes the form"
    );

    // `C-b P`: the screen's three requests, each refused by the daemon the same way.
    let sent = tagged(&prefixed(&mut app, 'P'));
    assert_eq!(sent.len(), 3, "{sent:?}");
    assert!(app.screen.is_some());
    for (id, request) in sent {
        let reply = h.tagged(id, request.clone(), REQUEST_WAIT);
        match &reply {
            RunReply::Profile { reply: answer, .. } => assert!(
                matches!(&**answer, ProfileReply::Refused { message } if *message == want),
                "{request:?}: {answer:?}"
            ),
            other => panic!("{request:?}: {other:?}"),
        }
        app.on_daemon(DaemonMsg::Run(reply));
    }
    assert!(app.screen.is_some(), "the screen stays open on its refusal");
    // Final fix wave (task 13b's minor): the refusal is the screen's error row, and
    // both sides read it as absent. The screen's types are the client's own, so this
    // reads them through their `Debug` form.
    let screen = format!("{:?}", app.screen);
    for field in ["error: Some", "stored: Absent", "proposal: Absent"] {
        let want = format!("{field}({want:?})");
        assert!(screen.contains(&want), "{want} in {screen}");
    }
}
