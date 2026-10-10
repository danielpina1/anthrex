//! Milestone 9.0.6 task 13, decision 35, and milestone 9.10 decisions 16-20 and 31: the
//! Profile screen's editors, its unset page, the row edit's check (`checking…`, ✗,
//! save anyway, revert, `saved <label>`) and the daemon's refusal of an edit. Split
//! from `profile_screen.rs` (rule 8).

use super::profile_screen::{
    dir, profile_requests, reply, screen, select, set_status, shown, status, status_with,
    stored_app, stored_profile, tagged, tap, typed,
};
use super::*;
use crate::app::profile_screen::{EditorField, ProfilePage};
use proto::{ProfileReply, ProfileRequest, ProposalOrigin, ProposalState, RowEdit, RowEditState};
use std::time::{Duration, Instant};

fn editor(app: &App) -> &crate::app::profile_screen::Editor {
    match &screen(app).page {
        Some(ProfilePage::Edit(e)) => e,
        other => panic!("no editor: {other:?}"),
    }
}

/// Decision 35: an editor fitted to the key's type, starting from its value.
#[test]
fn e_opens_an_editor_fitted_to_the_key() {
    let (mut app, _) = stored_app();
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
}

/// Decisions 16 and 35: Enter sends `Edit` with the literal, always `yes: true`.
#[test]
fn saving_an_edit_sends_profile_edit() {
    let (mut app, _) = stored_app();
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
            yes: true,
            unconfined_checks: false,
            anyway: false,
            on_proposal: false,
        }]
    );
    assert_eq!(screen(&app).page, None);
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
            anyway: false,
            on_proposal: false,
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
            anyway: false,
            on_proposal: false,
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
    let (mut app, _) = stored_app();
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
        yes: true,
        unconfined_checks: false,
        anyway: false,
        on_proposal: false,
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

/// Review finding m4: the delivery rows drawn at 80×24, Unicode and ASCII: the head,
/// both keys (no `[delivery]` reads as a local merge), the bar on `delivery.mode`, and its
/// editor's choice after a step from `local`: `< pr >` (`‹ pr ›` in Unicode).
#[test]
fn the_delivery_rows_draw_at_80x24() {
    for ascii in [false, true] {
        let (mut app, _) = stored_app();
        app.settings.badges.ascii = ascii;
        app.set_terminal_size(80, 24);
        tap(&mut app, KeyCode::Char('a'));
        select(&mut app, "delivery.mode");
        let rows = frame_rows(&app, 80, 24);
        let (bar, choice) = if ascii {
            (">", "< pr >")
        } else {
            ("▌", "‹ pr ›")
        };
        let mode = rows
            .iter()
            .find(|r| r.contains("delivery.mode"))
            .expect("mode row");
        assert!(mode.contains(&format!("{bar}Delivery")), "{mode}");
        // No `[delivery]` reads as a local merge (decision 26).
        assert!(mode.contains("merge here, no pull request"), "{mode}");
        assert!(rows.iter().any(|r| r.contains("delivery")), "{rows:#?}");
        tap(&mut app, KeyCode::Char('e'));
        tap(&mut app, KeyCode::Right);
        let rows = frame_rows(&app, 80, 24);
        assert!(rows.iter().any(|r| r.contains(choice)), "{rows:#?}");
        if ascii {
            assert!(rows.iter().all(|r| r.is_ascii()), "{rows:#?}");
        }
        // The remote sits inside Advanced (decision 24).
        tap(&mut app, KeyCode::Esc);
        select(&mut app, "delivery.remote");
        let rows = frame_rows(&app, 80, 24);
        assert!(
            rows.iter().any(|r| r.contains("delivery.remote")),
            "{rows:#?}"
        );
    }
}

fn frame_rows(app: &App, w: u16, h: u16) -> Vec<String> {
    crate::ui::audit::rows(&crate::ui::audit::draw(app, w, h))
}

/// Decision 35: `u` asks first; its page's `y` sends the unset.
#[test]
fn u_unsets_after_its_page() {
    let (mut app, _) = stored_app();
    select(&mut app, "setup");
    assert!(tap(&mut app, KeyCode::Char('u')).is_empty());
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::Unset {
            key: "setup".into(),
            on_proposal: false,
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
            yes: true,
            unconfined_checks: false,
            anyway: false,
            on_proposal: false,
        }]
    );
}

