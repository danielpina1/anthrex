//! Milestone 9.10 decisions 27-30: the review card. A ready review proposal shows it in
//! place of the profile: Enter is **Use this**, `e` edits the proposal, `x` discards it,
//! Esc is **Later**.

use super::profile_screen::{
    dir, keys_of, open, open_app, profile_requests, ready_app, reply, screen, select, shown,
    status, status_with, stored_profile, tap, typed,
};
use super::*;
use crate::app::AlertKey;
use crate::app::profile_screen::{ADVANCED_ROW, EditorField, ProfilePage};
use proto::{
    ProfileReply, ProfileRequest, ProfileSource, ProposalOrigin, ProposalState, QueuedGoalInfo,
    RepoProfile, SetupState,
};
use std::path::PathBuf;

/// The screen on a repository with no stored profile and a ready proposal `profile`:
/// the fresh card.
fn fresh_app(profile: &RepoProfile) -> App {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(
        &mut app,
        ids[0],
        status_with(Some(ProposalState::Ready), |st| {
            st.source = ProfileSource::None;
            st.confirmed_at = None;
        }),
    );
    let none = ProfileReply::Refused {
        message: "no stored profile for /p/shop".into(),
    };
    reply(&mut app, ids[1], none);
    reply(&mut app, ids[2], shown(profile, vec![]));
    app
}

/// Decision 27: the card shows while the status has a review proposal `Ready`; not for
/// one still running, nor for an `Edit`-origin proposal.
#[test]
fn a_ready_review_proposal_shows_the_card() {
    let (app, _) = ready_app();
    assert!(screen(&app).showing_card());
    assert!(screen(&app).review_proposal());
    let mut app = fresh_app(&stored_profile());
    assert!(screen(&app).showing_card());
    for (state, origin) in [
        (ProposalState::Scouting, ProposalOrigin::Goal),
        (ProposalState::Verifying, ProposalOrigin::Detect),
        (
            ProposalState::Ready,
            ProposalOrigin::Edit {
                keys: vec!["check".into()],
            },
        ),
    ] {
        let reply = status_with(Some(state.clone()), |st| {
            if let Some(p) = st.proposal.as_mut() {
                p.origin = origin.clone();
            }
        });
        super::profile_screen::set_status(&mut app, reply);
        assert!(!screen(&app).showing_card(), "{state:?} {origin:?}");
    }
}

/// Decision 29: Enter on the card is **Use this**: `Confirm` with the proposal's
/// `Shown.toml` byte for byte, no page between.
#[test]
fn enter_on_the_card_uses_this() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    let toml = "check = \"cargo test --workspace\"  \n\n# protected: built-in .git/** + no \
                extras\n# verification 2026-10-01 10:00 (confined)\n";
    let proposal = ProfileReply::Shown {
        source: ProfileSource::None,
        toml: toml.into(),
        meta: None,
        verification: None,
        dropped: vec![],
    };
    reply(&mut app, ids[2], proposal);
    assert!(screen(&app).showing_card());
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Confirm {
            dir: dir(),
            shown: Some(toml.into())
        }]
    );
    assert_eq!(screen(&app).page, None);
}

/// Decision 29: Esc on the card is **Later**: the screen closes and nothing is sent.
#[test]
fn esc_on_the_card_is_later() {
    let (mut app, _) = ready_app();
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert!(app.screen.is_none());
}

/// Decisions 16 and 29: `e` on the card edits the proposal, starting from its value;
/// Enter sends `Edit { on_proposal: true, yes: true }`.
#[test]
fn e_on_the_card_edits_the_proposal() {
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    match &screen(&app).page {
        Some(ProfilePage::Edit(e)) => match &e.field {
            EditorField::Line(area) => {
                assert_eq!(
                    area.text(),
                    "cargo test --workspace",
                    "the proposal's value"
                )
            }
            other => panic!("{other:?}"),
        },
        other => panic!("no editor: {other:?}"),
    }
    typed(&mut app, " --all");
    let effects = tap(&mut app, KeyCode::Enter);
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Edit {
            dir: dir(),
            key: "check".into(),
            value: Some(
                crate::profile_view::value_literal("check", "cargo test --workspace --all")
                    .unwrap()
            ),
            yes: true,
            unconfined_checks: false,
            anyway: false,
            on_proposal: true,
        }]
    );
}

