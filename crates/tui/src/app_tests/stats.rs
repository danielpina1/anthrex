//! Milestone 9.0.6 task 15, decision 38: the run-history stats screen, opened from the
//! menu's `Stats`. Its one tagged `Stats { dir }` comes back by `request_id`; a late
//! reply, or one for an older request, changes nothing; an expiry or a lost link never
//! leaves it loading. No test sleeps (Global Constraint 11).

use super::actions::{running_app, sent_tagged_id, tap};
use super::*;
use crate::actions_request::ActionTarget;
use crate::app::screens::Screen;
use crate::app::stats::{StatsScreen, StatsState};
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::stats::tests::history;
use proto::{ActionKind, HistoryStats, RunReply, RunRequest};
use ratatui::layout::Rect;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn project(app: &App) -> PathBuf {
    app.runs.runs[0].project.clone()
}

pub(crate) fn stats_screen(app: &App) -> &StatsScreen {
    match &app.screen {
        Some(Screen::Stats(s)) => s,
        other => panic!("no stats screen: {:?}", other.is_some()),
    }
}

/// The menu on the run with `Stats` preselected, then Enter: the request's id.
fn open(app: &mut App) -> u64 {
    app.open_actions((RUN_ID.into(), ActionTarget::Run), Some(ActionKind::Stats));
    let effects = tap(app, KeyCode::Enter);
    sent_tagged_id(&effects, |r| matches!(r, RunRequest::Stats { .. }))
}

fn reply(app: &mut App, id: u64, stats: HistoryStats) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Stats {
        stats,
        request_id: Some(id),
    }))
}

fn refuse(app: &mut App, id: u64, message: &str) -> Vec<Effect> {
    app.on_daemon(DaemonMsg::Run(RunReply::Refused {
        request: "stats".into(),
        message: message.into(),
        request_id: Some(id),
    }))
}

/// Preflight F24 is gone: `Stats` sends one tagged `Stats { dir: run.project }`, closes
/// the menu and opens the screen loading on that request.
#[test]
fn the_stats_entry_sends_stats_for_the_runs_project() {
    let mut app = running_app();
    let dir = project(&app);
    app.open_actions((RUN_ID.into(), ActionTarget::Run), Some(ActionKind::Stats));
    let effects = tap(&mut app, KeyCode::Enter);
    let id = sent_tagged_id(&effects, |r| {
        *r == RunRequest::Stats {
            dir: dir.clone(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        }
    });
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert!(app.modal.is_none(), "the menu closes");
    assert_eq!(app.toast_text(), None);
    let s = stats_screen(&app);
    assert_eq!(s.project, dir);
    assert_eq!(s.state, StatsState::Loading(id));
    assert_eq!(s.scroll, 0);
    assert!(app.replies.contains(id));
}

/// The reply fills the screen; a refusal is shown on the screen, never toasted.
#[test]
fn the_reply_fills_the_screen_and_a_refusal_shows_inline() {
    let mut app = running_app();
    let id = open(&mut app);
    assert!(reply(&mut app, id, history()).is_empty());
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Ready(Box::new(history()))
    );
    assert!(!app.replies.contains(id));
    assert_eq!(app.toast_text(), None);

    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());
    let id = open(&mut app);
    assert!(refuse(&mut app, id, "no history for /r/demo\nsecond line").is_empty());
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("no history for /r/demo\nsecond line".into())
    );
    assert_eq!(app.toast_text(), None, "shown inline, not toasted");
}

/// A reply for a closed screen, or for an older request than the one the screen waits
/// on, is dropped: no toast, no screen, no change.
#[test]
fn a_late_or_older_reply_is_dropped() {
    let mut app = running_app();
    let first = open(&mut app);
    tap(&mut app, KeyCode::Esc);
    assert!(reply(&mut app, first, history()).is_empty());
    assert!(app.screen.is_none(), "a closed screen stays closed");
    assert_eq!(app.toast_text(), None);

    let older = open(&mut app);
    tap(&mut app, KeyCode::Esc);
    let newer = open(&mut app);
    assert!(refuse(&mut app, older, "old").is_empty());
    assert_eq!(app.toast_text(), None);
    assert_eq!(stats_screen(&app).state, StatsState::Loading(newer));
    reply(&mut app, newer, history());
    assert!(matches!(stats_screen(&app).state, StatsState::Ready(_)));
}

