//! Milestone 9.0.6 task 13: the Profile screen, drawn (one page since milestone 9.10.8;
//! milestone 9.10.9 rewrites these for the plain screen).

use crate::app::App;
use crate::app::profile_screen::{ProfilePage, ProfileScreen, Shown, Side};
use crate::app::screens::Screen;
use crate::settings::UiSettings;
use proto::{
    CommandCheck, DroppedCommand, ProfileSource, ProfileStatus, ProfileVerification,
    ProposalOrigin, ProposalRecord, ProposalState, RepoProfile,
};
use ratatui::{Terminal, backend::TestBackend};
use std::collections::BTreeMap;

const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];

fn check(command: &str, code: Option<i32>, secs: u64) -> CommandCheck {
    CommandCheck {
        command: command.into(),
        ok: code == Some(0),
        code,
        timed_out: false,
        secs,
        tail: "running 3 tests\ntest result: ok. 3 passed".into(),
    }
}

fn profile() -> RepoProfile {
    RepoProfile {
        setup: Some("make setup".into()),
        check: Some("cargo test --workspace".into()),
        build_check: Some("cargo build".into()),
        source: vec!["src/**".into()],
        env: BTreeMap::from([("RUST_LOG".into(), "debug".into())]),
        ..RepoProfile::default()
    }
}

fn verification() -> ProfileVerification {
    ProfileVerification {
        at: 0,
        confined: true,
        setup: Some(check("make setup", Some(0), 2)),
        check: Some(check("cargo test --workspace", Some(0), 4)),
        single_test: None,
        build_check: Some(check("cargo build", Some(101), 12)),
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    }
}

fn status(state: Option<ProposalState>) -> ProfileStatus {
    ProfileStatus {
        project: "/p/shop".into(),
        repo_dir: "/data/shop".into(),
        source: ProfileSource::Stored,
        confirmed_at: Some(1_060),
        stale: vec!["check".into()],
        unparseable: None,
        proposal: state.map(|state| ProposalRecord {
            project: "/p/shop".into(),
            state,
            origin: ProposalOrigin::Detect,
            started_at: 3_400,
            updated_at: 3_400,
            base_sha: String::new(),
            scout_id: None,
            window_id: None,
            profile: None,
            verification: None,
            dropped: vec![],
            proposed: None,
            trusted_project: vec![],
            unconfined_checks: false,
            auto_confirm: false,
            edit: None,
        }),
        scout: None,
        verify_confined: true,
        queued: Vec::new(),
        checking: None,
        verified_at: None,
        unreadable_text: None,
        dropped_goals: Vec::new(),
    }
}

fn shown(profile: RepoProfile, v: ProfileVerification, dropped: Vec<DroppedCommand>) -> Side {
    Side::Ready(Box::new(Shown {
        toml: toml::to_string(&profile).unwrap(),
        profile: Some(profile),
        verification: Some(v),
        dropped,
    }))
}

pub(crate) fn screen_mut(app: &mut App) -> &mut ProfileScreen {
    match &mut app.screen {
        Some(Screen::Profile(s)) => s,
        _ => panic!("no profile screen"),
    }
}

fn base_app(ascii: bool) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(vec![], "/tmp".into(), settings);
    app.set_terminal_size(80, 24);
    let snap = crate::tree::run_fixtures::snapshot(3_460, vec![]);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let _ = app.open_profile_on("/p/shop".into());
    app
}

/// The screen on `/p/shop` with a stored profile and its verification (each changed by
/// `change`), and a proposal that drops a command, on the Profile tab.
pub(crate) fn app_with_profile(
    change: impl FnOnce(&mut RepoProfile, &mut ProfileVerification),
) -> App {
    let mut app = base_app(false);
    let (mut p, mut v) = (profile(), verification());
    change(&mut p, &mut v);
    // The check record is of the profile's own command, so the row carries it.
    if let (Some(command), Some(c)) = (&p.check, v.check.as_mut()) {
        c.command = command.clone();
    }
    let s = screen_mut(&mut app);
    s.status = Some(status(Some(ProposalState::Scouting)));
    s.stored = shown(p, v, vec![]);
    s.advanced = true;
    s.expanded = Some("check".into());
    app
}

