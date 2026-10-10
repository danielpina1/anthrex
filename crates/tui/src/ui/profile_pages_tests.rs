//! Milestone 9.10.9: the Profile screen's pages (decision 30) and what every view and
//! page draws of hostile text. Split from `profile_tests.rs` (`AGENTS.md` rule 8).

use super::tests::{
    SIZES, app_with_profile, render_text, screen_mut, screen_text, shown, sp_app, sp_profile,
    sp_verification, status_of,
};
use crate::app::App;
use crate::app::profile_screen::ProfilePage;
use crate::theme::{Role, role};
use crate::ui::audit;
use proto::{DroppedCommand, ProposalState};
use ratatui::{Terminal, backend::TestBackend};
use std::collections::BTreeMap;

#[test]
fn the_raw_text_page_shows_the_file() {
    let mut app = app_with_profile(|_, _| {});
    let toml = "check = \"cargo test\"\nsource = [\"src/**\"]\n\n# protected: built-in";
    screen_mut(&mut app).page = Some(ProfilePage::RawText {
        text: toml.into(),
        scroll: 0,
    });
    for (w, h) in SIZES {
        let text = screen_text(&app, w, h);
        for want in [
            "profile file",
            "check = \"cargo test\"",
            "source = [\"src/**\"]",
            "# protected: built-in",
            "j/k scroll",
        ] {
            assert!(text.contains(want), "{w}x{h}: {want}\n{text}");
        }
    }
}

/// Decision 30: the detect page's toggles in plain words, and the row page.
#[test]
fn the_pages_read_plainly() {
    let mut app = sp_app(false);
    screen_mut(&mut app).page = Some(ProfilePage::Detect {
        trust_project: false,
        unconfined_checks: true,
        focus: 0,
    });
    let text = screen_text(&app, 120, 40);
    for want in [
        "detect again",
        "Trust this repo's agent settings",
        "let agents use the settings files this repo tracks",
        ".claude/ and .mcp.json",
        "Run checks outside the sandbox",
        "only needed on systems where anthrex can't confine",
        "a real agent will read the repository",
        "y start",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    screen_mut(&mut app).page = Some(ProfilePage::Row {
        key: "protected".into(),
        scroll: 0,
    });
    let text = screen_text(&app, 120, 40);
    for want in [
        "protected",
        ".github/\n",
        "Cargo.lock",
        "files agents may not change, beyond the built-in ones",
    ] {
        assert!(text.contains(want.trim_end()), "{want}\n{text}");
    }
    let rows = audit::rows(&audit::draw(&app, 120, 40));
    assert!(
        rows.iter()
            .any(|r| r.contains("│ .github/ ") && !r.contains("Cargo.lock")),
        "one item per line\n{}",
        rows.join("\n")
    );
}

/// Review Focus 5: profile commands, check output tails and environment names can carry
/// ESC, U+202E and line separators.
#[test]
fn profile_screen_text_is_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let app = app_with_profile(|p, v| {
        p.check = Some(hostile.clone());
        v.check.as_mut().unwrap().tail = hostile.clone();
        p.env.insert("NAME".into(), hostile.clone());
        p.env.insert(hostile.clone(), "x".into());
    });
    for (w, h) in SIZES {
        assert_eq!(
            crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
            None
        );
    }
}

