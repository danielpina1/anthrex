//! Milestone 9.10.9: a row's ✗ and `checking…` (decision 31), the narrow columns, the
//! status bar's hints by view (decision 29) and the error rows. Split from
//! `profile_tests.rs` (`AGENTS.md` rule 8).

use super::tests::{
    SIZES, app_with_profile, interior, record, screen_mut, screen_text, select, sp_app, status_of,
};
use crate::app::App;
use crate::app::profile_screen::ProfileScreen;
use crate::theme::{Role, role};
use crate::ui::audit;
use proto::{ProposalOrigin, ProposalState, RowEdit, RowEditState};

/// Decision 31: a ✗ row edit draws `✗` and the choices in place of the hint; one
/// being checked draws `checking…`; `o` shows the failed check's output.
#[test]
fn a_failed_row_draws_its_cross_and_choices() {
    let mut app = sp_app(false);
    let failed = role(Role::Failed, app.palette()).fg.unwrap();
    let working = role(Role::Working, app.palette()).fg.unwrap();
    let edit = |state| {
        let mut st = status_of(true, None);
        let mut r = record(
            ProposalState::Ready,
            ProposalOrigin::Edit {
                keys: vec!["check".into()],
            },
        );
        r.edit = Some(RowEdit {
            key: "check".into(),
            value: Some("cargo nextest run".into()),
            state,
        });
        st.proposal = Some(r);
        st
    };
    select(&mut app, "check");
    let s = screen_mut(&mut app);
    s.status = Some(edit(RowEditState::Failed {
        reason: "exit 101 after 3s".into(),
        tail: "error[E0425]: cannot find value".into(),
        secs: 3,
    }));
    s.expanded = Some("check".into());
    for (w, h) in SIZES {
        let buffer = audit::draw(&app, w, h);
        let rows = interior(&buffer, false).rows;
        let all = rows.join("\n");
        let row = rows
            .iter()
            .find(|r| r.contains("cargo test --workspace"))
            .unwrap();
        let &(x, y) = audit::find(&buffer, "✗")
            .first()
            .unwrap_or_else(|| panic!("{all}"));
        assert!(row.contains('✗'), "{all}");
        // Final review C-I1: the value that failed is drawn, beside the one kept.
        if w == 120 {
            assert!(
                row.contains("cargo test --workspace → cargo nextest run"),
                "{all}"
            );
            assert!(
                all.contains("your edit of check (cargo nextest run) failed its check; the profile is unchanged"),
                "{all}"
            );
        }
        assert_eq!(buffer[(x, y)].fg, failed);
        let choices = "couldn't verify: exit 101 after 3s · o output · s save anyway · r revert";
        assert!(
            rows.iter().any(|r| r.trim_start().starts_with(choices)),
            "{w}x{h}\n{all}"
        );
        assert!(all.contains("error[E0425]: cannot find value"), "{all}");
        assert!(
            !all.contains("check — the command"),
            "the ✗ line replaces the hint"
        );
    }
    screen_mut(&mut app).status = Some(edit(RowEditState::Verifying));
    let buffer = audit::draw(&app, 120, 40);
    let &(x, y) = audit::find(&buffer, "checking…")
        .first()
        .expect("checking…");
    assert_eq!(buffer[(x, y)].fg, working);
}

/// Under 60 interior columns the key column goes first; every label stays.
#[test]
fn narrow_drops_the_key_column_first() {
    let app = sp_app(false);
    let text = screen_text(&app, 50, 24);
    for key in ["single_test", "test_paths", "delivery.mode"] {
        assert!(!text.contains(key), "{key}\n{text}");
    }
    for label in [
        "setup",
        "check",
        "single test",
        "source",
        "tests",
        "generated",
        "protected",
        "Delivery",
    ] {
        assert!(text.contains(label), "{label}\n{text}");
    }
    assert!(text.contains('✓'), "the cells stay\n{text}");
    let text = screen_text(&app, 36, 24);
    assert!(!text.contains('✓'), "then the cells go\n{text}");
    assert!(text.contains("single test"), "{text}");
}

