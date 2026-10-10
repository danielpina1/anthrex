//! Milestone 9.10.9: the plain Profile screen, drawn (SP §4.4): the fixtures, then the
//! status line, the three sections, Advanced, the dimmed keys and the hint line. A row's
//! ✗, the hints and the error rows are in `profile_rows_tests.rs`, the pages and the
//! hostile text in `profile_pages_tests.rs`.

use crate::app::App;
use crate::app::profile_screen::{ProfileScreen, Shown, Side};
use crate::app::screens::Screen;
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::ui::audit;
use proto::{
    CheckProgress, CommandCheck, DeliveryMode, DeliveryProfile, DroppedCommand, ProfileSource,
    ProfileStatus, ProfileVerification, ProposalOrigin, ProposalRecord, ProposalState, RepoProfile,
};
use ratatui::buffer::Buffer;
use std::collections::BTreeMap;

pub(crate) const SIZES: [(u16, u16); 2] = [(80, 24), (120, 40)];
/// The daemon's clock in these tests.
pub(crate) const NOW: u64 = 1_000_000;
/// Verified two days (and a bit) before [`NOW`].
const VERIFIED: u64 = NOW - 2 * 86_400 - 100;

pub(crate) fn check(command: &str, code: Option<i32>, secs: u64) -> CommandCheck {
    CommandCheck {
        command: command.into(),
        ok: code == Some(0),
        code,
        timed_out: false,
        secs,
        tail: "running 3 tests\ntest result: ok. 3 passed".into(),
    }
}

/// SP §4.4's profile.
pub(crate) fn sp_profile() -> RepoProfile {
    RepoProfile {
        setup: Some("cargo fetch".into()),
        check: Some("cargo test --workspace".into()),
        single_test: Some("cargo test -p {crate} {test}".into()),
        source: vec!["crates/".into()],
        test_paths: vec!["crates/*/tests".into()],
        protected: vec![".github/".into(), "Cargo.lock".into()],
        delivery: Some(DeliveryProfile {
            mode: DeliveryMode::Pr,
            remote: "origin".into(),
        }),
        ..RepoProfile::default()
    }
}

/// Its verification: every main command passed.
pub(crate) fn sp_verification() -> ProfileVerification {
    ProfileVerification {
        at: VERIFIED,
        confined: true,
        setup: Some(check("cargo fetch", Some(0), 12)),
        check: Some(check("cargo test --workspace", Some(0), 190)),
        single_test: Some(check("cargo test -p {crate} {test}", Some(0), 4)),
        build_check: None,
        module_graph: None,
        module_test: None,
        module_tests: None,
        toolchain_id: None,
    }
}

pub(crate) fn record(state: ProposalState, origin: ProposalOrigin) -> ProposalRecord {
    ProposalRecord {
        project: "/p/shop".into(),
        state,
        origin,
        started_at: NOW - 60,
        updated_at: NOW - 60,
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
    }
}

/// A status with a stored profile (when `stored`), verified two days ago, and a review
/// proposal in `state` (when given).
pub(crate) fn status_of(stored: bool, state: Option<ProposalState>) -> ProfileStatus {
    ProfileStatus {
        project: "/p/shop".into(),
        repo_dir: "/data/shop".into(),
        source: if stored {
            ProfileSource::Stored
        } else {
            ProfileSource::None
        },
        confirmed_at: stored.then_some(VERIFIED),
        stale: vec![],
        unparseable: None,
        proposal: state.map(|s| record(s, ProposalOrigin::Detect)),
        scout: None,
        verify_confined: true,
        queued: Vec::new(),
        checking: None,
        verified_at: stored.then_some(VERIFIED),
        unreadable_text: None,
        dropped_goals: Vec::new(),
    }
}

pub(crate) fn shown(
    profile: RepoProfile,
    v: ProfileVerification,
    dropped: Vec<DroppedCommand>,
) -> Side {
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

pub(crate) fn base_app(ascii: bool) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(vec![], "/tmp".into(), settings);
    app.set_terminal_size(80, 24);
    let snap = crate::tree::run_fixtures::snapshot(NOW, vec![]);
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snap)));
    let _ = app.open_profile_on("/p/shop".into());
    app
}

/// SP §4.4's screen: the stored profile, verified two days ago, nothing proposed.
pub(crate) fn sp_app(ascii: bool) -> App {
    let mut app = base_app(ascii);
    let s = screen_mut(&mut app);
    s.status = Some(status_of(true, None));
    s.stored = shown(sp_profile(), sp_verification(), vec![]);
    s.proposal = Side::Absent("no proposal".into());
    app
}

/// The screen with a stored profile (changed by `change`) whose re-detection runs,
/// Advanced open and the `check` row's output shown (the audit's fixture too).
pub(crate) fn app_with_profile(
    change: impl FnOnce(&mut RepoProfile, &mut ProfileVerification),
) -> App {
    let mut app = base_app(false);
    let (mut p, mut v) = (sp_profile(), sp_verification());
    p.build_check = Some("cargo build".into());
    v.build_check = Some(check("cargo build", Some(101), 12));
    p.env = BTreeMap::from([("RUST_LOG".into(), "debug".into())]);
    change(&mut p, &mut v);
    if let (Some(command), Some(c)) = (&p.check, v.check.as_mut()) {
        c.command = command.clone();
    }
    let s = screen_mut(&mut app);
    let mut st = status_of(true, Some(ProposalState::Scouting));
    st.stale = vec!["Cargo.toml".into()];
    s.status = Some(st);
    s.stored = shown(p, v, vec![]);
    s.advanced = true;
    s.expanded = Some("check".into());
    app
}

