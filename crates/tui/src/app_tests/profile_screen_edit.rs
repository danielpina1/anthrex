//! Milestone 9.0.6 task 13, decision 35: the Profile screen's editors, its unset page
//! and the daemon's refusal of an edit. Split from `profile_screen.rs` (rule 8).

use super::profile_screen::{
    dir, profile_requests, ready_app, reply, screen, select, tagged, tap, typed,
};
use super::*;
use crate::app::profile_screen::{EditorField, ProfilePage};
use proto::{ProfileReply, ProfileRequest};

fn editor(app: &App) -> &crate::app::profile_screen::Editor {
    match &screen(app).page {
        Some(ProfilePage::Edit(e)) => e,
        other => panic!("no editor: {other:?}"),
    }
}

/// Decision 35: an editor fitted to the key's type, starting from its value.
#[test]
fn e_opens_an_editor_fitted_to_the_key() {
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::Line(area) => assert_eq!(area.text(), "cargo test"),
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Esc);
    select(&mut app, "source");
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::List(area) => assert_eq!(area.text(), "src/**\nlib/**"),
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Esc);
    select(&mut app, "full_shards");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, "x5");
    match &editor(&app).field {
        EditorField::Digits(text) => assert_eq!(text, "25", "digits only"),
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Esc);
    select(&mut app, "module_names");
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::Choice { options, at } => {
            assert_eq!(*options, ["cargo", "dir"]);
            assert_eq!(options[*at], "dir");
        }
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Right);
    match &editor(&app).field {
        EditorField::Choice { options, at } => assert_eq!(options[*at], "cargo"),
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Esc);
    select(&mut app, "env.RUST_LOG");
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::Env { name, value, .. } => {
            assert_eq!((name.text(), value.text()), ("RUST_LOG", "debug"));
        }
        other => panic!("{other:?}"),
    }
    // The add row starts empty, never from the whole environment table.
    tap(&mut app, KeyCode::Esc);
    select(&mut app, crate::profile_view::ENV_ADD);
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::Env { name, value, .. } => {
            assert_eq!((name.text(), value.text()), ("", ""));
        }
        other => panic!("{other:?}"),
    }
    assert!(!editor(&app).from_proposal);
}

/// Progress ruling: an edit from the proposal view says it starts from the stored
/// profile and replaces the proposal; from the stored view it does not.
#[test]
fn an_edit_from_the_proposal_view_says_what_it_replaces() {
    // The dialog wraps at 60 columns: the note's two halves.
    let note = "starts from the stored profile;";
    let rest = "saving replaces the current";
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    assert!(!render(&app).contains(note));
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('p'));
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    assert!(editor(&app).from_proposal);
    match &editor(&app).field {
        EditorField::Line(area) => assert_eq!(area.text(), "cargo test", "the stored value"),
        other => panic!("{other:?}"),
    }
    let text = render(&app);
    assert!(text.contains(note) && text.contains(rest), "{text}");
}

fn render(app: &App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..40)
        .map(|y| {
            (0..120)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decision 35: Enter sends `Edit` with the literal and the store toggle.
#[test]
fn saving_an_edit_sends_profile_edit() {
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    for _ in 0..20 {
        tap(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "cargo nextest run \"x\"");
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "check".into(),
            value: Some(
                crate::profile_view::value_literal("check", "cargo nextest run \"x\"").unwrap()
            ),
            yes: false,
            unconfined_checks: false,
        }]
    );
    assert_eq!(screen(&app).page, None);
    // `s` turns on "store once verification passes": the CLI's `--yes`.
    tap(&mut app, KeyCode::Char('s'));
    assert!(screen(&app).store_on_pass);
    select(&mut app, "source");
    tap(&mut app, KeyCode::Char('e'));
    press(&mut app, KeyCode::Char('j'), KeyModifiers::CONTROL);
    typed(&mut app, "gen/**");
    let effects = tap(&mut app, KeyCode::Enter);
    let literal = crate::profile_view::value_literal("source", "src/**\nlib/**\ngen/**").unwrap();
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "source".into(),
            value: Some(literal),
            yes: true,
            unconfined_checks: false,
        }]
    );
    // An environment value goes as written; Tab moves to the value.
    select(&mut app, "env.RUST_LOG");
    tap(&mut app, KeyCode::Char('e'));
    tap(&mut app, KeyCode::Tab);
    for _ in 0..10 {
        tap(&mut app, KeyCode::Backspace);
    }
    typed(&mut app, "info");
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "env.RUST_LOG".into(),
            value: Some("info".into()),
            yes: true,
            unconfined_checks: false,
        }]
    );
    // A bad value stays in the editor with its reason, and sends nothing.
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    for _ in 0..40 {
        tap(&mut app, KeyCode::Backspace);
    }
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(editor(&app).error.is_some());
}

