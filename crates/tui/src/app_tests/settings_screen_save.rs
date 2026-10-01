//! Milestone 9.0.6 task 14, decision 36: the Settings screen's save (`w`, `Saved`,
//! `Refused`, no reply, no link), leaving it (`esc`, `y`), the keys it holds, and the
//! cache reaching it. Split from `settings_screen.rs` (`AGENTS.md` hard rule 8).

use super::actions::tap;
use super::orch::tagged;
use super::settings_screen::{
    current, names, open, opened, origin, puts, reply, sample, screen, select, set_limit,
    settings_sent, w,
};
use super::*;
use crate::app::screens::Screen;
use crate::app::settings_screen::{
    DISCARD_ASK, LEAVE_SETTINGS_FIRST, LINK_LOST, NO_CHANGES, SAVED, SaveOutcome, SettingsPage,
    UNSAVED_FIRST,
};
use proto::SettingsReply;
use proto::settings::key;

#[test]
fn w_sends_put_and_saved_shows_the_line() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let effects = w(&mut app);
    let sent = puts(&effects);
    assert_eq!(sent.len(), 1, "{effects:?}");
    let (id, saved_doc) = sent[0].clone();
    assert_eq!(saved_doc.limits.max_readers, 4);
    assert!(app.settings_saving());
    assert!(w(&mut app).is_empty(), "one save at a time");
    let saved = SettingsReply::Saved {
        doc: saved_doc.clone(),
        origin: origin(&[]),
    };
    assert!(app.on_daemon(reply(saved, id)).is_empty());
    assert!(!app.settings_saving());
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::Saved));
    assert!(!screen(&app).dirty());
    assert_eq!(app.settings_cache.as_ref().unwrap().doc, saved_doc);
    let text = crate::ui::settings::tests::screen_text(&app, 80, 24);
    assert!(text.contains(SAVED), "{text}");
    assert_eq!(
        SAVED,
        "saved · new runs use these settings · runs in progress keep theirs"
    );
    // The next edit makes the line stale.
    tap(&mut app, KeyCode::Char('1'));
    assert_eq!(screen(&app).outcome, None);
}

#[test]
fn a_refused_put_lists_every_problem() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, _) = puts(&w(&mut app))[0].clone();
    let problems = vec![
        "orchestrator.agent is written in a form the settings screen does not edit".to_string(),
        "config.toml was not written within 5 s; nothing changed".to_string(),
    ];
    let effects = app.on_daemon(reply(
        SettingsReply::Refused {
            problems: problems.clone(),
        },
        id,
    ));
    // The cache re-syncs (progress ruling); the edits stay; nothing is saving.
    assert_eq!(settings_sent(&effects).len(), 1);
    assert!(!app.settings_saving());
    assert_eq!(
        screen(&app).outcome,
        Some(SaveOutcome::Refused(problems.clone()))
    );
    assert!(screen(&app).dirty());
    let text = crate::ui::settings::tests::screen_text(&app, 120, 40);
    for p in &problems {
        assert!(text.contains(p.as_str()), "{p}\n{text}");
    }
}

#[test]
fn a_put_with_no_reply_shows_so_and_stops_saving() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, _) = puts(&w(&mut app))[0].clone();
    app.set_reply_sent_at(
        id,
        std::time::Instant::now() - std::time::Duration::from_secs(60),
    );
    let effects = app.on_tick();
    assert_eq!(settings_sent(&effects).len(), 1, "the cache re-syncs");
    assert!(!app.settings_saving());
    assert_eq!(
        screen(&app).outcome,
        Some(SaveOutcome::Refused(vec!["no reply from daemon".into()]))
    );
    // A lost link mid-save leaves no `saving…` either.
    let (_, _) = puts(&w(&mut app))[0].clone();
    assert!(app.settings_saving());
    app.on_link_lost("gone");
    assert!(!app.settings_saving());
}

#[test]
fn disconnected_w_toasts_not_connected() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    app.on_link_lost("gone");
    assert!(w(&mut app).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
    assert_eq!(app.toast_level(), Some(ToastLevel::Warn));
    assert_eq!(screen(&app).put_id, None);
}

