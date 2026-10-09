//! Milestone 9.0.6 final review, minors 1, 2 and 4: the Profile screen's views never
//! stay `loading…` (an expired or unsent `Status` or `Show` is the screen's to show and
//! to ask again, once a second, one at a time), a detection that ends fetches the
//! proposal again, and a page keeps its edit while the link is down.

use super::profile_screen::{
    dir, open, open_app, profile_requests, ready_app, reply, screen, select, shown, status,
    stored_app, stored_profile, tagged, tap, typed,
};
use super::*;
use crate::app::profile_screen::{EditorField, ProfilePage, Side};
use proto::{ProfileReply, ProfileRequest, ProposalState, RepoProfile, RunRequest};
use std::time::{Duration, Instant};

fn show(proposed: bool) -> ProfileRequest {
    ProfileRequest::Show {
        dir: dir(),
        proposed,
    }
}

fn shows(effects: &[Effect], proposed: bool) -> usize {
    profile_requests(effects)
        .iter()
        .filter(|r| **r == show(proposed))
        .count()
}

fn expire(app: &mut App, id: u64) {
    app.set_reply_sent_at(id, Instant::now() - Duration::from_secs(31));
    app.on_tick();
}

/// Minor 1: a detection that leaves the running states for `Failed` fetches the
/// proposal again, and the old proposal's rows are not shown meanwhile.
#[test]
fn a_failed_detection_fetches_the_proposal_again() {
    let (mut app, _) = stored_app();
    tap(&mut app, KeyCode::Char('d'));
    let id = tagged(&tap(&mut app, KeyCode::Char('y')))[0].0;
    let done = ProfileReply::Done {
        message: "detection started".into(),
    };
    let status_id = tagged(&reply(&mut app, id, done))[0].0;
    reply(&mut app, status_id, status(Some(ProposalState::Scouting)));
    let effects = app.screens_tick(Instant::now() + Duration::from_secs(2));
    let poll_id = tagged(&effects)[0].0;
    let failed = ProposalState::Failed {
        reason: "scout exited".into(),
    };
    let effects = reply(&mut app, poll_id, status(Some(failed)));
    assert_eq!(shows(&effects, true), 1, "{effects:?}");
    assert_eq!(screen(&app).proposal, Side::Loading);
    assert!(!screen(&app).showing_card(), "not the old proposal");
}

/// Minor 2: an expired `Show` is the screen's: no toast, its side says so, and the
/// 1 s tick asks again once (never a second while one is out); the reply fills it.
#[test]
fn an_expired_show_fails_its_side_and_is_asked_again() {
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Ready)));
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    expire(&mut app, ids[2]);
    assert_eq!(app.toast_text(), None, "the screen owns its views");
    assert_eq!(
        screen(&app).proposal,
        Side::Failed("no reply from daemon".into())
    );
    let t0 = Instant::now();
    let effects = app.screens_tick(t0 + Duration::from_secs(2));
    assert_eq!(shows(&effects, true), 1, "{effects:?}");
    assert_eq!(shows(&effects, false), 0, "the ready side is not asked");
    let id = tagged(&effects)[0].0;
    for s in 3..=10 {
        let effects = app.screens_tick(t0 + Duration::from_secs(s));
        assert_eq!(shows(&effects, true), 0, "one out at a time ({s} s)");
    }
    reply(&mut app, id, shown(&stored_profile(), vec![]));
    assert!(matches!(screen(&app).proposal, Side::Ready(_)));
    assert!(app.screens_tick(t0 + Duration::from_secs(20)).is_empty());
}

/// Minor 2: a `Show` whose send the connection refused fails its side with the reason,
/// and the tick asks again a second later, not at once.
#[test]
fn an_unsent_show_fails_its_side_and_is_asked_again() {
    let mut app = open_app();
    let ids = open(&mut app);
    app.on_send_failed(&ClientMsg::RunTagged {
        id: ids[1],
        request: RunRequest::Profile(show(false)),
    });
    assert_eq!(app.toast_text(), None);
    assert_eq!(
        screen(&app).stored,
        Side::Failed("not sent: daemon is not responding".into())
    );
    let text = render(&app);
    assert!(
        text.contains("not sent: daemon is not responding"),
        "{text}"
    );
    assert_eq!(shows(&app.on_tick(), false), 0, "not within the second");
    let effects = app.screens_tick(Instant::now() + Duration::from_secs(2));
    assert_eq!(shows(&effects, false), 1, "{effects:?}");
}