/// What a frame shows, and every span the screen built (the buffer drops control
/// characters on its own, so the spans are checked too).
fn render_text(app: &App, w: u16, h: u16) -> String {
    let mut out = screen_text(app, w, h).replace('\n', " ");
    if let Some(Screen::Profile(s)) = &app.screen {
        for line in crate::ui::profile::body_lines(app, s, w) {
            for span in line.spans {
                out.push_str(&span.content);
            }
        }
        if let Some(page) = &s.page {
            for line in crate::ui::profile::page_lines(app, s, page, w) {
                for span in line.spans {
                    out.push_str(&span.content);
                }
            }
        }
    }
    out
}

fn screen_text(app: &App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal
        .draw(|f| {
            crate::ui::draw(f, app);
        })
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
        .collect()
}

#[test]
fn status_rows_render_at_80x24_and_120x40() {
    for ascii in [false, true] {
        let mut app = base_app(ascii);
        let s = screen_mut(&mut app);
        s.status = Some(status(Some(ProposalState::Scouting)));
        for (w, h) in SIZES {
            let text = screen_text(&app, w, h);
            let dot = if ascii { "-" } else { "·" };
            for want in [
                format!("profile {dot} shop"),
                "stored    confirmed 40m ago".into(),
                "stale     check".into(),
                "proposal  scout running 1m".into(),
                "d detect".into(),
                " PROFILE ".into(),
            ] {
                assert!(text.contains(&want), "{ascii} {w}x{h}: {want}\n{text}");
            }
            if ascii {
                assert!(text.is_ascii(), "{w}x{h}\n{text}");
            }
        }
        let s = screen_mut(&mut app);
        s.status = Some(status(Some(ProposalState::Failed {
            reason: "scout timed out".into(),
        })));
        let text = screen_text(&app, 120, 40);
        assert!(text.contains("proposal  failed: scout timed out"), "{text}");
        let s = screen_mut(&mut app);
        s.status = None;
        assert!(screen_text(&app, 80, 24).contains("loading"));
    }
}

#[test]
fn the_page_renders_sections_checks_and_dropped() {
    for ascii in [false, true] {
        let mut app = app_with_profile(|_, _| {});
        app.settings.badges.ascii = ascii;
        // The failed `build_check` row selected, so it is in view at 80x24.
        let s = screen_mut(&mut app);
        s.selected = s
            .rows()
            .iter()
            .position(|r| r.key == "build_check")
            .unwrap();
        for (w, h) in SIZES {
            let text = screen_text(&app, w, h);
            let (pass, fail) = if ascii {
                ("+ 0 - 4s", "x 101 - 12s")
            } else {
                ("✓ 0 · 4s", "✗ 101 · 12s")
            };
            for want in [
                "How anthrex checks your work",
                "testing tiers",
                "cargo test --workspace",
                pass,
                fail,
                "test result: ok. 3 passed",
            ] {
                assert!(text.contains(want), "{ascii} {w}x{h}: {want}\n{text}");
            }
            if ascii {
                assert!(text.is_ascii(), "{w}x{h}\n{text}");
            }
        }
        // The full height shows every section and the environment's rows.
        screen_mut(&mut app).expanded = None;
        let text = screen_text(&app, 120, 60);
        for want in [
            "Your repo",
            "Delivery",
            "environment",
            "env.RUST_LOG",
            "debug",
        ] {
            assert!(text.contains(want), "{want}\n{text}");
        }
        // The changes card: its marks and its dropped commands with their reasons.
        let mut proposal = profile();
        proposal.check = Some("cargo test".into());
        proposal.setup = None;
        let dropped = vec![DroppedCommand {
            key: "single_test".into(),
            command: "cargo test {test}".into(),
            reason: "exit 101 after 3s".into(),
            tail: String::new(),
        }];
        let s = screen_mut(&mut app);
        s.proposal = shown(proposal, verification(), dropped);
        s.status = Some(status(Some(ProposalState::Ready)));
        s.expanded = None;
        s.selected = s.rows().len() - 1;
        let text = screen_text(&app, 120, 60);
        let (removed, changed) = if ascii { ("-", "~") } else { ("−", "~") };
        for want in [
            "anthrex found changes in how to work in this repo".to_string(),
            format!("{changed} check"),
            format!("{removed} setup"),
            "dropped".into(),
            "single_test".into(),
            "exit 101 after 3s".into(),
        ] {
            assert!(text.contains(&want), "{ascii}: {want}\n{text}");
        }
    }
}

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

