//! Milestone 9.0.7 final fix wave I1: decision 31's rule for the action menu, extended
//! to every modal. A modal takes every key (`App::on_key`), so the bar names no key of
//! the mode underneath: ` HELP ` over the help, ` MENU ` over the action menu,
//! ` DIALOG ` over every other dialog, and `esc` alone.

use crate::app::{App, Modal, region::KeyRegion};
use crate::ui::audit::fixtures;
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

/// The status bar alone, `w` columns wide, trailing spaces trimmed: the full-height
/// help covers the bar's row at 80x24, so the whole frame would hide what it says.
fn bar(app: &App, w: u16, _h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, 1)).expect("a test terminal");
    terminal
        .draw(|f| super::render(f, app, Rect::new(0, 0, w, 1)))
        .expect("a frame");
    let buffer = terminal.backend().buffer();
    let row: String = (0..w).map(|x| buffer[(x, 0)].symbol()).collect();
    row.trim_end().to_string()
}

/// The exact bar each Dialog-region audit fixture draws.
fn expected(name: &str) -> &'static str {
    match name {
        "help over the sidebar tree"
        | "profile under the help"
        | "help over the run view"
        | "help over the alerts view" => " HELP  esc close",
        "action menu over the run view" | "action menu on its message form" => " MENU  esc back",
        "config notice over the pane" => " DIALOG  esc close",
        "confirm over the overview"
        | "new agent over the pane"
        | "remove over the pane"
        | "force remove over the pane"
        | "rename over the pane"
        | "edit form over the run view"
        | "goal form over the pane" => " DIALOG  esc back",
        other => panic!("the Dialog-region fixture {other:?} has no expected bar"),
    }
}

fn dialogs() -> Vec<(&'static str, App)> {
    fixtures()
        .into_iter()
        .filter(|(_, app)| app.key_region() == KeyRegion::Dialog)
        .collect()
}

/// I1: over every Dialog-region fixture the bar wears the modal's badge and names
/// `esc` alone, at both audit sizes.
#[test]
fn every_modal_bar_names_only_esc() {
    let all = dialogs();
    assert!(all.len() >= 14, "{} dialog fixtures", all.len());
    for (name, app) in all {
        for (w, h) in [(80, 24), (120, 40)] {
            assert_eq!(bar(&app, w, h), expected(name), "{name} at {w}x{h}");
        }
    }
}

/// I1 with the sidebar hidden: the flag keeps its count, but not `<p> a`, which the
/// modal would swallow (M1).
#[test]
fn a_modal_hides_the_flags_key() {
    for (name, mut app) in dialogs() {
        app.sidebar_visible = false;
        let text = bar(&app, 120, 40);
        let badge = expected(name).split("  ").next().unwrap();
        let count = crate::app::alerts(&app).len();
        let esc = expected(name).rsplit("  ").next().unwrap();
        let want = if count > 0 {
            format!("{badge}  ⚑ {count}  {esc}")
        } else {
            expected(name).to_string()
        };
        assert_eq!(text, want, "{name}");
        assert!(app.modal.is_some(), "{name}");
    }
}

/// I1: a prefix left pending as a modal opens (the daemon's force-removal follow-up
/// arrives while `C-b` waits) still shows the modal's bar: the modal takes the next key.
#[test]
fn a_modal_outranks_a_pending_prefix() {
    let mut app = fixtures()
        .into_iter()
        .find(|(n, _)| *n == "pane")
        .unwrap()
        .1;
    super::kit_tests::prefix(&mut app);
    assert!(super::kit_tests::bar(&app, 120, 40).starts_with(" PREFIX "));
    app.modal = Some(Modal::ForceRemove {
        window_id: app.windows[0].id,
        name: app.windows[0].name.clone(),
        message: "dirty".into(),
    });
    assert_eq!(bar(&app, 120, 40), " DIALOG  esc back");
}

fn fixture(name: &str) -> App {
    fixtures()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}

/// M1: with the sidebar hidden the flag names `<p> a` only where `C-b a` opens the
/// Alerts view; under the open view, a screen or the plan review it keeps the count
/// alone.
#[test]
fn the_flag_names_its_key_only_where_it_works() {
    let flag = |name: &str| {
        let mut app = fixture(name);
        app.sidebar_visible = false;
        let count = crate::app::alerts(&app).len();
        assert!(count > 0, "{name} has an alert");
        (bar(&app, 160, 40), count)
    };
    for name in [
        "pane",
        "run view at the gate",
        "conversation",
        "project overview",
    ] {
        let (text, n) = flag(name);
        assert!(text.contains(&format!("⚑ {n} C-b a  ")), "{name}: {text:?}");
    }
    for name in ["alerts view", "plan review", "stats"] {
        let (text, n) = flag(name);
        assert!(text.contains(&format!("⚑ {n}  ")), "{name}: {text:?}");
        assert!(!text.contains("C-b a"), "{name}: {text:?}");
    }
}

/// M1: `PgDn panel` only while a task is selected and its panel is shown (`i` hides
/// it; PgDn does nothing on a run, a stage or with the panel off).
#[test]
fn page_down_is_hinted_only_over_a_shown_task_panel() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = fixture("run view running");
    app.tree.selected = Some(crate::tree::NodeKey::Run(
        crate::tree::run_fixtures::RUN_ID.into(),
    ));
    assert!(
        !bar(&app, 200, 40).contains("PgDn"),
        "{}",
        bar(&app, 200, 40)
    );
    let mut app = fixture("run view on a task");
    let text = bar(&app, 200, 40);
    assert!(text.contains("PgDn panel"), "{text:?}");
    app.on_key(KeyEvent::new(KeyCode::Char('i'), KeyModifiers::NONE));
    assert!(!app.inspector_visible, "`i` hid the panel");
    assert!(
        !bar(&app, 200, 40).contains("PgDn"),
        "{}",
        bar(&app, 200, 40)
    );
    // A main pane too short for a panel draws the single line.
    app.inspector_visible = true;
    app.graph_main = Some(Rect::new(34, 0, 86, 12));
    assert!(
        !bar(&app, 200, 40).contains("PgDn"),
        "{}",
        bar(&app, 200, 40)
    );
}
