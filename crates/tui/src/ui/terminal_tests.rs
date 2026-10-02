//! M9.0.7.4: the terminal pane's title (decision 29).

use crate::app::App;
use crate::settings::UiSettings;
use crate::tree::run_fixtures::pty;
use crate::ui::badge::BadgeSet;
use proto::{Runtime, Status, WindowInfo};
use ratatui::{Terminal, backend::TestBackend};

fn title_of(windows: Vec<WindowInfo>, ascii: bool) -> String {
    title_at(windows, ascii, 80)
}

/// The top row of the pane drawn `width` columns wide.
fn title_at(windows: Vec<WindowInfo>, ascii: bool, width: u16) -> String {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    if ascii {
        app.settings.badges = BadgeSet::from_config(&app.settings.badges_config, true);
    }
    let _ = app.set_terminal_size(78, 10);
    let mut terminal = Terminal::new(TestBackend::new(width, 12)).unwrap();
    terminal.draw(|f| super::render(f, &app, f.area())).unwrap();
    let buffer = terminal.backend().buffer();
    (0..width).map(|x| buffer[(x, 0)].symbol()).collect()
}

/// `~/<rest>`: `shorten_home` reads the real home directory, so the fixture lives
/// under it.
fn home(rest: &str) -> std::path::PathBuf {
    dirs::home_dir().expect("a home directory").join(rest)
}

/// Decision 29: ` <tag> <name> · <dir> `, the runtime said once, as its tag; a
/// worktree window names its project and branch.
#[test]
fn the_pane_title_is_tag_name_dir() {
    let shell = pty(1, "shell-1", home("repo").to_str().unwrap(), Status::Idle);
    let title = title_of(vec![shell.clone()], false);
    assert!(title.starts_with("╭ sh shell-1 · ~/repo ─"), "{title:?}");
    assert!(!title.contains("shell ·"), "{title:?}");
    let title = title_of(vec![shell], true);
    assert!(title.starts_with("+ sh shell-1 - ~/repo -"), "{title:?}");

    let mut worktree = pty(2, "wt-form", home("repo").to_str().unwrap(), Status::Idle);
    worktree.runtime = Runtime::Codex;
    worktree.cwd = home("data/worktrees/repo-ab12/wt-form");
    worktree.branch = Some("anthrex/wt-form".into());
    let title = title_of(vec![worktree], false);
    assert!(
        title.starts_with("╭ cx wt-form · ~/repo (anthrex/wt-form, worktree) ─"),
        "{title:?}"
    );

    let title = title_of(vec![], false);
    assert!(title.starts_with("╭ no window ─"), "{title:?}");
}

/// Review fix round 1 (principle 6): at the 80×24 main pane's width (46) the title
/// keeps `<tag> <name>` and cuts the directory, then the worktree suffix, with `…`.
#[test]
fn a_long_pane_title_is_cut_with_a_mark() {
    let deep = home("repo/a/rather/long/path/to/the/package/inside");
    let shell = pty(1, "shell-1", deep.to_str().unwrap(), Status::Idle);
    let title = title_at(vec![shell.clone()], false, 46);
    assert!(title.starts_with("╭ sh shell-1 · ~/repo/a/"), "{title:?}");
    assert!(title.ends_with("… ╮"), "{title:?}");
    assert_eq!(title.chars().count(), 46, "{title:?}");
    let title = title_at(vec![shell], true, 46);
    assert!(title.ends_with("... +"), "{title:?}");
    assert!(title.is_ascii(), "{title:?}");

    let mut worktree = pty(2, "wt-form", deep.to_str().unwrap(), Status::Idle);
    worktree.runtime = Runtime::Codex;
    worktree.branch = Some("ax/wt".into());
    let title = title_at(vec![worktree.clone()], false, 46);
    assert!(title.starts_with("╭ cx wt-form · ~/"), "{title:?}");
    assert!(title.ends_with("… (ax/wt, worktree) ╮"), "{title:?}");
    // Too narrow for the suffix: it is cut too, the head kept.
    let title = title_at(vec![worktree], false, 30);
    assert!(title.starts_with("╭ cx wt-form · "), "{title:?}");
    assert!(title.ends_with("… ╮"), "{title:?}");
    assert!(!title.contains("worktree)"), "{title:?}");
}

/// Final fix wave (task 4's deferred minor): hostile and multi-byte text through the
/// title's fit. The name's bidi and zero-width characters are dropped inside the drawn
/// columns; a directory with line separators (a space each once sanitised) and wide
/// characters is measured as drawn, so the cut title still ends at the corner.
#[test]
fn a_hostile_or_wide_title_is_measured_as_drawn() {
    let dir = format!(
        "/tmp/{}日本語の長いディレクトリ/x",
        "\u{1b}\u{2028}\t".repeat(4)
    );
    let mut shell = pty(1, "x\u{200D}y\u{202E}z", "/tmp", Status::Idle);
    shell.cwd = dir.into();
    let title = title_at(vec![shell], false, 46);
    // Twelve separators drawn as twelve spaces; a wide character's second cell reads
    // as a space too, and a cut that cannot split one leaves a border cell.
    let want = format!(
        "╭ sh xyz · /tmp/{}日 本 語 の 長 い デ … ─╮",
        " ".repeat(12)
    );
    assert_eq!(title, want);
    // The fit's own text, before the frame: sanitised where it is measured, so the
    // frame's own pass changes nothing (red without `fit_title`'s `one_line`).
    let fitted = super::fit_title(
        "sh x\u{200D}y\u{202E}z",
        "/a\u{1b}b\u{2028}c",
        "",
        40,
        false,
    );
    assert_eq!(fitted, "sh xyz · /a b c");
}