/// Decision 19 and ruling R-a: with no reply within `REPLY_TIMEOUT` the screen says so
/// (its own feedback, as the Settings screen's save, so no toast). Ruling R-b
/// (decision 16): a late reply to the screen's own request still fills it.
#[test]
fn an_expired_request_fails_the_screen_and_its_late_reply_still_fills_it() {
    let mut app = running_app();
    let id = open(&mut app);
    app.set_reply_sent_at(id, Instant::now() - Duration::from_secs(29));
    app.on_tick();
    assert_eq!(stats_screen(&app).state, StatsState::Loading(id), "not due");
    app.set_reply_sent_at(id, Instant::now() - Duration::from_secs(31));
    app.on_tick();
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("no reply from daemon".into())
    );
    assert_eq!(app.toast_text(), None, "the screen says it");
    assert!(reply(&mut app, id, history()).is_empty());
    assert_eq!(app.toast_text(), None);
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Ready(Box::new(history()))
    );
}

/// Ruling R-b's limit: a late reply to any other request (an older screen's) is
/// dropped, pending or expired, and leaves the open screen as it is.
#[test]
fn a_late_reply_to_another_request_is_dropped() {
    let mut app = running_app();
    let older = open(&mut app);
    tap(&mut app, KeyCode::Esc);
    let newer = open(&mut app);
    let past = Instant::now() - Duration::from_secs(31);
    app.set_reply_sent_at(older, past);
    app.set_reply_sent_at(newer, past);
    app.on_tick();
    let failed = StatsState::Failed("no reply from daemon".into());
    assert_eq!(stats_screen(&app).state, failed);
    // The closed screen's request is no screen's own: its expiry is toasted.
    assert_eq!(app.toast_text(), Some("no reply from daemon"));
    app.toast = None;
    assert!(reply(&mut app, older, history()).is_empty());
    assert!(refuse(&mut app, older, "old").is_empty());
    assert_eq!(app.toast_text(), None);
    assert_eq!(stats_screen(&app).state, failed);
    refuse(&mut app, newer, "late refusal");
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("late refusal".into())
    );
}

/// A lost link drops the request; the screen says `not connected` at the next tick
/// and asks again once reconnected.
#[test]
fn a_lost_link_fails_the_screen_and_a_reconnect_asks_again() {
    let mut app = running_app();
    let dir = project(&app);
    open(&mut app);
    app.on_link_lost("gone");
    app.on_tick();
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("not connected".into())
    );
    let effects = app.on_reconnected(app.windows.clone());
    let again: Vec<u64> = effects
        .iter()
        .filter_map(|e| match e {
            Effect::Send(ClientMsg::RunTagged {
                id,
                request: RunRequest::Stats { dir: d, .. },
            }) if *d == dir => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(again.len(), 1, "{effects:?}");
    assert_eq!(stats_screen(&app).state, StatsState::Loading(again[0]));
    reply(&mut app, again[0], history());
    assert!(matches!(stats_screen(&app).state, StatsState::Ready(_)));
}

/// A refused send is not waited on: the screen says so at once (not after 30 s, and
/// not at the next tick), in its own words.
#[test]
fn a_refused_send_fails_the_screen() {
    let mut app = running_app();
    let dir = project(&app);
    let id = open(&mut app);
    app.on_send_failed(&ClientMsg::RunTagged {
        id,
        request: RunRequest::Stats {
            dir,
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        },
    });
    assert!(!app.replies.contains(id));
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("not sent: daemon is not responding".into())
    );
    assert_eq!(
        app.toast_text(),
        None,
        "the screen owns it (final review I1)"
    );
    app.on_tick();
    assert_eq!(
        stats_screen(&app).state,
        StatsState::Failed("not sent: daemon is not responding".into())
    );
}