fn queued(goal: &str) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: format!("q-1-{goal}"),
        project: dir(),
        goal: goal.into(),
        queued_at: 0,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup: SetupState::NeedsReview,
    }
}

/// Decisions 8 and 30: `x` asks first, naming the queued goals it drops; only `y` sends
/// `Reject`.
#[test]
fn x_on_the_card_asks_then_discards() {
    let mut app = open_app();
    let ids = open(&mut app);
    let ready = status_with(Some(ProposalState::Ready), |st| {
        st.queued = vec![queued("add a flag"), queued("fix the bug")];
    });
    reply(&mut app, ids[0], ready);
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    reply(&mut app, ids[2], shown(&stored_profile(), vec![]));
    assert!(tap(&mut app, KeyCode::Char('x')).is_empty());
    assert_eq!(screen(&app).page, Some(ProfilePage::Discard));
    assert_eq!(
        screen(&app).discard_text(),
        "the proposal for /p/shop is deleted; a running scout or verification stops; 2 \
         queued goals are dropped"
    );
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to discard"));
    let effects = tap(&mut app, KeyCode::Char('y'));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Reject { dir: dir() }]
    );
    // One goal, and none.
    super::profile_screen::set_status(
        &mut app,
        status_with(Some(ProposalState::Ready), |st| {
            st.queued = vec![queued("add a flag")]
        }),
    );
    assert!(
        screen(&app)
            .discard_text()
            .ends_with("; 1 queued goal is dropped")
    );
    super::profile_screen::set_status(&mut app, status(Some(ProposalState::Ready)));
    assert!(screen(&app).discard_text().ends_with("verification stops"));
}

/// Decision 28: with a stored profile the card lists only the rows that change, an
/// Advanced one included, and nothing is folded.
#[test]
fn the_changes_card_lists_only_changed_rows() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    let mut proposal = stored_profile();
    proposal.check = Some("cargo nextest run".into());
    proposal.hub = vec!["src/core/**".into()];
    reply(&mut app, ids[2], shown(&proposal, vec![]));
    assert_eq!(keys_of(&app), ["check", "hub"]);
    let rows = screen(&app).rows();
    assert_eq!(rows[0].old.as_deref(), Some("cargo test"));
    assert_eq!(rows[0].value.as_deref(), Some("cargo nextest run"));
    assert!(rows[1].advanced, "hub sits inside Advanced");
}

/// Decision 28: the fresh card shows the main sections' rows that have a value, with
/// Advanced folded; `a` opens it.
#[test]
fn a_opens_advanced_on_the_fresh_card() {
    let mut app = fresh_app(&stored_profile());
    let keys = keys_of(&app);
    assert_eq!(keys, ["setup", "check", "source", ADVANCED_ROW]);
    assert!(!screen(&app).advanced);
    assert!(tap(&mut app, KeyCode::Char('a')).is_empty());
    assert!(screen(&app).advanced);
    let keys = keys_of(&app);
    for key in ["full_shards", "module_names", "env.RUST_LOG"] {
        assert!(keys.contains(&key.to_string()), "{key}: {keys:?}");
    }
    let rows = screen(&app).rows();
    assert!(
        rows.iter()
            .all(|r| r.key == ADVANCED_ROW || r.value.is_some()),
        "only rows with a value: {keys:?}"
    );
}

/// Preflight F26: Enter on a proposal alert opens this screen, which shows the card once
/// the proposal is ready (decision 33).
#[test]
fn enter_on_a_proposal_alert_opens_the_profile_screen() {
    let mut app = super::alerts::every_app();
    let key = AlertKey::Proposal("/r/shop".into());
    prefix(&mut app);
    tap(&mut app, KeyCode::Char('a'));
    let mut effects = vec![];
    for _ in 0..20 {
        let on = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
        if on.as_ref() == Some(&key) {
            effects = tap(&mut app, KeyCode::Enter);
            break;
        }
        tap(&mut app, KeyCode::Char('j'));
    }
    let shop = PathBuf::from("/r/shop");
    assert_eq!(
        profile_requests(&effects),
        vec![
            ProfileRequest::Status { dir: shop.clone() },
            ProfileRequest::Show {
                dir: shop.clone(),
                proposed: false
            },
            ProfileRequest::Show {
                dir: shop.clone(),
                proposed: true
            },
        ]
    );
    let s = screen(&app);
    assert_eq!(s.dir, shop);
    assert_eq!(app.alerts_focus, None);
    assert_eq!(app.toast_text(), None);
}
