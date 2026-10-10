//! Milestone 9.10.8 fix round 1: an edit keeps the target it was opened on, Save
//! anyway keeps the daemon's text, `s`, `r` and **Use this** wait for their reply, and an
//! edit's outcome is forgotten once it is known. Split from `profile_screen_edit.rs`
//! (rule 8).

use super::profile_screen::{
    dir, open, open_app, profile_requests, ready_app, reply, screen, screen_mut, select,
    set_status, shown, status, status_with, stored_app, stored_profile, tagged, tap, typed,
};
use super::profile_screen_edit::row_edit_status;
use super::*;
use crate::app::profile_screen::{ProfilePage, Shown, Side};
use proto::{ProfileReply, ProfileRequest, ProposalState, RowEditState};

fn failed() -> RowEditState {
    RowEditState::Failed {
        reason: "exit 101 after 3s".into(),
        tail: "test result: FAILED".into(),
        secs: 3,
    }
}

/// The `on_proposal` of the one `Edit` in `effects`.
fn target(effects: &[Effect]) -> bool {
    match profile_requests(effects).as_slice() {
        [ProfileRequest::Edit { on_proposal, .. }] => *on_proposal,
        other => panic!("not one edit: {other:?}"),
    }
}

/// A ready proposal arrives as a poll would bring it: the card shows.
fn card_appears(app: &mut App) {
    set_status(app, status(Some(ProposalState::Ready)));
    let mut proposal = stored_profile();
    proposal.check = Some("cargo test --workspace".into());
    screen_mut(app).proposal = Side::Ready(Box::new(Shown {
        toml: toml::to_string(&proposal).unwrap(),
        profile: Some(proposal),
        verification: None,
        dropped: vec![],
    }));
    assert!(screen(app).showing_card());
}

/// Decision 18 with 31: Save anyway's outcome is the daemon's text ("stored although
/// its check failed"), never `saved <label>`; `r` forgets the edit too.
#[test]
fn save_anyway_keeps_the_daemons_text() {
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, " --all");
    let id = tagged(&tap(&mut app, KeyCode::Enter))[0].0;
    let done = ProfileReply::Done {
        message: "proposed: check = \"cargo test --all\"; checking it".into(),
    };
    reply(&mut app, id, done);
    let literal = "\"cargo test --all\"";
    set_status(
        &mut app,
        row_edit_status(ProposalState::Ready, literal, failed()),
    );
    let id = tagged(&tap(&mut app, KeyCode::Char('s')))[0].0;
    assert_eq!(screen(&app).saving, None);
    let text = "proposed: check = \"cargo test --all\"; stored although its check failed";
    let done = ProfileReply::Done {
        message: text.into(),
    };
    let ids: Vec<u64> = tagged(&reply(&mut app, id, done))
        .iter()
        .map(|t| t.0)
        .collect();
    reply(&mut app, ids[0], status(None));
    let mut stored = stored_profile();
    stored.check = Some("cargo test --all".into());
    reply(&mut app, ids[1], shown(&stored, vec![]));
    assert_eq!(screen(&app).message.as_deref(), Some(text));
    // `r` forgets a watched edit as well.
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    tap(&mut app, KeyCode::Enter);
    assert!(screen(&app).saving.is_some());
    set_status(
        &mut app,
        row_edit_status(ProposalState::Ready, literal, failed()),
    );
    tap(&mut app, KeyCode::Char('r'));
    assert_eq!(screen(&app).saving, None);
}

/// An editor and an unset page opened on the stored profile still edit it when a
/// review proposal turns ready meanwhile (the card appears under the page).
#[test]
fn an_edit_keeps_its_target_when_the_card_appears() {
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, " --all");
    card_appears(&mut app);
    assert!(
        !target(&tap(&mut app, KeyCode::Enter)),
        "the stored profile"
    );
    assert_eq!(
        screen(&app).saving.as_ref().map(|s| s.on_proposal),
        Some(false)
    );
    let (mut app, _) = stored_app();
    select(&mut app, "setup");
    tap(&mut app, KeyCode::Char('u'));
    card_appears(&mut app);
    assert!(!target(&tap(&mut app, KeyCode::Char('y'))));
}

