//! M9.0.7.4: the terminal pane's title (decision 29).

use crate::app::App;
use crate::settings::UiSettings;
use crate::tree::run_fixtures::pty;
use crate::ui::badge::BadgeSet;
use proto::{Runtime, Status, WindowInfo};
use ratatui::{Terminal, backend::TestBackend};

fn title_of(windows: Vec<WindowInfo>, ascii: bool) -> String {
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    if ascii {
        app.settings.badges = BadgeSet::from_config(&app.settings.badges_config, true);
    }
    let _ = app.set_terminal_size(78, 10);
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
    terminal.draw(|f| super::render(f, &app, f.area())).unwrap();
    let buffer = terminal.backend().buffer();
    (0..80).map(|x| buffer[(x, 0)].symbol()).collect()
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