#[test]
fn an_error_row_shows_the_daemons_text() {
    let mut app = app_with_profile(|_, _| {});
    screen_mut(&mut app).error = Some("run r1 is live in /p/shop; edit later".into());
    for (w, h) in SIZES {
        assert!(screen_text(&app, w, h).contains("run r1 is live in /p/shop"));
    }
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
    });
    for (w, h) in [(80, 24), (120, 40)] {
        assert_eq!(
            crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
            None
        );
    }
}

/// The same for the status rows, the proposal view, the error row and every page.
#[test]
fn profile_screen_pages_are_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    let mut app = app_with_profile(|p, _| {
        p.env.insert(hostile.clone(), hostile.clone());
        p.source = vec![hostile.clone()];
    });
    let mut p = profile();
    p.check = Some(hostile.clone());
    let dropped = vec![DroppedCommand {
        key: hostile.clone(),
        command: hostile.clone(),
        reason: hostile.clone(),
        tail: hostile.clone(),
    }];
    let s = screen_mut(&mut app);
    s.proposal = shown(p, verification(), dropped);
    s.error = Some(hostile.clone());
    s.message = Some(hostile.clone());
    let mut st = status(Some(ProposalState::Failed {
        reason: hostile.clone(),
    }));
    st.stale = vec![hostile.clone()];
    st.unparseable = Some(hostile.clone());
    s.status = Some(st);
    let pages = [
        None,
        Some(ProfilePage::RawText {
            text: hostile.clone(),
            scroll: 0,
        }),
        Some(ProfilePage::Unset {
            key: hostile.clone(),
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
    ];
    // The profile, then the card.
    for card in [false, true] {
        if card {
            let mut st = status(Some(ProposalState::Ready));
            st.queued = vec![];
            screen_mut(&mut app).status = Some(st);
        }
        for page in &pages {
            let s = screen_mut(&mut app);
            s.page = page.clone();
            for (w, h) in SIZES {
                assert_eq!(
                    crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
                    None,
                    "{card} {page:?} {w}x{h}"
                );
            }
        }
    }
    // An editor over a hostile value.
    let s = screen_mut(&mut app);
    s.page = None;
    s.status = Some(status(None));
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
            s.status = Some(status(ready.then_some(ProposalState::Ready)));
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
    use crate::theme::{Role, role};
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
    // Final review minor 3: a modal over the screen (a late `ConfirmNeeded`'s menu can
    // open over any screen) mutes it too.
    screen_mut(&mut app).page = None;
    app.modal = Some(crate::app::Modal::Confirm {
        message: "Stop the daemon and kill every agent?".into(),
        action: crate::app::PendingAction::StopDaemon,
    });
    assert_ne!(Some(corner(&app)), accent);
}

/// Principle 6: a refusal longer than the footer's three lines is marked cut.
#[test]
fn a_cut_refusal_is_marked() {
    let mut app = app_with_profile(|_, _| {});
    screen_mut(&mut app).error = Some("word ".repeat(200));
    let text = screen_text(&app, 80, 24);
    assert!(text.contains("word …"), "{text}");
    screen_mut(&mut app).error = Some("short refusal".into());
    assert!(!screen_text(&app, 80, 24).contains('…'));
}

/// Decision 37 (principle 9): the Discard page is destructive, so its `y discard` is
/// drawn key and word in `Failed`, as the action menu's and the Settings discard page's
/// are; the raw-text page's `j/k scroll` keeps the plain grammar (key in the accent).
#[test]
fn the_discard_page_is_destructive() {
    use crate::theme::{Role, role};
    use crate::ui::audit;
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
