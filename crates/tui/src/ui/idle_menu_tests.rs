//! Milestone 9.3 task 10b: the idle orchestrator's row and menu as drawn (decision 32,
//! KG §3.2, §11), and the hostile-text rule (decision 33) on a round's goal head, a
//! chain id and a run id.

use crate::app::App;
use crate::app::idle_menu::tests::{AFTER, app_of, idle_fixture, on_idle_row, tap};
use crate::safe_text::tests::first_hostile;
use crate::theme::{Role, fg};
use crate::tree::{self, NodeKey};
use crate::ui::audit;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;

/// A visible carrier pair, a zero-width joiner and a bidi override, around `9`.
const CARRIED: &str = "3f\u{200D}9\u{202E}a";

fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (area.y..area.bottom())
        .map(|y| {
            let row: String = (area.x..area.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// The sidebar's interior on `row`, from the guides on, trimmed.
fn sidebar_row(buffer: &Buffer, app: &App, row: u16) -> String {
    let width = app.sidebar_width;
    let text: String = (1..width - 1).map(|x| buffer[(x, row)].symbol()).collect();
    text.trim_end().to_owned()
}

/// The idle fixture with `runs` runs in tree mode, the shell selected, its sidebar wide
/// enough for the row's whole text, drawn at 120×40.
fn sidebar(runs: u32, ascii: bool) -> (App, Buffer) {
    let mut app = app_of(idle_fixture(runs), ascii);
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('t'));
    let rows = tree::build_from(&app.windows, &app.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Window(1));
    app.sidebar_width = 50;
    let buffer = audit::draw(&app, 120, 40);
    (app, buffer)
}

#[test]
fn the_idle_orchestrator_row_is_muted_and_counts_runs() {
    let (app, buffer) = sidebar(1, false);
    assert_eq!(
        sidebar_row(&buffer, &app, 1),
        "▾ demo                            ○  cl 1 · sh 1"
    );
    assert_eq!(
        sidebar_row(&buffer, &app, 2),
        "├─  ◌ orchestrator · idle · after 3f9a"
    );
    assert_eq!(
        sidebar_row(&buffer, &app, 3),
        "└─▎ ○ 1 shell                             sh  0s"
    );
    // Muted whole, from its glyph to its last word.
    let text = sidebar_row(&buffer, &app, 2);
    let inked: Vec<_> = (5..1 + text.chars().count() as u16)
        .map(|x| &buffer[(x, 2)])
        .filter(|cell| !cell.symbol().trim().is_empty())
        .collect();
    assert_eq!(inked.len(), 28, "◌ and the words");
    assert!(
        inked.iter().all(|cell| cell.fg == fg(Role::Muted)),
        "{inked:?}"
    );

    let (app, buffer) = sidebar(4, false);
    assert_eq!(
        sidebar_row(&buffer, &app, 2),
        "├─  ◌ orchestrator · idle · after 3f9a · 4 runs"
    );
    let (app, buffer) = sidebar(4, true);
    assert_eq!(
        sidebar_row(&buffer, &app, 2),
        "|-  . orchestrator - idle - after 3f9a - 4 runs"
    );
    let (app, buffer) = sidebar(1, true);
    assert_eq!(
        sidebar_row(&buffer, &app, 2),
        "|-  . orchestrator - idle - after 3f9a"
    );

    // The project overview's canvas: the box, muted too.
    let mut app = on_idle_row(app_of(idle_fixture(4), false));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('T'));
    let buffer = audit::draw(&app, 120, 40);
    let (y, row) = (rows(&buffer).into_iter().enumerate())
        .find(|(_, row)| row.contains("┤ ◌ orchestrator"))
        .expect("the idle box, under the project's");
    assert!(row.contains("┤ ◌ orchestrator · idle · a… │"), "{row}");
    let x = (row.chars().enumerate())
        .filter(|(_, c)| *c == '◌')
        .last()
        .map(|(x, _)| x as u16)
        .expect("its glyph");
    assert_eq!(buffer[(x, y as u16)].fg, fg(Role::Muted));
    assert_eq!(buffer[(x + 2, y as u16)].fg, fg(Role::Muted), "its text");
}

#[test]
fn the_idle_menu_renders_at_80x24_and_120x40() {
    for (w, h, left) in [(80u16, 24u16, 8usize), (120, 40, 28)] {
        let mut app = on_idle_row(app_of(idle_fixture(1), false));
        tap(&mut app, KeyCode::Char('.'));
        let drawn = rows(&audit::draw(&app, w, h));
        let top = drawn
            .iter()
            .position(|row| row.contains("┌ orchestrator o-3f9a "))
            .expect("the menu's frame");
        let menu: Vec<String> = drawn[top..top + 6]
            .iter()
            .map(|row| row.chars().skip(left).take(64).collect())
            .collect();
        assert_eq!(
            menu,
            [
                format!("┌ orchestrator o-3f9a {}┐", "─".repeat(41)),
                format!("│ ▌ new goal here{}│", " ".repeat(46)),
                format!("│   close{}│", " ".repeat(54)),
                format!("│{}│", " ".repeat(62)),
                format!("│ ⏎ choose · j/k move · esc close{}│", " ".repeat(30)),
                format!("└{}┘", "─".repeat(62)),
            ],
            "{w}x{h}"
        );
        assert_eq!(drawn.last().map(String::as_str), Some(" MENU  esc back"));
    }
    // ASCII: the frame, the bar and the enter key fold.
    let mut app = on_idle_row(app_of(idle_fixture(1), true));
    tap(&mut app, KeyCode::Char('.'));
    let drawn = rows(&audit::draw(&app, 80, 24));
    assert!(
        drawn
            .iter()
            .any(|row| row.contains("+ orchestrator o-3f9a "))
    );
    assert!(drawn.iter().any(|row| row.contains("| > new goal here")));
    assert!(drawn.iter().all(|row| row.is_ascii()), "{drawn:#?}");
}

#[test]
fn round_and_idle_rows_are_sanitised() {
    // A round's goal head: the separator in the list and on the canvas.
    let carried_head = |w, h| {
        let mut app = crate::ui::run_list::rounds_tests::rounds_view(false, w, h);
        app.runs.runs[0].rounds[1].goal_head = "also x\u{200D}y\u{202E}z".into();
        rows(&audit::draw(&app, w, h))
    };
    let list = carried_head(80, 24);
    assert!(
        list.iter().any(|row| row.contains("◉ round 2 · also xyz")),
        "{list:#?}"
    );
    let canvas = carried_head(120, 40);
    assert!(
        canvas
            .iter()
            .any(|row| row.contains("◉ round 2 · also xyz")),
        "{canvas:#?}"
    );
    for drawn in [list, canvas] {
        assert!(
            drawn.iter().all(|row| first_hostile(row).is_none()),
            "{drawn:#?}"
        );
    }

    // A run id: the idle row's `<h4>` is the cleaned id's, never shifted.
    let (mut snap, windows) = idle_fixture(1);
    snap.idle_orchestrators[0].after_run = format!("add-reset-{CARRIED}");
    let mut app = app_of((snap, windows), false);
    app.sidebar_width = 50;
    let buffer = audit::draw(&app, 120, 40);
    assert_eq!(
        sidebar_row(&buffer, &app, 2),
        "├─  ◌ orchestrator · idle · after 3f9a"
    );

    // A chain id: the menu's title and the row's inspection.
    let (mut snap, windows) = idle_fixture(1);
    snap.idle_orchestrators[0].chain = format!("o-{CARRIED}");
    let mut app = app_of((snap, windows), false);
    let key = NodeKey::Chain(format!("o-{CARRIED}"));
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    tap(&mut app, KeyCode::Char('T'));
    let rows_now = tree::build_from(&app.windows, &app.runs, &app.tree);
    app.tree.select(&rows_now, key.clone());
    let row = rows_now
        .iter()
        .find(|row| row.key == key)
        .expect("the idle row");
    let inspection = crate::inspector::inspect(row, &app);
    assert_eq!(inspection.name, "orchestrator o-3f9a");
    let first = &inspection.fields[0];
    assert_eq!(
        (first.label, first.value.as_str()),
        ("after", format!("run {AFTER} · accepted").as_str())
    );
    let drawn = rows(&audit::draw(&app, 120, 40));
    assert!(
        drawn.iter().all(|row| first_hostile(row).is_none()),
        "{drawn:#?}"
    );
    tap(&mut app, KeyCode::Char('.'));
    let drawn = rows(&audit::draw(&app, 80, 24));
    assert!(
        drawn
            .iter()
            .any(|row| row.contains("┌ orchestrator o-3f9a ─")),
        "{drawn:#?}"
    );
    assert!(
        drawn.iter().all(|row| first_hostile(row).is_none()),
        "{drawn:#?}"
    );
}

/// Decision 32 on 9.0.7's rules: the three fixtures this task adds keep one accented
/// frame at both sizes and draw only ASCII in ASCII mode (the audit's own tests run
/// them with every other fixture; this names them).
#[test]
fn the_audit_holds_with_rounds_and_an_idle_row() {
    let names = [
        "run view with rounds",
        "sidebar idle orchestrator",
        "idle menu over the sidebar",
    ];
    let fixtures: Vec<(&str, App)> = (audit::fixtures().into_iter())
        .filter(|(name, _)| names.contains(name))
        .collect();
    assert_eq!(fixtures.len(), names.len(), "each fixture is in the audit");
    for (name, mut app) in fixtures {
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            let shown = audit::rows(&buffer).join("\n");
            assert_eq!(
                audit::accented_frames(&buffer, app.palette()),
                1,
                "{name} at {w}x{h}:\n{shown}"
            );
        }
        app.settings.badges.ascii = true;
        app.settings.badges =
            crate::ui::badge::BadgeSet::from_config(&app.settings.badges_config, true);
        for (w, h) in [(80, 24), (120, 40)] {
            let buffer = audit::draw(&app, w, h);
            assert_eq!(
                audit::first_non_ascii(&buffer),
                None,
                "{name} at {w}x{h} in ASCII"
            );
        }
    }
}