/// The same for the status line, the card, the error rows and every page.
#[test]
fn profile_screen_pages_are_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut app = app_with_profile(|p, _| {
        p.env.insert(hostile.clone(), hostile.clone());
        p.source = vec![hostile.clone()];
    });
    let mut p = sp_profile();
    p.check = Some(hostile.clone());
    let dropped = vec![DroppedCommand {
        key: hostile.clone(),
        command: hostile.clone(),
        reason: hostile.clone(),
        tail: hostile.clone(),
    }];
    let s = screen_mut(&mut app);
    s.proposal = shown(p, sp_verification(), dropped);
    s.error = Some(hostile.clone());
    s.message = Some(hostile.clone());
    let mut st = status_of(
        true,
        Some(ProposalState::Failed {
            reason: hostile.clone(),
        }),
    );
    st.stale = vec![hostile.clone()];
    st.unparseable = Some(hostile.clone());
    st.unreadable_text = Some(hostile.clone());
    s.status = Some(st);
    let pages = [
        None,
        Some(ProfilePage::RawText {
            text: hostile.clone(),
            scroll: 0,
        }),
        Some(ProfilePage::Unset {
            key: hostile.clone(),
            on_proposal: false,
        }),
        Some(ProfilePage::Discard),
        Some(ProfilePage::Detect {
            trust_project: true,
            unconfined_checks: false,
            focus: 1,
        }),
        Some(ProfilePage::Row {
            key: format!("env.{hostile}"),
            scroll: 0,
        }),
        Some(ProfilePage::Row {
            key: "source".into(),
            scroll: 0,
        }),
    ];
    // The profile, then the card.
    for card in [false, true] {
        if card {
            let mut st = status_of(true, Some(ProposalState::Ready));
            st.stale = vec![hostile.clone()];
            screen_mut(&mut app).status = Some(st);
        }
        for page in &pages {
            screen_mut(&mut app).page = page.clone();
            for (w, h) in SIZES {
                let text = render_text(&app, w, h);
                assert_eq!(
                    crate::safe_text::tests::first_hostile(&text),
                    None,
                    "{card} {page:?} {w}x{h}"
                );
            }
        }
    }
    // An editor over a hostile value.
    let s = screen_mut(&mut app);
    s.page = None;
    s.status = Some(status_of(true, None));
    let rows = s.rows();
    for (i, row) in rows.iter().enumerate() {
        let s = screen_mut(&mut app);
        s.selected = i;
        s.page = None;
        app.on_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('e'),
            crossterm::event::KeyModifiers::NONE,
        ));
        for (w, h) in SIZES {
            assert_eq!(
                crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
                None,
                "editor of {:?} {w}x{h}",
                row.key
            );
        }
    }
}

/// Control characters, ESC sequences and bidi overrides in a command value, a label's
/// source (an environment name), an old value and the unreadable file's text: none
/// reaches a drawn row or a span.
#[test]
fn hostile_text_never_reaches_a_drawn_row() {
    let evil = "a\x1b[31mred\x1b]0;title\x07\u{202E}gnp.exe\u{2066}x\u{2069}\r\n\tz\u{7f}\u{9b}2J";
    let mut stored = sp_profile();
    stored.check = Some(format!("old {evil}"));
    let mut proposal = sp_profile();
    proposal.check = Some(format!("new {evil}"));
    proposal.env = BTreeMap::from([(format!("N{evil}"), "1".to_string())]);
    for view in 0..5 {
        let mut app = sp_app(false);
        let s = screen_mut(&mut app);
        s.stored = shown(stored.clone(), sp_verification(), vec![]);
        s.proposal = shown(proposal.clone(), sp_verification(), vec![]);
        let mut st = status_of(true, (view == 1).then_some(ProposalState::Ready));
        if view == 2 {
            st.unparseable = Some(evil.into());
            st.unreadable_text = Some(evil.into());
            s.page = Some(ProfilePage::RawText {
                text: evil.into(),
                scroll: 0,
            });
            s.stored = shown(proposal.clone(), sp_verification(), vec![]);
            s.advanced = true;
        }
        if view == 3 {
            // A ✗ row edit's reason (the hint line) and tail (`o`), selected.
            let mut r = super::tests::record(
                ProposalState::Ready,
                proto::ProposalOrigin::Edit {
                    keys: vec!["check".into()],
                },
            );
            r.edit = Some(proto::RowEdit {
                key: "check".into(),
                value: Some(evil.into()),
                state: proto::RowEditState::Failed {
                    reason: evil.into(),
                    tail: evil.into(),
                    secs: 3,
                },
            });
            st.proposal = Some(r);
            s.selected = 1;
            s.expanded = Some("check".into());
        }
        s.status = Some(st);
        if view == 4 {
            // No status yet, and no reply will come: the no-reply text is the line.
            s.status = None;
            s.status_failed = Some(evil.into());
        }
        for (w, h) in SIZES {
            let buffer = audit::draw(&app, w, h);
            for row in audit::rows(&buffer) {
                assert_eq!(
                    crate::safe_text::tests::first_hostile(&row),
                    None,
                    "{view} {w}x{h}: {row:?}"
                );
            }
            assert_eq!(
                crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
                None,
                "{view} {w}x{h}"
            );
        }
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    let mut app = app_with_profile(|_, _| {});
    let pages = [
        None,
        Some(ProfilePage::RawText {
            text: "a = 1\n".repeat(50),
            scroll: 3,
        }),
        Some(ProfilePage::Discard),
        Some(ProfilePage::Detect {
            trust_project: false,
            unconfined_checks: false,
            focus: 0,
        }),
        Some(ProfilePage::Row {
            key: "check".into(),
            scroll: 1,
        }),
    ];
    for page in pages {
        for ready in [false, true] {
            let s = screen_mut(&mut app);
            s.page = page.clone();
            s.status = Some(status_of(true, ready.then_some(ProposalState::Ready)));
            for (w, h) in [(1, 1), (2, 2), (10, 3), (20, 5), (30, 8), (79, 23)] {
                screen_text(&app, w, h);
            }
        }
    }
}

/// Decision 5: while a page or a modal is open, the screen's border is muted; the
/// page's (or the modal's) is the one accented border.
#[test]
fn a_page_mutes_the_screens_border() {
    let mut app = app_with_profile(|_, _| {});
    let accent = role(Role::Accent, app.palette()).fg;
    let corner = |app: &App| {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|f| {
                crate::ui::draw(f, app);
            })
            .unwrap();
        terminal.backend().buffer()[(0, 0)].fg
    };
    assert_eq!(Some(corner(&app)), accent);
    screen_mut(&mut app).page = Some(ProfilePage::Discard);
    assert_ne!(Some(corner(&app)), accent);
    // Final review minor 3: a modal over the screen mutes it too.
    screen_mut(&mut app).page = None;
    app.modal = Some(crate::app::Modal::Confirm {
        message: "Stop the daemon and kill every agent?".into(),
        action: crate::app::PendingAction::StopDaemon,
    });
    assert_ne!(Some(corner(&app)), accent);
}