#[test]
fn esc_with_changes_asks_first() {
    let mut app = opened();
    tap(&mut app, KeyCode::Esc);
    assert_eq!(app.screen, None, "nothing changed: esc leaves at once");

    let mut app = opened();
    select(&mut app, 0);
    tap(&mut app, KeyCode::Char(' '));
    tap(&mut app, KeyCode::Esc);
    assert_eq!(screen(&app).page, Some(SettingsPage::Discard));
    // Discarding confirms only on `y`.
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert_eq!(app.toast_text(), Some("press y to discard"));
    assert_eq!(screen(&app).page, Some(SettingsPage::Discard));
    tap(&mut app, KeyCode::Esc);
    assert_eq!(screen(&app).page, None);
    assert!(screen(&app).dirty(), "back keeps the changes");
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('y'));
    assert_eq!(app.screen, None);
    assert_eq!(DISCARD_ASK, "discard unsaved settings? y");
    assert_eq!(app.settings_cache.as_ref().unwrap().doc, sample());
}

#[test]
fn the_screen_has_the_keys_while_it_is_open() {
    let mut app = opened();
    assert!(app.keymap.screen_mode());
    for k in ['a', 'm', 't'] {
        prefix(&mut app);
        press(&mut app, KeyCode::Char(k), KeyModifiers::NONE);
        assert_eq!(app.toast_text(), Some(LEAVE_SETTINGS_FIRST));
        assert!(matches!(app.screen, Some(Screen::Settings(_))));
    }
    // Unsaved changes are not dropped by opening the Profile screen over them.
    select(&mut app, 0);
    tap(&mut app, KeyCode::Char(' '));
    prefix(&mut app);
    press(&mut app, KeyCode::Char('P'), KeyModifiers::SHIFT);
    assert_eq!(app.toast_text(), Some(UNSAVED_FIRST));
    assert!(matches!(app.screen, Some(Screen::Settings(_))));
    // `C-b S` again keeps the open screen as it is.
    assert!(open(&mut app).is_empty());
    assert!(screen(&app).dirty());
}

#[test]
fn a_new_cache_reaches_an_unchanged_screen_only() {
    let mut app = opened();
    let mut other = sample();
    other.limits.max_bounces = 4;
    let (id, _) = tagged(&[app.settings_fetch()]);
    app.on_daemon(reply(current(other.clone(), origin(&[])), id));
    assert_eq!(screen(&app).doc(), Ok(other.clone()));
    // With changes of its own the screen keeps them.
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, _) = tagged(&[app.settings_fetch()]);
    app.on_daemon(reply(current(sample(), origin(&[])), id));
    assert_eq!(screen(&app).doc().unwrap().limits.max_readers, 4);
    assert_eq!(screen(&app).doc().unwrap().limits.max_bounces, 4);
}

#[test]
fn a_paste_reaches_only_the_custom_name() {
    let mut app = opened();
    app.on_paste("claude-y".into());
    assert_eq!(screen(&app).page, None);
    select(&mut app, 4);
    tap(&mut app, KeyCode::Enter);
    app.on_paste("claude-\u{1b}[31my\nz".into());
    tap(&mut app, KeyCode::Enter);
    let rows = names(&app, Runtime::Claude);
    let added = &rows.last().unwrap().0;
    assert!(added.starts_with("claude-") && !added.contains('\u{1b}') && !added.contains('\n'));
}

/// Fix round 1, ruling 1: editing stays allowed while a save is in flight, and a `Saved`
/// of the doc sent before those edits rebases on it instead of dropping them.
#[test]
fn a_saved_reply_keeps_edits_made_while_saving() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, sent) = puts(&w(&mut app))[0].clone();
    tap(&mut app, KeyCode::BackTab);
    tap(&mut app, KeyCode::BackTab);
    tap(&mut app, KeyCode::BackTab);
    select(&mut app, 0);
    tap(&mut app, KeyCode::Char(' '));
    let saved = SettingsReply::Saved {
        doc: sent.clone(),
        origin: origin(&[]),
    };
    app.on_daemon(reply(saved, id));
    assert!(names(&app, Runtime::Claude)[0].1, "the toggle survives");
    assert!(screen(&app).dirty());
    assert_eq!(screen(&app).base, sent);
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::Saved));
    assert_eq!(screen(&app).doc().unwrap().limits.max_readers, 4);
    // Unchanged since the send: the screen reloads on the saved doc.
    let (id, sent) = puts(&w(&mut app))[0].clone();
    let saved = SettingsReply::Saved {
        doc: sent.clone(),
        origin: origin(&[]),
    };
    app.on_daemon(reply(saved, id));
    assert!(!screen(&app).dirty());
}