/// Milestone 9.2 decision 3 (task M9.2.15): the delivery rows edit through the screen,
/// and send the bare value the daemon's `proposal_delivery::edit` reads.
#[test]
fn the_delivery_rows_send_the_bare_value() {
    let (mut app, _) = ready_app();
    select(&mut app, "delivery.mode");
    tap(&mut app, KeyCode::Char('e'));
    match &editor(&app).field {
        EditorField::Choice { options, at } => {
            assert_eq!(*options, ["local", "pr"]);
            assert_eq!(options[*at], "local", "unset reads local");
        }
        other => panic!("{other:?}"),
    }
    tap(&mut app, KeyCode::Right);
    let effects = tap(&mut app, KeyCode::Enter);
    let edit = |key: &str, value: &str| ProfileRequest::Edit {
        dir: dir(),
        key: key.into(),
        value: Some(value.into()),
        yes: false,
        unconfined_checks: false,
    };
    assert_eq!(
        profile_requests(&effects),
        vec![edit("delivery.mode", "pr")]
    );
    select(&mut app, "delivery.remote");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, "upstream");
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(
        profile_requests(&effects),
        vec![edit("delivery.remote", "upstream")]
    );
}

/// Decision 35: `u` asks first; its page's `y` sends the unset.
#[test]
fn u_unsets_after_its_page() {
    let (mut app, _) = ready_app();
    select(&mut app, "setup");
    assert!(tap(&mut app, KeyCode::Char('u')).is_empty());
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::Unset {
            key: "setup".into()
        })
    );
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(screen(&app).page, None);
    tap(&mut app, KeyCode::Char('u'));
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "setup".into(),
            value: None,
            yes: false,
            unconfined_checks: false,
        }]
    );
}

/// Decision 37 is the daemon's; the screen shows its text in its error row.
#[test]
fn an_edit_refused_during_a_live_run_shows_the_daemons_text() {
    let (mut app, _) = ready_app();
    select(&mut app, "setup");
    tap(&mut app, KeyCode::Char('u'));
    let effects = tap(&mut app, KeyCode::Char('y'));
    let id = tagged(&effects)[0].0;
    let text = "run r1 is live in /p/shop; edit the profile once it finishes (runs keep the \
                profile they started with)";
    assert!(
        reply(
            &mut app,
            id,
            ProfileReply::Refused {
                message: text.into()
            }
        )
        .is_empty()
    );
    assert_eq!(screen(&app).error.as_deref(), Some(text));
    assert_eq!(app.toast_text(), None, "not a toast");
    // A Done clears it and refreshes all three.
    tap(&mut app, KeyCode::Char('u'));
    let effects = tap(&mut app, KeyCode::Char('y'));
    let id = tagged(&effects)[0].0;
    let effects = reply(
        &mut app,
        id,
        ProfileReply::Done {
            message: "proposed: setup = (unset)".into(),
        },
    );
    assert_eq!(profile_requests(&effects).len(), 3);
    assert_eq!(screen(&app).error, None);
}