fn pairs(s: &ProfileScreen) -> Vec<(String, String)> {
    crate::ui::profile::hints(s)
        .into_iter()
        .map(|h| (h.key, h.word))
        .collect()
}

/// Decision 29's order, by view; they drop from the right (`esc` last of all).
#[test]
fn hints_follow_the_view() {
    let words = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(k, w)| (k.to_string(), w.to_string()))
            .collect()
    };
    let mut app = sp_app(false);
    let s = screen_mut(&mut app);
    assert_eq!(
        pairs(s),
        words(&[
            ("⏎", "open"),
            ("e", "edit"),
            ("u", "unset"),
            ("a", "advanced"),
            ("d", "detect again"),
            ("o", "output"),
            ("esc", "back")
        ])
    );
    s.status = Some(status_of(false, Some(ProposalState::Scouting)));
    assert_eq!(
        pairs(s),
        words(&[
            ("⏎", "open"),
            ("e", "edit"),
            ("u", "unset"),
            ("a", "advanced"),
            ("d", "set up"),
            ("o", "output"),
            ("x", "discard proposal"),
            ("esc", "back")
        ])
    );
    s.status = Some(status_of(false, Some(ProposalState::Ready)));
    assert_eq!(
        pairs(s),
        words(&[
            ("⏎", "use this"),
            ("e", "edit"),
            ("u", "unset"),
            ("a", "advanced"),
            ("o", "output"),
            ("x", "discard"),
            ("esc", "later")
        ])
    );
    let mut st = status_of(true, None);
    let mut r = record(
        ProposalState::Ready,
        ProposalOrigin::Edit {
            keys: vec!["setup".into()],
        },
    );
    r.edit = Some(RowEdit {
        key: "setup".into(),
        value: None,
        state: RowEditState::Failed {
            reason: "x".into(),
            tail: String::new(),
            secs: 1,
        },
    });
    st.proposal = Some(r);
    s.status = Some(st);
    s.selected = 0;
    assert_eq!(
        pairs(s),
        words(&[
            ("⏎", "open"),
            ("e", "edit"),
            ("u", "unset"),
            ("a", "advanced"),
            ("d", "detect again"),
            ("o", "output"),
            ("s", "save anyway"),
            ("r", "revert"),
            ("esc", "back")
        ])
    );
    let hints = crate::ui::profile::hints(s);
    let (esc, rest) = hints.split_last().unwrap();
    assert!(
        rest.windows(2).all(|p| p[0].priority > p[1].priority),
        "dropped from the right"
    );
    assert!(
        rest.iter().all(|h| h.priority < esc.priority),
        "esc stays longest"
    );
}

#[test]
fn an_error_row_shows_the_daemons_text() {
    let mut app = app_with_profile(|_, _| {});
    screen_mut(&mut app).error = Some("run r1 is live in /p/shop; edit later".into());
    for (w, h) in SIZES {
        assert!(screen_text(&app, w, h).contains("run r1 is live in /p/shop"));
    }
    // Decision 23: a failed set-up's reason is the error row.
    let mut app = sp_app(false);
    screen_mut(&mut app).status = Some(status_of(
        false,
        Some(ProposalState::Failed {
            reason: "the scout timed out".into(),
        }),
    ));
    assert!(screen_text(&app, 80, 24).contains("setting up failed: the scout timed out"));
}

/// Principle 6: a refusal longer than the footer's three lines is marked cut.
#[test]
fn a_cut_refusal_is_marked() {
    let mut app = app_with_profile(|_, _| {});
    screen_mut(&mut app).error = Some("word ".repeat(200));
    let text = screen_text(&app, 80, 24);
    assert!(text.contains("word …"), "{text}");
    screen_mut(&mut app).error = Some("short refusal".into());
    let text = screen_text(&app, 80, 24);
    assert!(
        !text.contains("word …") && text.contains("short refusal"),
        "{text}"
    );
}