/// Decision 20 is the daemon's; the screen shows its text in its error row.
#[test]
fn an_edit_refused_during_a_live_run_shows_the_daemons_text() {
    let (mut app, _) = stored_app();
    select(&mut app, "setup");
    tap(&mut app, KeyCode::Char('u'));
    let effects = tap(&mut app, KeyCode::Char('y'));
    let id = tagged(&effects)[0].0;
    let text = "finish or cancel the run in this repo to change its profile";
    // Final review M5: the views are asked again after a refusal.
    let refused = ProfileReply::Refused {
        message: text.into(),
    };
    assert_eq!(profile_requests(&reply(&mut app, id, refused)).len(), 3);
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

/// A status whose `Edit`-origin proposal (a row edit of the stored profile) carries
/// `edit` of `check` = `value` in `state`.
pub(super) fn row_edit_status(
    proposal: ProposalState,
    value: &str,
    state: RowEditState,
) -> ProfileReply {
    status_with(Some(proposal), |st| {
        if let Some(p) = st.proposal.as_mut() {
            p.origin = ProposalOrigin::Edit {
                keys: vec!["check".into()],
            };
            p.edit = Some(RowEdit {
                key: "check".into(),
                value: Some(value.into()),
                state,
            });
        }
    })
}

/// Decisions 16 and 31: a command row's edit sends `yes: true`; while its check runs the
/// row reads `checking…` (its state is the status's row edit) and the screen polls.
#[test]
fn a_command_edit_sends_yes_and_shows_checking() {
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, " --all");
    let effects = tap(&mut app, KeyCode::Enter);
    let literal = crate::profile_view::value_literal("check", "cargo test --all").unwrap();
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "check".into(),
            value: Some(literal.clone()),
            yes: true,
            unconfined_checks: false,
            anyway: false,
            on_proposal: false,
        }]
    );
    let id = tagged(&effects)[0].0;
    let done = ProfileReply::Done {
        message: format!("proposed: check = {literal}; checking it"),
    };
    let ids: Vec<u64> = tagged(&reply(&mut app, id, done))
        .iter()
        .map(|t| t.0)
        .collect();
    let checking = row_edit_status(ProposalState::Verifying, &literal, RowEditState::Verifying);
    reply(&mut app, ids[0], checking);
    assert_eq!(
        screen(&app).edit_of("check"),
        Some(&RowEditState::Verifying)
    );
    assert_eq!(screen(&app).edit_of("setup"), None);
    assert!(screen(&app).row_edit().is_some());
    // The poll runs while the edit is checked: on the stored profile, and on a review
    // proposal that stays `Ready` meanwhile.
    let later = Instant::now() + Duration::from_secs(2);
    assert_eq!(super::profile_screen::statuses(&app.screens_tick(later)), 1);
    let (mut app, _) = stored_app();
    set_status(
        &mut app,
        status_with(Some(ProposalState::Ready), |st| {
            if let Some(p) = st.proposal.as_mut() {
                p.edit = Some(RowEdit {
                    key: "check".into(),
                    value: Some(literal.clone()),
                    state: RowEditState::Verifying,
                });
            }
        }),
    );
    assert_eq!(super::profile_screen::statuses(&app.screens_tick(later)), 1);
}

/// Decisions 18, 19 and 29: on a ✗ row, `s` saves anyway (the failed edit's key and
/// value, `anyway` and `yes`) and `r` reverts; on any other row neither does anything.
#[test]
fn a_failed_row_offers_save_anyway_and_revert() {
    let (mut app, _) = stored_app();
    let failed = RowEditState::Failed {
        reason: "exit 101 after 3s".into(),
        tail: "test result: FAILED".into(),
        secs: 3,
    };
    set_status(
        &mut app,
        row_edit_status(ProposalState::Ready, "\"make test\"", failed.clone()),
    );
    assert_eq!(screen(&app).edit_of("check"), Some(&failed));
    select(&mut app, "setup");
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('r')).is_empty());
    assert_eq!(screen(&app).page, None);
    select(&mut app, "check");
    let effects = tap(&mut app, KeyCode::Char('s'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "check".into(),
            value: Some("\"make test\"".into()),
            yes: true,
            unconfined_checks: false,
            anyway: true,
            on_proposal: false,
        }]
    );
    // `r` once `s` has its reply (a second key waits for the first's).
    let id = tagged(&effects)[0].0;
    let refused = ProfileReply::Refused {
        message: "finish or cancel the run in this repo to change its profile".into(),
    };
    reply(&mut app, id, refused);
    let effects = tap(&mut app, KeyCode::Char('r'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::RevertEdit { dir: dir() }]
    );
    // Its Done refreshes the three views.
    let id = tagged(&effects)[0].0;
    let done = ProfileReply::Done {
        message: "reverted check; the profile is unchanged".into(),
    };
    assert_eq!(profile_requests(&reply(&mut app, id, done)).len(), 3);
    // `o` shows the failed check's output.
    tap(&mut app, KeyCode::Char('o'));
    assert_eq!(screen(&app).expanded.as_deref(), Some("check"));
    // While it is still checked, neither key acts.
    set_status(
        &mut app,
        row_edit_status(
            ProposalState::Verifying,
            "\"make test\"",
            RowEditState::Verifying,
        ),
    );
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('r')).is_empty());
}

/// Decision 31: once the check passes (no row edit left, the stored value changed), the
/// message row says `saved <label>`.
#[test]
fn a_passed_edit_says_saved() {
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, " --all");
    let effects = tap(&mut app, KeyCode::Enter);
    let id = tagged(&effects)[0].0;
    let done = ProfileReply::Done {
        message: "proposed: check = \"cargo test --all\"; checking it".into(),
    };
    let ids: Vec<u64> = tagged(&reply(&mut app, id, done))
        .iter()
        .map(|t| t.0)
        .collect();
    reply(&mut app, ids[0], status(None));
    assert_ne!(
        screen(&app).message.as_deref(),
        Some("saved check"),
        "not yet"
    );
    let mut saved = stored_profile();
    saved.check = Some("cargo test --all".into());
    reply(&mut app, ids[1], shown(&saved, vec![]));
    assert_eq!(screen(&app).message.as_deref(), Some("saved check"));
    assert_eq!(screen(&app).error, None);
}