/// `j`/`k`/Down/Up move one line, PgDn/PgUp ten, between the first line and the
/// renderer's own last first line over the reported body (`ui::stats::max_scroll`), so
/// no press is dead; a taller body clamps first. Esc leaves.
#[test]
fn stats_scroll() {
    let mut app = running_app();
    // 13 rows inside the frame for the sample's 16 lines: the last first line is 4.
    app.set_body_area(Rect::new(0, 0, 80, 15));
    let id = open(&mut app);
    reply(&mut app, id, history());
    let scroll = |app: &App| stats_screen(app).scroll;
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(scroll(&app), 0);
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Down);
    assert_eq!(scroll(&app), 2);
    tap(&mut app, KeyCode::Up);
    assert_eq!(scroll(&app), 1);
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(scroll(&app), 4);
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(scroll(&app), 4);
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(scroll(&app), 3, "the first press back moves the view");
    tap(&mut app, KeyCode::PageUp);
    assert_eq!(scroll(&app), 0);
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(scroll(&app), 4);
    // A taller body (15 rows inside): the last first line is 2, and `k` steps from it.
    app.set_body_area(Rect::new(0, 0, 80, 17));
    tap(&mut app, KeyCode::Char('k'));
    assert_eq!(scroll(&app), 1, "clamped first");
    app.set_body_area(Rect::new(0, 0, 80, 40));
    tap(&mut app, KeyCode::Char('j'));
    assert_eq!(scroll(&app), 0, "everything fits");
    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());
    assert!(!app.keymap.screen_mode());
}

/// A refusal is drawn wrapped, and scrolls by its wrapped lines.
#[test]
fn a_tall_refusal_scrolls() {
    let mut app = running_app();
    app.set_body_area(Rect::new(0, 0, 80, 10));
    let id = open(&mut app);
    let text: Vec<String> = (0..20).map(|i| format!("line {i}")).collect();
    refuse(&mut app, id, &text.join("\n"));
    tap(&mut app, KeyCode::PageDown);
    assert_eq!(stats_screen(&app).scroll, 10);
    tap(&mut app, KeyCode::PageDown);
    // 20 lines in 8 rows: the last first line is 13.
    assert_eq!(stats_screen(&app).scroll, 13);
}

/// Ruling R-c: the menu's `Stats` opens over the plan review; Esc gives the review back
/// unchanged, with its keys.
#[test]
fn esc_from_stats_returns_to_the_plan_review() {
    let mut app = super::actions::gate_app();
    app.open_plan_review(RUN_ID.into(), crate::app::ReviewTarget::Gate);
    let before = app.plan_review.clone();
    assert!(before.is_some() && app.keymap.review_mode());
    open(&mut app);
    assert!(matches!(app.screen, Some(Screen::Stats(_))));
    tap(&mut app, KeyCode::Char('j'));
    tap(&mut app, KeyCode::Esc);
    assert!(app.screen.is_none());
    assert_eq!(app.plan_review, before);
    assert!(app.keymap.review_mode() && !app.keymap.screen_mode());
}

/// `C-b a`, `C-b m` and `C-b t` would act under the screen, so they are refused as on
/// the other screens; `C-b S` replaces it (decision 33).
#[test]
fn the_screen_refuses_what_would_act_under_it() {
    let mut app = running_app();
    open(&mut app);
    for c in ['a', 'm', 't'] {
        prefix(&mut app);
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        assert_eq!(app.toast_text(), Some("leave the stats first (esc)"), "{c}");
        assert!(matches!(app.screen, Some(Screen::Stats(_))), "{c}");
        app.toast = None;
    }
    prefix(&mut app);
    press(&mut app, KeyCode::Char('S'), KeyModifiers::SHIFT);
    assert!(matches!(app.screen, Some(Screen::Settings(_))));
}

/// Ruling R-b's `own` guard, pinned past the quiet list's 32 ids: the screen's expired
/// request is pushed off the list by 33 further quiet expiries, and its late reply still
/// fills the screen.
#[test]
fn a_late_reply_fills_the_screen_after_the_quiet_list_forgot_it() {
    let mut app = running_app();
    let dir = project(&app);
    let id = open(&mut app);
    let past = Instant::now() - Duration::from_secs(31);
    app.set_reply_sent_at(id, past);
    app.on_tick();
    assert!(app.replies.expired_quietly(id));
    for n in 0..33 {
        let other = 50_000 + n;
        let what = crate::app::replies::PendingWhat::Stats { dir: dir.clone() };
        app.replies
            .insert(other, what, crate::app::replies::REPLY_TIMEOUT);
        app.set_reply_sent_at(other, past);
        app.on_tick();
    }
    assert!(!app.replies.expired_quietly(id), "evicted");
    assert!(matches!(stats_screen(&app).state, StatsState::Failed(_)));
    reply(&mut app, id, history());
    assert!(matches!(stats_screen(&app).state, StatsState::Ready(_)));
}