/// Minor 2: an expired `Status` with nothing shown yet is the screen's too: it says so
/// and is asked again; a refused one is not retried.
#[test]
fn an_expired_status_says_so_and_is_asked_again() {
    let mut app = open_app();
    let ids = open(&mut app);
    expire(&mut app, ids[0]);
    assert_eq!(app.toast_text(), None);
    let text = render(&app);
    assert!(text.contains("no reply from daemon"), "{text}");
    let effects = app.screens_tick(Instant::now() + Duration::from_secs(2));
    let asked = tagged(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    assert_eq!(
        asked[0].1,
        RunRequest::Profile(ProfileRequest::Status { dir: dir() })
    );
    let refused = ProfileReply::Refused {
        message: "not a git project".into(),
    };
    reply(&mut app, asked[0].0, refused);
    assert!(
        app.screens_tick(Instant::now() + Duration::from_secs(5))
            .is_empty()
    );
}

/// Minor 2 with decision 16: a late view for the open screen still loading on it is
/// applied; once the screen is closed, a late refusal is neither applied nor toasted.
#[test]
fn a_late_view_fills_the_loading_screen() {
    let mut app = open_app();
    let ids = open(&mut app);
    expire(&mut app, ids[1]);
    assert_eq!(
        screen(&app).stored,
        Side::Failed("no reply from daemon".into())
    );
    reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
    assert!(matches!(screen(&app).stored, Side::Ready(_)));
    expire(&mut app, ids[2]);
    tap(&mut app, KeyCode::Esc);
    let refusal = ProfileReply::Refused {
        message: "no proposal for /p/shop".into(),
    };
    assert!(reply(&mut app, ids[2], refusal).is_empty());
    assert_eq!(app.toast_text(), None);
    assert_eq!(app.screen, None);
}

/// Minor 4: a page's request while disconnected toasts `not connected` and keeps the
/// page open with what was typed.
#[test]
fn a_disconnected_page_keeps_its_edit() {
    let (mut app, _) = ready_app();
    select(&mut app, "check");
    tap(&mut app, KeyCode::Char('e'));
    typed(&mut app, " --all");
    app.on_link_lost("gone");
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    match &screen(&app).page {
        Some(ProfilePage::Edit(e)) => match &e.field {
            // The card edits the proposal, from its value.
            EditorField::Line(area) => assert_eq!(area.text(), "cargo test --workspace --all"),
            other => panic!("{other:?}"),
        },
        other => panic!("the page closed: {other:?}"),
    }
    // A discard page, too.
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('x'));
    assert!(tap(&mut app, KeyCode::Char('y')).is_empty());
    assert_eq!(screen(&app).page, Some(ProfilePage::Discard));
}