/// The other way: opened on the card, an edit still goes to the proposal after the
/// card went (used or discarded from the CLI).
#[test]
fn an_edit_keeps_its_target_when_the_card_goes() {
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    set_status(&mut app, status(None));
    assert!(!screen(&app).showing_card());
    assert!(target(&tap(&mut app, KeyCode::Enter)), "the proposal");
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('u'));
    set_status(&mut app, status(None));
    assert!(target(&tap(&mut app, KeyCode::Char('y'))));
}

/// Review M1: `s`, `r` and **Use this** wait for their reply; a second press sends
/// nothing (a second Save anyway would run a whole verification for nothing).
#[test]
fn a_second_press_waits_for_the_first_reply() {
    let (mut app, _) = stored_app();
    set_status(
        &mut app,
        row_edit_status(ProposalState::Ready, "\"make test\"", failed()),
    );
    select(&mut app, "check");
    let id = tagged(&tap(&mut app, KeyCode::Char('s')))[0].0;
    assert!(tap(&mut app, KeyCode::Char('s')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('r')).is_empty());
    let refused = ProfileReply::Refused {
        message: "finish or cancel the run in this repo to change its profile".into(),
    };
    reply(&mut app, id, refused);
    let effects = tap(&mut app, KeyCode::Char('r'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::RevertEdit { dir: dir() }]
    );
    assert!(tap(&mut app, KeyCode::Char('r')).is_empty());
    let (mut app, _) = ready_app();
    assert_eq!(profile_requests(&tap(&mut app, KeyCode::Enter)).len(), 1);
    assert!(tap(&mut app, KeyCode::Enter).is_empty(), "one Confirm");
}

/// Review M2: an edit whose outcome left the value as it was is forgotten, so a later
/// change from elsewhere never reads `saved <label>`.
#[test]
fn an_edit_that_changes_nothing_is_forgotten() {
    let (mut app, _) = stored_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    let id = tagged(&tap(&mut app, KeyCode::Enter))[0].0;
    let done = ProfileReply::Done {
        message: "proposed: check = \"cargo test\"; stored".into(),
    };
    let ids: Vec<u64> = tagged(&reply(&mut app, id, done))
        .iter()
        .map(|t| t.0)
        .collect();
    reply(&mut app, ids[0], status(None));
    assert!(screen(&app).saving.is_some(), "its side not back yet");
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    assert_eq!(screen(&app).saving, None);
    // A later refetch with the key changed elsewhere says nothing.
    let effects = app.profile_fetch_all(std::time::Instant::now());
    let mut other = stored_profile();
    other.check = Some("make check".into());
    reply(&mut app, tagged(&effects)[1].0, shown(&other, vec![]));
    assert_ne!(screen(&app).message.as_deref(), Some("saved check"));
}

/// Review M5: Enter on the card before the proposal's text arrived sends nothing.
#[test]
fn enter_on_the_card_waits_for_the_proposal() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    assert!(screen(&app).showing_card());
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("the proposal is still loading"));
    assert_eq!(screen(&app).page, None);
}

/// Review M6, decision 30: the raw-text page shows `unreadable_text` exactly, never the
/// parse error in its place.
#[test]
fn the_raw_text_page_shows_only_the_files_text() {
    let mut app = open_app();
    let ids = open(&mut app);
    let unreadable = status_with(None, |st| st.unparseable = Some("expected `]`".into()));
    reply(&mut app, ids[0], unreadable);
    tap(&mut app, KeyCode::Enter);
    assert_eq!(
        screen(&app).page,
        Some(ProfilePage::RawText {
            text: String::new(),
            scroll: 0
        })
    );
}
