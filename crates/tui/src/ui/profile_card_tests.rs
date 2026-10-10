//! Milestone 9.10.9: the review card, drawn (SP §4.2, decision 28).

use super::tests::{
    SIZES, check, interior, record, render_text, screen_mut, shown, sp_app, sp_profile,
    sp_verification, status_of,
};
use crate::app::App;
use crate::app::profile_screen::Side;
use crate::theme::{Role, role};
use crate::ui::audit;
use proto::{DroppedCommand, ProposalOrigin, ProposalState, RepoProfile};

/// One row as decision 28 and the brief lay it out: the bar, two spaces, the label in
/// 13 columns, the value, the cell (11 columns) and the key (18 columns) at the right.
fn row(sel: &str, label: &str, value: &str, cell: &str, key: &str, iw: usize) -> String {
    let room = iw - 16 - 13 - 20;
    format!("{sel}  {label:<13}{value:<room$}  {cell:<11}  {key}")
}

/// SP §4.2's fresh card: no stored profile, a ready detection.
fn fresh_app(ascii: bool) -> App {
    let mut app = sp_app(ascii);
    let s = screen_mut(&mut app);
    s.status = Some(status_of(false, Some(ProposalState::Ready)));
    s.stored = Side::Absent("no stored profile for /p/shop".into());
    let dropped = vec![DroppedCommand {
        key: "build_check".into(),
        command: "cargo clippy".into(),
        reason: "timed out after 600s".into(),
        tail: String::new(),
    }];
    s.proposal = shown(sp_profile(), sp_verification(), dropped);
    s.selected = 0;
    app
}

#[test]
fn the_fresh_card_at_80x24_and_120x40() {
    for ascii in [false, true] {
        let app = fresh_app(ascii);
        let f = |text: &str| crate::theme::fold(text, ascii);
        let (sel, pass) = if ascii { (">", "+") } else { ("▌", "✓") };
        let attention = role(Role::Attention, app.palette()).fg.unwrap();
        for (w, h) in SIZES {
            let buffer = audit::draw(&app, w, h);
            let i = interior(&buffer, ascii);
            let iw = usize::from(i.width);
            let status = f("Needs review — anthrex has a proposal");
            let mark = if ascii { ">" } else { "▸" };
            let advanced = format!(
                "Advanced {mark}     {}",
                crate::profile_words::ADVANCED_SUMMARY
            );
            // Too narrow for the summary beside its head: whole, on the next line.
            let advanced = if 1 + advanced.chars().count() > iw {
                vec![
                    format!(" Advanced {mark}"),
                    format!("   {}", crate::profile_words::ADVANCED_SUMMARY),
                ]
            } else {
                vec![format!(" {advanced}")]
            };
            let mut want = vec![
                format!("{status:>iw$}"),
                String::new(),
                " anthrex learned how to work in this repo".into(),
                String::new(),
                " How anthrex checks your work".into(),
                row(
                    sel,
                    "setup",
                    "cargo fetch",
                    &format!("{pass} 12s"),
                    "setup",
                    iw,
                ),
                row(
                    " ",
                    "check",
                    "cargo test --workspace",
                    &format!("{pass} 3m10s"),
                    "check",
                    iw,
                ),
                row(
                    " ",
                    "single test",
                    "cargo test -p {crate} {test}",
                    &format!("{pass} 4s"),
                    "single_test",
                    iw,
                ),
                " Your repo".into(),
                row(" ", "source", "crates/", "", "source", iw),
                row(" ", "tests", "crates/*/tests", "", "test_paths", iw),
                row(
                    " ",
                    "protected",
                    ".github/, Cargo.lock",
                    "",
                    "protected",
                    iw,
                ),
                format!(
                    " {:<15}{:<room$}  {:<11}  delivery.mode",
                    "Delivery",
                    f("pull request · remote origin"),
                    "",
                    room = iw - 49
                ),
            ];
            want.extend(advanced);
            want.extend([
                f(" couldn't verify: build check (cargo clippy) — timed out after 600s"),
                String::new(),
                f("   setup — how anthrex prepares a fresh checkout before any check"),
            ]);
            assert_eq!(
                &i.rows[..want.len()],
                &want[..],
                "{ascii} {w}x{h}\n{}",
                i.rows.join("\n")
            );
            let &(x, y) = audit::find(&buffer, &f("couldn't verify")).first().unwrap();
            assert_eq!(buffer[(x, y)].fg, attention);
            let title = audit::find(&buffer, "anthrex learned")[0];
            assert!(
                buffer[title]
                    .modifier
                    .contains(ratatui::style::Modifier::BOLD)
            );
            if ascii {
                assert_eq!(audit::first_non_ascii(&buffer), None);
            }
        }
    }
}

