//! Milestone 9.10's final review, client slice: a held ✗ inside Advanced opens it (M7),
//! Enter on the card's Advanced line folds or opens it (M3), a refused request asks
//! for the views again (M5), and the reply gate of `s`, `r` and **Use this** says why
//! a press waits and opens on an expiry or a lost link (task 8 re-review).

use super::profile_screen::{
    DIR, dir, open, open_app, profile_requests, ready_app, reply, screen, select, shown, status,
    status_with, stored_app, stored_profile, tagged, tap,
};
use super::*;
use crate::app::profile_screen::ADVANCED_ROW;
use crate::app::replies::LONG_REPLY_TIMEOUT;
use proto::{
    ProfileReply, ProfileRequest, ProfileSource, ProposalOrigin, ProposalState, RowEdit,
    RowEditState,
};
use std::time::{Duration, Instant};

fn failed() -> RowEditState {
    RowEditState::Failed {
        reason: "exit 101 after 3s".into(),
        tail: String::new(),
        secs: 3,
    }
}

/// A status whose `Edit`-origin proposal holds a ✗ row edit of `key`.
fn failed_edit_of(key: &str) -> ProfileReply {
    status_with(Some(ProposalState::Ready), |st| {
        if let Some(p) = st.proposal.as_mut() {
            p.origin = ProposalOrigin::Edit {
                keys: vec![key.into()],
            };
            p.edit = Some(RowEdit {
                key: key.into(),
                value: Some("\"cargo build --all\"".into()),
                state: failed(),
            });
        }
    })
}

/// The status a fresh fetch asks for, answered with `with`.
fn poll(app: &mut App, with: ProfileReply) {
    let effects = app.profile_fetch_all(Instant::now());
    reply(app, tagged(&effects)[0].0, with);
}

/// Final review M7: a ✗ row edit of a key inside Advanced opens it when it arrives, so
/// its row (and its `s` / `r`) shows; folded again, the same ✗ leaves it folded.
#[test]
fn a_failed_edit_inside_advanced_opens_it() {
    let (mut app, _) = stored_app();
    assert!(!screen(&app).advanced);
    poll(&mut app, failed_edit_of("build_check"));
    assert!(screen(&app).advanced, "opened for the ✗");
    assert!(screen(&app).rows().iter().any(|r| r.key == "build_check"));
    tap(&mut app, KeyCode::Char('a'));
    poll(&mut app, failed_edit_of("build_check"));
    assert!(!screen(&app).advanced, "the user's fold is kept");
    // A ✗ outside Advanced leaves it as it is.
    let (mut app, _) = stored_app();
    poll(&mut app, failed_edit_of("check"));
    assert!(!screen(&app).advanced);
}

/// The screen on a repository with no stored profile and a ready review proposal: the
/// fresh card, which lists the Advanced line.
fn fresh_card() -> App {
    let mut app = open_app();
    let ids = open(&mut app);
    let none = status_with(Some(ProposalState::Ready), |st| {
        st.source = ProfileSource::None;
        st.confirmed_at = None;
    });
    reply(&mut app, ids[0], none);
    let refused = ProfileReply::Refused {
        message: "no stored profile for /p/shop".into(),
    };
    reply(&mut app, ids[1], refused);
    reply(&mut app, ids[2], shown(&stored_profile(), vec![]));
    app
}

/// Final review M3: Enter on the card's Advanced line opens or folds it, as on the
/// profile; it never stores the proposal.
#[test]
fn enter_on_the_cards_advanced_line_toggles_it() {
    let mut app = fresh_card();
    assert!(screen(&app).showing_card());
    select(&mut app, ADVANCED_ROW);
    assert!(tap(&mut app, KeyCode::Enter).is_empty(), "no Confirm");
    assert!(screen(&app).advanced);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(!screen(&app).advanced);
}

/// Final review M5: a refused **Use this** (the proposal changed meanwhile) asks for the
/// status and both sides again, so the next Enter sends what the card now shows.
#[test]
fn a_refused_use_this_fetches_the_views_again() {
    let (mut app, _) = ready_app();
    let id = tagged(&tap(&mut app, KeyCode::Enter))[0].0;
    let refused = ProfileReply::Refused {
        message: "the proposal changed since it was shown".into(),
    };
    let asked = profile_requests(&reply(&mut app, id, refused));
    assert_eq!(
        asked,
        vec![
            ProfileRequest::Status { dir: dir() },
            ProfileRequest::Show {
                dir: dir(),
                proposed: false
            },
            ProfileRequest::Show {
                dir: dir(),
                proposed: true
            },
        ]
    );
    assert_eq!(
        screen(&app).error.as_deref(),
        Some("the proposal changed since it was shown")
    );
}