/// Fix round 1, m3: the idle inspection's run id and a round inspection's request carry
/// no hidden character.
#[test]
fn the_idle_and_round_inspections_are_sanitised() {
    let (mut snap, windows) = idle_fixture(1);
    snap.idle_orchestrators[0].after_run = format!("add-reset-{CARRIED}");
    let app = app_of((snap, windows), false);
    let rows = tree::build_from(&app.windows, &app.runs, &app.tree);
    let row = (rows.iter())
        .find(|row| matches!(row.key, NodeKey::Chain(_)))
        .expect("the idle row");
    let after = &crate::inspector::inspect(row, &app).fields[0];
    assert_eq!(
        (after.label, after.value.as_str()),
        ("after", "run add-reset-3f9a · accepted")
    );

    let mut app = crate::ui::run_list::rounds_tests::rounds_view(false, 80, 24);
    app.runs.runs[0].rounds[1].goal_head = format!("also {CARRIED}");
    let run = &app.runs.runs[0];
    let rows = tree::run_rows(run, &app.windows, &app.tree, tree::RunFilter::All);
    let key = NodeKey::Round {
        run: run.run_id.clone(),
        n: 2,
    };
    let row = rows.iter().find(|row| row.key == key).expect("round 2");
    let request = &crate::inspector::inspect(row, &app).fields[0];
    assert_eq!(
        (request.label, request.value.as_str()),
        ("request", "also 3f9a")
    );
}

/// Fix round 1, m4: the menu's title is cleaned before it is cut, so a hidden character
/// of width two (U+115F) takes no room: the title fits exactly, uncut.
#[test]
fn the_menu_title_is_cleaned_before_it_is_cut() {
    let menu = crate::app::idle_menu::IdleMenu {
        chain: "o-3f9a\u{115F}".into(),
        project: "/r/demo".into(),
        window_id: 7,
        selected: 0,
    };
    let p = crate::theme::Palette::PLAIN;
    assert_eq!(super::title(&menu, 19, p), "orchestrator o-3f9a");
}