/// Decision 28: with a stored profile, only the changed rows, each `<old> → <new>`
/// (`—` for none), whatever their section; nothing folded.
#[test]
fn the_changes_card_shows_old_and_new() {
    for ascii in [false, true] {
        let f = |text: &str| crate::theme::fold(text, ascii);
        let mut app = sp_app(ascii);
        let mut proposal: RepoProfile = sp_profile();
        proposal.check = Some("cargo nextest run".into());
        proposal.protected = vec![];
        proposal.build_check = Some("cargo build".into());
        let mut v = sp_verification();
        v.check = Some(check("cargo nextest run", Some(0), 70));
        let s = screen_mut(&mut app);
        let mut st = status_of(true, Some(ProposalState::Ready));
        st.stale = vec!["Cargo.toml".into()];
        st.proposal = Some(record(
            ProposalState::Ready,
            ProposalOrigin::Auto {
                stale: vec!["Cargo.toml".into()],
            },
        ));
        s.status = Some(st);
        s.proposal = shown(proposal, v, vec![]);
        let text = audit::rows(&audit::draw(&app, 120, 40)).join("\n");
        for want in [
            f("Out of date — Cargo.toml changed · review the changes"),
            " anthrex found changes in how to work in this repo".into(),
            " How anthrex checks your work".into(),
            f("  check        cargo test --workspace → cargo nextest run"),
            " Your repo".into(),
            f("   protected    .github/, Cargo.lock → —"),
            " testing tiers".into(),
            f("   build check  — → cargo build"),
        ] {
            assert!(text.contains(&want), "{ascii}: {want}\n{text}");
        }
        let pass = if ascii { "+ 1m10s" } else { "✓ 1m10s" };
        assert!(text.contains(pass), "{text}");
        assert!(
            !text.contains("cargo fetch"),
            "an unchanged row is not listed"
        );
        assert!(!text.contains("Advanced"), "nothing folded");
        let text = audit::rows(&audit::draw(&app, 80, 24)).join("\n");
        assert!(
            text.contains(&f("  check        cargo test --workspace →")),
            "{text}"
        );
    }
}

/// A hostile command, output tail, dropped key, command and reason, and old value:
/// nothing hostile is drawn.
#[test]
fn card_text_is_sanitised() {
    let hostile = crate::safe_text::tests::hostile_text();
    for stored in [false, true] {
        let mut app = fresh_app(false);
        let mut p = sp_profile();
        p.check = Some(hostile.clone());
        p.env.insert(hostile.clone(), hostile.clone());
        let mut v = sp_verification();
        v.check = Some(check(&hostile, Some(0), 3));
        v.check.as_mut().unwrap().tail = hostile.clone();
        let dropped = vec![DroppedCommand {
            key: hostile.clone(),
            command: hostile.clone(),
            reason: hostile.clone(),
            tail: hostile.clone(),
        }];
        let s = screen_mut(&mut app);
        s.proposal = shown(p, v, dropped);
        s.expanded = Some("check".into());
        s.advanced = true;
        if stored {
            let mut old = sp_profile();
            old.check = Some(format!("old {hostile}"));
            s.stored = shown(old, sp_verification(), vec![]);
            s.status = Some(status_of(true, Some(ProposalState::Ready)));
        }
        for (w, h) in SIZES {
            assert_eq!(
                crate::safe_text::tests::first_hostile(&render_text(&app, w, h)),
                None,
                "{stored} {w}x{h}"
            );
        }
    }
}

/// Final review C-I1: on the card a ✗ row edit draws the value that failed, and,
/// whatever row is selected, says plainly that **Use this** leaves it out.
#[test]
fn a_failed_proposal_edit_is_named_before_use_this() {
    for ascii in [false, true] {
        let mut app = fresh_app(ascii);
        let s = screen_mut(&mut app);
        let mut st = status_of(false, Some(ProposalState::Ready));
        if let Some(p) = st.proposal.as_mut() {
            p.edit = Some(proto::RowEdit {
                key: "check".into(),
                value: Some("\"cargo nextest run\"".into()),
                state: proto::RowEditState::Failed {
                    reason: "exit 101 after 3s".into(),
                    tail: String::new(),
                    secs: 3,
                },
            });
        }
        s.status = Some(st);
        s.selected = 0;
        let f = |text: &str| crate::theme::fold(text, ascii);
        let text = crate::ui::audit::rows(&audit::draw(&app, 120, 40)).join("\n");
        assert!(
            text.contains(&f("cargo test --workspace → cargo nextest run")),
            "{text}"
        );
        let notice = "your edit of check (cargo nextest run) failed its check; ⏎ uses this proposal without it";
        assert!(text.contains(&f(notice)), "{text}");
        // At 80 columns the notice wraps, and still shows.
        let text = render_text(&app, 80, 24);
        assert!(text.contains(&f("failed its check;")), "{text}");
    }
}

/// Final review M6: over an unreadable stored profile the card's Enter is **Use this**,
/// which replaces the file; the status line says that, not `⏎ shows it`.
#[test]
fn the_card_over_an_unreadable_file_says_enter_replaces_it() {
    let mut app = fresh_app(false);
    let s = screen_mut(&mut app);
    let mut st = status_of(true, Some(ProposalState::Ready));
    st.unparseable = Some("expected `]`".into());
    s.status = Some(st);
    assert!(s.showing_card());
    let text = crate::ui::audit::rows(&audit::draw(&app, 120, 40)).join("\n");
    assert!(
        text.contains("Needs review — the profile file can't be read; ⏎ replaces it"),
        "{text}"
    );
    assert!(!text.contains("⏎ shows it"), "{text}");
}