/// Decision 37 (principle 9): the Discard page is destructive, so its `y discard` is
/// drawn key and word in `Failed`; the raw-text page's `j/k scroll` keeps the plain
/// grammar (key in the accent).
#[test]
fn the_discard_page_is_destructive() {
    let mut app = app_with_profile(|_, _| {});
    let (failed, accent) = (
        role(Role::Failed, app.palette()).fg.expect("a colour"),
        role(Role::Accent, app.palette()).fg.expect("a colour"),
    );
    for (w, h) in SIZES {
        screen_mut(&mut app).page = Some(ProfilePage::Discard);
        let buffer = audit::draw(&app, w, h);
        let &(x, y) = audit::find(&buffer, "y discard · esc back")
            .first()
            .unwrap_or_else(|| panic!("{w}x{h}:\n{}", audit::rows(&buffer).join("\n")));
        for dx in 0..9 {
            if dx != 1 {
                assert_eq!(buffer[(x + dx, y)].fg, failed, "{w}x{h} y discard +{dx}");
            }
        }
        assert_ne!(
            buffer[(x + 12, y)].fg,
            failed,
            "{w}x{h}: esc is not destructive"
        );
        let &(tx, ty) = audit::find(&buffer, "discard proposal").first().unwrap();
        assert_eq!(buffer[(tx, ty)].fg, failed, "{w}x{h}: the title");

        screen_mut(&mut app).page = Some(ProfilePage::RawText {
            text: "check = \"cargo test\"".into(),
            scroll: 0,
        });
        let buffer = audit::draw(&app, w, h);
        let &(x, y) = audit::find(&buffer, "j/k scroll").first().unwrap();
        assert_eq!(buffer[(x, y)].fg, accent, "{w}x{h}: the page's key");
        assert_ne!(buffer[(x + 4, y)].fg, failed, "{w}x{h}: its word");
    }
}

/// Review M5 (fix round 1): a row page over a failed row edit draws its hostile value,
/// reason and output sanitised, on the profile and on the card.
#[test]
fn a_failed_row_page_is_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut app = app_with_profile(|p, _| p.check = Some(hostile.clone()));
    for state in [Some(ProposalState::Verifying), Some(ProposalState::Ready)] {
        let mut st = status_of(true, state);
        if let Some(p) = st.proposal.as_mut() {
            p.edit = Some(proto::RowEdit {
                key: "check".into(),
                value: Some(hostile.clone()),
                state: proto::RowEditState::Failed {
                    reason: hostile.clone(),
                    tail: hostile.clone(),
                    secs: 3,
                },
            });
        }
        let s = screen_mut(&mut app);
        s.status = Some(st);
        s.page = Some(ProfilePage::Row {
            key: "check".into(),
            scroll: 0,
        });
        for (w, h) in SIZES {
            assert_eq!(
                crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
                None,
                "{w}x{h}"
            );
        }
    }
}