/// The words a gated press shows.
const WAITING: &str = "waiting for the daemon's reply to the last request";

/// Task 8 re-review: a press the reply gate holds says so, instead of doing nothing.
#[test]
fn a_gated_press_says_it_waits() {
    let (mut app, _) = stored_app();
    poll(&mut app, failed_edit_of("check"));
    select(&mut app, "check");
    assert_eq!(tagged(&tap(&mut app, KeyCode::Char('s'))).len(), 1);
    app.toast = None;
    assert!(tap(&mut app, KeyCode::Char('r')).is_empty());
    assert_eq!(app.toast_text(), Some(WAITING));
    let (mut app, _) = ready_app();
    assert_eq!(tagged(&tap(&mut app, KeyCode::Enter)).len(), 1);
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some(WAITING));
}

/// Task 8 re-review: the gate opens once its request expires; the next press is sent.
#[test]
fn the_gate_opens_when_its_request_expires() {
    let (mut app, _) = ready_app();
    let id = tagged(&tap(&mut app, KeyCode::Enter))[0].0;
    let past = Instant::now()
        .checked_sub(LONG_REPLY_TIMEOUT + Duration::from_secs(1))
        .unwrap();
    app.set_reply_sent_at(id, past);
    app.on_tick();
    assert!(!app.replies.contains(id));
    let again = profile_requests(&tap(&mut app, KeyCode::Enter));
    assert!(
        matches!(again.as_slice(), [ProfileRequest::Confirm { .. }]),
        "{again:?}"
    );
}

/// Task 8 re-review: the gate opens when the link is lost; after the reconnect the next
/// press is sent.
#[test]
fn the_gate_opens_when_the_link_is_lost() {
    let (mut app, _) = ready_app();
    assert_eq!(tagged(&tap(&mut app, KeyCode::Enter)).len(), 1);
    app.on_link_lost("gone");
    app.on_reconnected(vec![project_win(1, DIR)]);
    if !screen(&app).showing_card() {
        panic!("the card went with the reconnect");
    }
    let again = profile_requests(&tap(&mut app, KeyCode::Enter));
    assert!(
        matches!(again.as_slice(), [ProfileRequest::Confirm { .. }]),
        "{again:?}"
    );
}

/// A status with a ready review proposal and one goal waiting for it.
fn ready_with_a_queued_goal() -> ProfileReply {
    status_with(Some(ProposalState::Ready), |st| {
        st.queued = vec![proto::QueuedGoalInfo {
            id: "q1".into(),
            project: dir(),
            goal: "add a flag".into(),
            queued_at: 0,
            yes: false,
            trust_project: false,
            unconfined_checks: false,
            setup: proto::SetupState::NeedsReview,
        }];
    })
}

/// Final review D-I2: **Use this** with goals waiting starts them after its reply, and
/// a goal that cannot start is recorded a moment later; the screen asks for the status
/// once a second for [`DRAIN_WATCH`] after it, so the dropped goal shows. Without
/// waiting goals it does not poll.
#[test]
fn use_this_with_queued_goals_watches_them_start() {
    use super::profile_screen::statuses;
    use crate::app::profile_screen::DRAIN_WATCH;
    for queued in [true, false] {
        let (mut app, _) = ready_app();
        if queued {
            poll(&mut app, ready_with_a_queued_goal());
        }
        let id = tagged(&tap(&mut app, KeyCode::Enter))[0].0;
        let done = ProfileReply::Done {
            message: "stored the profile for /p/shop".into(),
        };
        let asked = tagged(&reply(&mut app, id, done));
        reply(&mut app, asked[0].0, status(None));
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(2);
        let polled = app.screens_tick(later);
        assert_eq!(statuses(&polled), usize::from(queued), "{queued}");
        // Answered, the poll goes on; nothing once the watch is over.
        for (id, _) in tagged(&polled) {
            reply(&mut app, id, status(None));
        }
        let past = t0 + DRAIN_WATCH + Duration::from_secs(2);
        assert_eq!(statuses(&app.screens_tick(past)), 0, "{queued}");
    }
}