/// What a frame shows, and every span the screen built (the buffer drops control
/// characters on its own, so the spans are checked too).
pub(crate) fn render_text(app: &App, w: u16, h: u16) -> String {
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

pub(crate) fn screen_text(app: &App, w: u16, h: u16) -> String {
    audit::rows(&audit::draw(app, w, h)).join("\n")
}

/// The screen's frame in `buffer`: its interior rows (between the borders, trailing
/// spaces trimmed) from the first, the interior's left column, first row and width.
pub(crate) struct Interior {
    pub rows: Vec<String>,
    pub x0: u16,
    pub y0: u16,
    pub width: u16,
}

pub(crate) fn interior(buffer: &Buffer, ascii: bool) -> Interior {
    let title = if ascii {
        "profile - shop"
    } else {
        "profile · shop"
    };
    let &(tx, ty) = audit::find(buffer, title)
        .first()
        .unwrap_or_else(|| panic!("no frame:\n{}", audit::rows(buffer).join("\n")));
    let left = tx - 2;
    let right = (left + 1..buffer.area.right())
        .find(|&x| matches!(buffer[(x, ty)].symbol(), "┐" | "+"))
        .expect("the frame's corner");
    let rows = (ty + 1..buffer.area.bottom())
        .map(|y| {
            let row: String = (left + 1..right).map(|x| buffer[(x, y)].symbol()).collect();
            row.trim_end().to_owned()
        })
        .collect();
    Interior {
        rows,
        x0: left + 1,
        y0: ty + 1,
        width: right - left - 1,
    }
}

pub(crate) fn select(app: &mut App, key: &str) {
    let s = screen_mut(app);
    s.selected = s.rows().iter().position(|r| r.key == key).unwrap();
}

/// SP §4.4: the status line on the first row, right-aligned; the three sections; the
/// folded Advanced line; each key dimmed in the right column, the cells in one column.
#[test]
fn the_screen_at_80x24_and_120x40() {
    for ascii in [false, true] {
        let app = sp_app(ascii);
        let muted = role(Role::Muted, app.palette()).fg.unwrap();
        let f = |text: &str| crate::theme::fold(text, ascii);
        let mark = if ascii { "+" } else { "✓" };
        for (w, h) in SIZES {
            let buffer = audit::draw(&app, w, h);
            let i = interior(&buffer, ascii);
            let (rows, iw) = (&i.rows, usize::from(i.width));
            let all = rows.join("\n");
            let status = f("Ready · verified 2 days ago");
            assert_eq!(rows[0], format!("{status:>iw$}"), "{w}x{h}\n{all}");
            assert_eq!(rows[1], "");
            for want in [
                " How anthrex checks your work".to_string(),
                "   check        cargo test --workspace".into(),
                "   single test  cargo test -p {crate} {test}".into(),
                " Your repo".into(),
                "   source       crates/".into(),
                "   tests        crates/*/tests".into(),
                f("   generated    —"),
                "   protected    .github/, Cargo.lock".into(),
                f(" Delivery       pull request · remote origin"),
            ] {
                assert!(
                    rows.iter().any(|r| r.starts_with(&want)),
                    "{ascii} {w}x{h}: {want:?}\n{all}"
                );
            }
            let fold_mark = if ascii { ">" } else { "▸" };
            let advanced = format!(
                "Advanced {fold_mark}     {}",
                crate::profile_words::ADVANCED_SUMMARY
            );
            let cut = &advanced[..advanced.find(", environment").unwrap()];
            let line = rows
                .iter()
                .find(|r| r.contains("Advanced"))
                .expect("Advanced");
            assert!(line.starts_with(&format!(" {cut}")), "{w}x{h}: {line}");
            if w >= 120 {
                assert_eq!(line, &format!(" {advanced}"));
            }
            // Each key in the right column, dimmed; the cells in one column.
            let advanced_at = rows.iter().position(|r| r.contains("Advanced")).unwrap();
            let mut cells = Vec::new();
            for key in [
                "setup",
                "check",
                "single_test",
                "source",
                "test_paths",
                "generated",
                "protected",
                "delivery.mode",
            ] {
                let at: Vec<(u16, u16)> = audit::find(&buffer, key)
                    .into_iter()
                    .filter(|&(x, y)| {
                        x == i.x0 + i.width - 18 && usize::from(y - i.y0) < advanced_at
                    })
                    .collect();
                assert_eq!(at.len(), 1, "{ascii} {w}x{h}: {key}\n{all}");
                let (x, y) = at[0];
                assert_eq!(buffer[(x, y)].fg, muted, "{key} is dimmed");
                if ["setup", "check", "single_test"].contains(&key) {
                    let cell = (i.x0..x).find(|&cx| buffer[(cx, y)].symbol() == mark);
                    cells.push(cell.unwrap_or_else(|| panic!("{key}: no cell\n{all}")));
                }
            }
            assert!(cells.windows(2).all(|p| p[0] == p[1]), "{cells:?}");
            if ascii {
                assert_eq!(audit::first_non_ascii(&buffer), None);
            }
        }
    }
}

/// Decision 23: each of the ten lines on the first row, in its role.
#[test]
fn every_status_line_draws() {
    let stale = |mut s: ProfileStatus| {
        s.stale = vec!["Cargo.toml".into()];
        s
    };
    let checking = |done: Option<u32>| {
        let mut s = status_of(false, Some(ProposalState::Verifying));
        s.checking = done.map(|done| CheckProgress { done, total: 4 });
        s
    };
    let mut unreadable = status_of(true, None);
    unreadable.unparseable = Some("expected `=`".into());
    let cases = [
        (
            status_of(false, None),
            "Not set up — press d to set up",
            Role::Muted,
        ),
        (
            status_of(false, Some(ProposalState::Scouting)),
            "Setting up… reading the repo",
            Role::Working,
        ),
        (
            checking(Some(2)),
            "Setting up… checking commands (2/4)",
            Role::Working,
        ),
        (
            checking(None),
            "Setting up… checking commands",
            Role::Working,
        ),
        (
            status_of(false, Some(ProposalState::Ready)),
            "Needs review — anthrex has a proposal",
            Role::Attention,
        ),
        (
            status_of(true, None),
            "Ready · verified 2 days ago",
            Role::Done,
        ),
        (
            stale(status_of(true, Some(ProposalState::Scouting))),
            "Out of date — Cargo.toml changed · re-checking",
            Role::Working,
        ),
        (
            stale(status_of(true, Some(ProposalState::Ready))),
            "Out of date — Cargo.toml changed · review the changes",
            Role::Attention,
        ),
        (
            stale(status_of(true, None)),
            "Out of date — Cargo.toml changed · press d to check again",
            Role::Attention,
        ),
        (
            unreadable,
            "Can't read the profile file — ⏎ shows it",
            Role::Failed,
        ),
    ];
    for ascii in [false, true] {
        for (status, line, r) in &cases {
            let mut app = sp_app(ascii);
            screen_mut(&mut app).status = Some(status.clone());
            let buffer = audit::draw(&app, 120, 40);
            let rows = interior(&buffer, ascii).rows;
            let line = crate::theme::fold(line, ascii);
            assert!(
                rows[0].ends_with(&line),
                "{ascii}: {line}\n{}",
                rows.join("\n")
            );
            let &(x, y) = audit::find(&buffer, &line).first().unwrap();
            assert_eq!(
                buffer[(x, y)].fg,
                role(*r, app.palette()).fg.unwrap(),
                "{line}"
            );
        }
    }
    // Too narrow, the line is cut from its left.
    let mut app = sp_app(false);
    let mut st = status_of(true, None);
    st.stale = vec!["crates/a/very/long/path/Cargo.toml".into()];
    screen_mut(&mut app).status = Some(st);
    let rows = interior(&audit::draw(&app, 40, 20), false).rows;
    assert!(
        rows[0].starts_with('…') && rows[0].ends_with("press d to check again"),
        "{}",
        rows[0]
    );
}

#[test]
fn advanced_open_draws_its_sub_heads() {
    let mut app = app_with_profile(|_, _| {});
    screen_mut(&mut app).expanded = None;
    let text = screen_text(&app, 120, 60);
    for want in [
        " Advanced ▾",
        " testing tiers",
        " output filter",
        " environment",
        " timeouts",
        " test result pattern",
        " shared code area",
        " repo details",
        " delivery remote",
        "   build check  cargo build",
        "   RUST_LOG     debug",
        "env.RUST_LOG",
        "   add a variable",
    ] {
        assert!(text.contains(want), "{want}\n{text}");
    }
    assert!(!text.contains(crate::profile_words::ADVANCED_SUMMARY));
}

#[test]
fn the_hint_line_follows_the_selection() {
    let mut app = sp_app(false);
    let muted = role(Role::Muted, app.palette()).fg.unwrap();
    for (key, hint) in [
        (
            "single_test",
            "single test — how anthrex runs one test; {test} is filled in",
        ),
        ("source", "source — where the code lives"),
        (
            "delivery.mode",
            "Delivery — how finished work reaches you: a local merge or a pull request",
        ),
    ] {
        select(&mut app, key);
        for (w, h) in SIZES {
            let buffer = audit::draw(&app, w, h);
            let rows = interior(&buffer, false).rows;
            let at = rows
                .iter()
                .position(|r| r.trim_start().starts_with(hint))
                .unwrap_or_else(|| panic!("{hint}\n{}", rows.join("\n")));
            assert_eq!(rows[at - 1], "", "a blank row above the hint");
            let &(x, y) = audit::find(&buffer, hint).first().unwrap();
            assert_eq!(buffer[(x, y)].fg, muted);
        }
    }
}
