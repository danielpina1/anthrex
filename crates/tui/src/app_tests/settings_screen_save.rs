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
    DISCARD_ASK, LEAVE_SETTINGS_FIRST, SAVED, SaveOutcome, SettingsPage, UNSAVED_FIRST,
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
