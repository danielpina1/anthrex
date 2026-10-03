//! Milestone 9.2 task M9.2.15: a `pr`-mode run view drawn whole, at 120×40 (the graph)
//! and 80×24 (the compact list): its stage rows' pull requests, the stage panel, and
//! the hostile-text rule on the host's text.

use crate::app::App;
use crate::safe_text::tests::first_hostile;
use crate::settings::UiSettings;
use crate::tree::pr_fixtures::{check, pr_fixture};
use crate::tree::run_fixtures::RUN_ID;
use crate::tree::{self, NodeKey};
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{CiState, DaemonMsg, RunReply, RunsSnapshot, WindowInfo};

fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// `C-b T`, the run's node selected, then `l`; then `selected` selected.
pub(crate) fn pr_view(
    (snap, windows): (RunsSnapshot, Vec<WindowInfo>),
    ascii: bool,
    selected: NodeKey,
) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(windows, "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Char('T'));
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Run(RUN_ID.into()));
    key(&mut app, KeyCode::Char('l'));
    assert!(app.run_view.is_some(), "the run view opened");
    let rows = crate::app::nav_rows_of(&app.windows, &app.runs, &app.tree, app.run_view.as_ref());
    assert!(rows.iter().any(|row| row.key == selected), "{selected:?}");
    app.tree.select(&rows, selected);
    app
}

pub(crate) fn stage_key(n: u16) -> NodeKey {
    NodeKey::Stage {
        run: RUN_ID.into(),
        n,
    }
}

fn screen(app: &App, w: u16, h: u16) -> String {
    audit::rows(&audit::draw(app, w, h)).join("\n")
}

/// The stage rows carry their pull requests in the graph (120×40) and the compact
/// list (80×24), and the stage panel its PR lines; the ASCII frame holds no other
/// character.
#[test]
fn a_pr_run_view_draws_its_prs_at_both_sizes() {
    let app = pr_view(pr_fixture(), false, stage_key(2));
    let big = screen(&app, 120, 40);
    for text in [
        "stage 1/3  tier 3 ✓ 38s  #141  merged",
        "▌ ✓ stage 2/3  tier 3 ✓ 38s  #142  ci ✗  3 threads",
        "stage 3/3  tier 3 ✓ 38s  #143  ci …",
        "pr        #142 open, based on anthrex/add-reset-3f9a/stage-1, opened 02:30",
        "ci        ✓ build · ✗ test (fix3)",
        "fix tasks ci: fix3 working; review: fix4 merged",
    ] {
        assert!(big.contains(text), "{text:?} in\n{big}");
    }
    let small = screen(&app, 80, 24);
    for text in [
        "  ✓ stage 1/3  tier 3 ✓ 38s  #141  merged",
        "▌ ✓ stage 2/3  tier 3 ✓ 38s  #142  ci ✗  3 …",
        "  ✓ stage 3/3  tier 3 ✓ 38s  #143  ci …",
        "pr        #142 open, based on anthrex/a…",
    ] {
        assert!(small.contains(text), "{text:?} in\n{small}");
    }
    let app = pr_view(pr_fixture(), true, stage_key(2));
    for (w, h) in [(120, 40), (80, 24)] {
        let buffer = audit::draw(&app, w, h);
        assert_eq!(audit::first_non_ascii(&buffer), None, "{w}x{h}");
        let text = audit::rows(&buffer).join("\n");
        assert!(text.contains("#142  ci x"), "{text}");
        assert!(text.contains("#143  ci ..."), "{text}");
    }
}

/// The cell after `needle` (every character one column), and the one after that.
fn after(buffer: &ratatui::buffer::Buffer, needle: &str) -> (ratatui::style::Color, String) {
    let chars: Vec<String> = needle.chars().map(String::from).collect();
    let area = buffer.area;
    for y in area.y..area.bottom() {
        for x in area.x..area.right().saturating_sub(chars.len() as u16) {
            let at = |dx: usize| &buffer[(x + dx as u16, y)];
            if (0..chars.len()).all(|dx| at(dx).symbol() == chars[dx]) {
                let mark = at(chars.len());
                return (mark.fg, mark.symbol().to_owned());
            }
        }
    }
    panic!("{needle:?} is not drawn");
}