/// Decision 37: a proposal `Show` sent before a detection and answered after the
/// detection left the running states is not the side's latest request: it is dropped,
/// the side stays `loading…`, and the re-fetch's reply fills it. While the old request
/// is still out, once it expired before the detection ended (its late reply), and when
/// it expires after the re-fetch went out (its expiry fails nothing).
#[test]
fn a_late_pre_detection_show_is_dropped() {
    // When the old request expires: never, before the detection ends, after the re-fetch.
    for expired in ["never", "before", "after"] {
        let mut app = open_app();
        let ids = open(&mut app);
        reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
        reply(&mut app, ids[1], shown(&stored_profile(), vec![]));
        if expired == "before" {
            expire(&mut app, ids[2]);
        }
        let effects = app.screens_tick(Instant::now() + Duration::from_secs(2));
        let poll_id = tagged(&effects)[0].0;
        let effects = reply(&mut app, poll_id, status(Some(ProposalState::Ready)));
        assert_eq!(shows(&effects, true), 1, "{effects:?}");
        let refetch = tagged(&effects)[0].0;
        assert_eq!(screen(&app).proposal, Side::Loading, "expired {expired}");
        if expired == "after" {
            expire(&mut app, ids[2]);
            assert_eq!(
                screen(&app).proposal,
                Side::Loading,
                "its expiry fails nothing"
            );
            assert_eq!(app.toast_text(), None, "the screen's own view");
        }
        let old = RepoProfile {
            check: Some("old check".into()),
            ..RepoProfile::default()
        };
        assert!(reply(&mut app, ids[2], shown(&old, vec![])).is_empty());
        assert_eq!(
            screen(&app).proposal,
            Side::Loading,
            "expired {expired}: dropped"
        );
        assert_eq!(app.toast_text(), None);
        reply(&mut app, refetch, shown(&stored_profile(), vec![]));
        match &screen(&app).proposal {
            Side::Ready(shown) => assert!(shown.toml.contains("cargo test"), "{shown:?}"),
            other => panic!("expired {expired}: {other:?}"),
        }
    }
}

/// Decision 34: one `Status` a second while the detection runs, never two in flight;
/// none after `Esc`; none once `Ready`, which fetches the proposal once.
#[test]
fn a_running_detection_polls_once_a_second() {
    let mut app = open_app();
    let ids = open(&mut app);
    let t0 = Instant::now();
    let at = |tenths: u64| t0 + Duration::from_millis(100 * tenths);
    // Nothing runs yet: no poll.
    assert_eq!(
        super::profile_screen::statuses(&app.screens_tick(at(30))),
        0
    );
    reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
    let mut sent = Vec::new();
    for tick in 1..=10 {
        let effects = app.screens_tick(at(tick));
        if super::profile_screen::statuses(&effects) > 0 {
            sent.push((tick, tagged(&effects)[0].0));
        }
    }
    assert_eq!(sent.len(), 1, "one per second: {sent:?}");
    assert_eq!(sent[0].0, 10);
    // Unanswered: three more seconds send nothing.
    for tick in 11..=40 {
        assert_eq!(
            super::profile_screen::statuses(&app.screens_tick(at(tick))),
            0,
            "in flight at {tick}"
        );
    }
    // Answered, still running: the next one goes.
    reply(&mut app, sent[0].1, status(Some(ProposalState::Verifying)));
    let effects = app.screens_tick(at(41));
    assert_eq!(super::profile_screen::statuses(&effects), 1);
    let id = tagged(&effects)[0].0;
    // Ready: the proposal is fetched once, and the polling stops.
    let effects = reply(&mut app, id, status(Some(ProposalState::Ready)));
    assert_eq!(
        profile_requests(&effects),
        vec![ProfileRequest::Show {
            dir: dir(),
            proposed: true
        }]
    );
    for tick in 42..=80 {
        assert_eq!(
            super::profile_screen::statuses(&app.screens_tick(at(tick))),
            0,
            "ready at {tick}"
        );
    }
    // A proposal already ready at open was asked for by the open: nothing more.
    let mut app = open_app();
    let ids = open(&mut app);
    assert!(reply(&mut app, ids[0], status(Some(ProposalState::Ready))).is_empty());
    // Esc stops it too.
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Preparing)));
    tap(&mut app, KeyCode::Esc);
    for tick in 1..=30 {
        assert!(app.screens_tick(at(tick)).is_empty());
    }
    // `on_tick` drives it: a Status once a second has passed since the last.
    let mut app = open_app();
    let ids = open(&mut app);
    reply(&mut app, ids[0], status(Some(ProposalState::Scouting)));
    assert_eq!(super::profile_screen::statuses(&app.on_tick()), 0);
    super::profile_screen::screen_mut(&mut app).status_sent_at =
        Some(Instant::now() - Duration::from_secs(2));
    assert_eq!(super::profile_screen::statuses(&app.on_tick()), 1);
}

fn render(app: &App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..24)
        .map(|y| (0..80).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect()
}