/// Fix round 1, ruling 2: a link lost mid-save says so (not `saving…`, not nothing); a
/// re-sync whose doc is exactly the screen's (the save landed) is taken.
#[test]
fn a_lost_link_mid_save_says_so_and_a_landed_save_is_taken() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (_, sent) = puts(&w(&mut app))[0].clone();
    app.on_link_lost("gone");
    app.screens_tick(std::time::Instant::now());
    assert_eq!(screen(&app).put_id, None);
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::LinkLost));
    let text = crate::ui::settings::tests::screen_text(&app, 80, 24);
    assert!(text.contains(LINK_LOST), "{text}");
    assert_eq!(LINK_LOST, "not saved: link lost");
    // Another doc: the edits stay.
    let (id, _) = tagged(&[app.settings_fetch()]);
    app.on_daemon(reply(current(sample(), origin(&[])), id));
    assert!(screen(&app).dirty());
    // The saved doc: the screen takes it and the line goes.
    let (id, _) = tagged(&[app.settings_fetch()]);
    app.on_daemon(reply(current(sent, origin(&[])), id));
    assert!(!screen(&app).dirty());
    assert_eq!(screen(&app).outcome, None);
}

/// Fix round 1, ruling 3: the open screen owns its save's feedback (no toast for its
/// `Refused` or its expiry); with no screen open the toast stays.
#[test]
fn the_screens_own_save_is_not_toasted() {
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, _) = puts(&w(&mut app))[0].clone();
    let refused = || SettingsReply::Refused {
        problems: vec!["config.toml was not written within 5 s; nothing changed".into()],
    };
    app.on_daemon(reply(refused(), id));
    assert_eq!(app.toast_text(), None);
    let (id, _) = puts(&w(&mut app))[0].clone();
    app.set_reply_sent_at(
        id,
        std::time::Instant::now() - std::time::Duration::from_secs(60),
    );
    app.on_tick();
    assert_eq!(app.toast_text(), None);
    assert!(matches!(
        screen(&app).outcome,
        Some(SaveOutcome::Refused(_))
    ));
    // The screen discarded before the reply: the toast is the only feedback.
    let (id, _) = puts(&w(&mut app))[0].clone();
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('y'));
    assert_eq!(app.screen, None);
    app.on_daemon(reply(refused(), id));
    assert!(app.toast_text().unwrap().contains("not written within 5 s"));
}

/// Fix round 1, ruling 5.
#[test]
fn w_with_no_changes_sends_nothing() {
    let mut app = opened();
    assert!(w(&mut app).is_empty());
    assert_eq!(app.toast_text(), Some(NO_CHANGES));
    assert_eq!(NO_CHANGES, "no changes to save");
}

/// Final review I1: the open screen's own `Put` refused by the connection says so on
/// the screen (not a toast), by the link: `not sent: daemon is not responding` while
/// connected, `not saved: link lost` once the link went. A closed screen's still toasts.
#[test]
fn an_unsent_put_says_so_on_the_screen() {
    let unsent = |id: u64, settings: proto::SettingsDoc| ClientMsg::RunTagged {
        id,
        request: proto::RunRequest::Settings(proto::SettingsRequest::Put { settings }),
    };
    let mut app = opened();
    set_limit(&mut app, key::MAX_READERS, "4");
    let (id, doc) = puts(&w(&mut app))[0].clone();
    assert!(app.on_send_failed(&unsent(id, doc)).is_empty());
    assert!(!app.replies.contains(id));
    assert!(!app.settings_saving());
    assert_eq!(app.toast_text(), None);
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::NotSent));
    let text = crate::ui::settings::tests::screen_text(&app, 80, 24);
    assert!(
        text.contains("not sent: daemon is not responding"),
        "{text}"
    );
    // Disconnected: the link's words.
    let (id, doc) = puts(&w(&mut app))[0].clone();
    app.on_link_lost("gone");
    app.on_send_failed(&unsent(id, doc));
    assert_eq!(screen(&app).outcome, Some(SaveOutcome::LinkLost));
    assert_eq!(app.toast_text(), Some("connection to the daemon lost"));
    // A save whose screen was discarded: the toast is its only feedback.
    let windows = app.windows.clone();
    app.on_reconnected(windows);
    let (id, doc) = puts(&w(&mut app))[0].clone();
    tap(&mut app, KeyCode::Esc);
    tap(&mut app, KeyCode::Char('y'));
    app.on_send_failed(&unsent(id, doc));
    assert!(!app.replies.contains(id));
    assert_eq!(app.toast_text(), Some("daemon is not responding"));
}