/// Review finding m2: a stage row's CI mark is drawn in `theme::ci_look`'s role, in
/// the graph (120×40) and the compact list (80×24), Unicode and ASCII; the rest of the
/// row keeps the plain foreground. Status roles, never the accent.
#[test]
fn a_stage_rows_ci_mark_wears_its_role() {
    use crate::theme::{Role, role};
    for ascii in [false, true] {
        let app = pr_view(pr_fixture(), ascii, stage_key(1));
        let fg = |r| role(r, app.palette()).fg.expect("a role has a foreground");
        let (red, pending) = if ascii { ("x", ".") } else { ("✗", "…") };
        for (w, h) in [(120, 40), (80, 24)] {
            let buffer = audit::draw(&app, w, h);
            assert_eq!(after(&buffer, "#142  ci "), (fg(Role::Failed), red.into()));
            assert_eq!(
                after(&buffer, "#143  ci "),
                (fg(Role::Working), pending.into())
            );
            let (plain, _) = after(&buffer, "#142  ci ?  ".replace('?', red).as_str());
            assert_eq!(plain, ratatui::style::Color::Reset, "{w}x{h} ascii {ascii}");
            let (merged, _) = after(&buffer, "#141  ");
            assert_eq!(
                merged,
                ratatui::style::Color::Reset,
                "a merged PR has no mark"
            );
        }
    }
}

/// The hostile-text rule, drawn: a check's name, the base and the URL come from the
/// host, so the zero-width joiner, the bidi controls and the byte-order mark planted
/// in them never reach a cell, at either size.
#[test]
fn host_text_is_cleaned_where_it_is_drawn() {
    let (mut snap, windows) = pr_fixture();
    let pr = snap.runs[0].stages[1].pr.as_mut().expect("#142");
    pr.checks = vec![check(
        "bu\u{200D}ild\u{202E}",
        CiState::Red,
        Some("fi\u{2067}x3"),
    )];
    pr.base = "ma\u{FEFF}in\u{202D}".into();
    pr.url = "https://ex\u{200B}.com/\u{2066}1".into();
    let app = pr_view((snap, windows), false, stage_key(2));
    for (w, h) in [(120, 40), (80, 24)] {
        let rows = audit::rows(&audit::draw(&app, w, h));
        for row in &rows {
            assert_eq!(first_hostile(row), None, "{w}x{h}: {row:?}");
        }
        let text = rows.join("\n");
        assert!(text.contains("✗ build (fix3)"), "{w}x{h}:\n{text}");
        assert!(text.contains("based on main"), "{w}x{h}:\n{text}");
    }
}

/// Decision 36 in the run view's own header (milestone 9.0.7 decision 21's title):
/// the long form where it fits; `delivering · <age>` where it does not (review finding
/// I1), no wider than the plain form, so the name keeps its room; a local run's title
/// as before.
#[test]
fn the_run_view_title_says_delivering() {
    let top = |app: &App, w, h| audit::rows(&audit::draw(app, w, h))[0].clone();
    let app = pr_view(pr_fixture(), false, stage_key(2));
    let wide = top(&app, 120, 40);
    assert!(wide.ends_with(" 4/5 merged · delivering · 2h ╮"), "{wide}");
    let narrow = top(&app, 80, 24);
    assert!(narrow.ends_with(" delivering · 2h ╮"), "{narrow}");
    assert!(narrow.contains("run · "), "{narrow}");
    let (mut snap, windows) = pr_fixture();
    if let Some(d) = snap.runs[0].delivery.as_mut() {
        d.delivering = false;
    }
    let app = pr_view((snap, windows), false, stage_key(2));
    let wide = top(&app, 120, 40);
    assert!(wide.ends_with(" 4/5 merged · 2h ╮"), "{wide}");
}