/// M9.10.9 fix round 1 (review minor 3): the footer's precedence. The daemon's last
/// text (a refusal, else a `Done` such as `saved <label>`) comes first, then a failed
/// set-up's reason on its own lines: neither hides the other.
#[test]
fn the_footer_shows_the_last_text_then_a_failed_set_up() {
    let mut app = sp_app(false);
    let s = screen_mut(&mut app);
    s.status = Some(status_of(
        true,
        Some(ProposalState::Failed {
            reason: "the scout timed out".into(),
        }),
    ));
    s.message = Some("saved check".into());
    for (w, h) in SIZES {
        let rows = interior(&audit::draw(&app, w, h), false).rows;
        let at = |t: &str| rows.iter().position(|r| r.contains(t));
        let (saved, failed) = (at("saved check"), at("setting up failed: the scout"));
        assert!(saved.is_some() && failed.is_some(), "{}", rows.join("\n"));
        assert!(saved < failed, "{}", rows.join("\n"));
    }
    screen_mut(&mut app).error = Some("finish or cancel the run".into());
    let text = screen_text(&app, 120, 40);
    assert!(text.contains("finish or cancel the run"), "{text}");
    assert!(text.contains("setting up failed: the scout"), "{text}");
}

/// M9.10.9 fix round 1 (review minor 4): at 80x24 the folded Advanced summary is not
/// cut; it goes under its head when the line is too narrow.
#[test]
fn the_advanced_summary_is_whole_at_80x24() {
    for ascii in [false, true] {
        let app = sp_app(ascii);
        let rows = interior(&audit::draw(&app, 80, 24), ascii).rows;
        let summary = crate::profile_words::ADVANCED_SUMMARY;
        assert!(
            rows.iter().any(|r| r.contains(summary)),
            "{ascii}\n{}",
            rows.join("\n")
        );
    }
}

/// M9.10.9 fix round 1 (review minor 5): the hint line's rows are kept while the
/// selection is on a row without a hint, so the list window does not jump.
#[test]
fn the_list_window_keeps_its_height_without_a_hint() {
    let mut app = sp_app(false);
    let height = |app: &mut App, key: &str| {
        select(app, key);
        let Some(crate::app::screens::Screen::Profile(s)) = &app.screen else {
            unreachable!()
        };
        crate::ui::profile::body_lines(app, s, 78).len()
    };
    let with_hint = height(&mut app, "check");
    assert_eq!(
        height(&mut app, crate::app::profile_screen::ADVANCED_ROW),
        with_hint
    );
}

/// Final review C-I1: a ✗ row's page names the value that failed, above its reason.
#[test]
fn a_failed_rows_page_names_the_value_that_failed() {
    let mut app = sp_app(false);
    let mut st = status_of(true, None);
    let mut r = record(
        ProposalState::Ready,
        ProposalOrigin::Edit {
            keys: vec!["check".into()],
        },
    );
    r.edit = Some(RowEdit {
        key: "check".into(),
        value: Some("\"cargo nextest run\"".into()),
        state: RowEditState::Failed {
            reason: "exit 101 after 3s".into(),
            tail: String::new(),
            secs: 3,
        },
    });
    st.proposal = Some(r);
    let s = screen_mut(&mut app);
    s.status = Some(st);
    let page = crate::app::profile_screen::ProfilePage::Row {
        key: "check".into(),
        scroll: 0,
    };
    let s = screen_mut(&mut app);
    s.page = Some(page.clone());
    let s = match &app.screen {
        Some(crate::app::screens::Screen::Profile(s)) => s,
        _ => unreachable!(),
    };
    let lines: Vec<String> = crate::ui::profile::page_lines(&app, s, &page, 76)
        .into_iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    let at = |text: &str| {
        lines
            .iter()
            .position(|l| l == text)
            .unwrap_or_else(|| panic!("{text:?} in {lines:#?}"))
    };
    assert!(at("your edit: cargo nextest run") < at("couldn't verify: exit 101 after 3s"));
}
