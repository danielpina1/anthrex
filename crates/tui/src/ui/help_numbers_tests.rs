//! M9.19.3: the numbers glossary, its doc, the help's `numbers` group and the README link.

use super::*;
use crate::app::Modal;
use crate::settings::UiSettings;
use crate::ui::audit;
use common::glossary::GLOSSARY;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const DOC: &str = include_str!("../../../../docs/numbers.md");
const README: &str = include_str!("../../../../README.md");

/// The glossary table's data rows as their four cells. A row is a line that starts with
/// `| ` and ends with ` |`; those two are removed and the rest is split on ` | `. The
/// table has no escaped pipe (`\|`), and the test fails if a cell ever holds one, so
/// this split cannot silently move a cell.
fn doc_rows() -> Vec<[String; 4]> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in DOC.lines() {
        if line.starts_with("| Term |") {
            in_table = true;
            continue;
        }
        if !in_table || line.starts_with("|---") {
            continue;
        }
        if !line.starts_with("| ") {
            break;
        }
        assert!(!line.contains("\\|"), "escaped pipe in {line:?}");
        let inner = line
            .strip_prefix("| ")
            .and_then(|l| l.strip_suffix(" |"))
            .unwrap_or_else(|| panic!("not a table row: {line:?}"));
        let cells: Vec<&str> = inner.split(" | ").collect();
        let cells: [&str; 4] = cells
            .try_into()
            .unwrap_or_else(|c: Vec<&str>| panic!("{} cells in {line:?}", c.len()));
        rows.push(cells.map(str::to_string));
    }
    rows
}

#[test]
fn the_glossary_doc_and_glossary_agree() {
    let rows = doc_rows();
    let terms: Vec<&str> = rows.iter().map(|r| r[0].as_str()).collect();
    let want: Vec<&str> = GLOSSARY.iter().map(|t| t.term).collect();
    assert_eq!(terms, want, "terms, in order");
    for (row, term) in rows.iter().zip(GLOSSARY) {
        assert_eq!(row[1].replace('`', ""), term.shown, "shown: {}", term.term);
        assert_eq!(
            row[3].replace(" (not yet unified)", ""),
            term.definition,
            "definition: {}",
            term.term
        );
    }
}

fn numbers_group() -> HelpGroup {
    help_groups("C-b")
        .into_iter()
        .find(|g| g.name == "numbers")
        .expect("a numbers group")
}

#[test]
fn help_has_a_numbers_group_of_every_term() {
    let group = numbers_group();
    let keys: Vec<&str> = group.rows.iter().map(|(k, _)| k.as_str()).collect();
    let want: Vec<&str> = GLOSSARY.iter().map(|t| t.term).collect();
    assert_eq!(keys, want);
    for ((_, words), term) in group.rows.iter().zip(GLOSSARY) {
        assert_eq!(*words, format!("{} — {}", term.shown, term.definition));
    }
    let names: Vec<&str> = help_groups("C-b").iter().map(|g| g.name).collect();
    assert_eq!(names.last(), Some(&"numbers"));
}

#[test]
fn the_readme_links_the_glossary() {
    let section = README
        .split("\n## ")
        .find(|s| s.starts_with("Reading the numbers"))
        .expect("a Reading the numbers heading");
    let section = section.split("\n## ").next().unwrap();
    assert!(section.contains("](docs/numbers.md)"), "{section}");
}

fn app_at(scroll: u16) -> App {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    app.modal = Some(Modal::Help(HelpView {
        scroll,
        ..HelpView::default()
    }));
    app.set_body_area(ratatui::layout::Rect::new(0, 0, 80, 23));
    app
}

#[test]
fn the_help_box_still_fits() {
    // Every scroll position renders, the last one ends on the last glossary row cut to
    // the box, and nothing is drawn wider than the box.
    let groups = help_groups("C-b");
    let key_w = numbers_group()
        .rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap();
    let room = 60 - (INDENT + key_w + GAP);
    let last = GLOSSARY.last().unwrap();
    let words = format!("{} — {}", last.shown, last.definition);
    let cut = kit::cut(&words, room, "…");
    let want = format!("  {:<key_w$}  {cut}", last.term);
    let mut app = app_at(0);
    let mut seen_last = false;
    for _ in 0..=line_count(&groups) {
        let rows = audit::rows(&audit::draw(&app, 80, 24));
        seen_last |= rows.iter().any(|r| r.contains(&want));
        let before = app.modal.clone();
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if app.modal == before {
            break;
        }
    }
    assert!(seen_last, "{want:?}");
}

/// In ASCII mode no glossary glyph (`✓`, `⚑`, `×`, `÷`) leaks through at any scroll.
#[test]
fn the_numbers_group_is_ascii_in_ascii_mode() {
    let mut app = app_at(0);
    app.settings.badges.ascii = true;
    let top = line_count(&help_groups("C-b"));
    for scroll in 0..=top as u16 {
        app.modal = Some(Modal::Help(HelpView {
            scroll,
            ..HelpView::default()
        }));
        let buffer = audit::draw(&app, 80, 24);
        assert_eq!(audit::first_non_ascii(&buffer), None, "scroll {scroll}");
    }
}

#[test]
fn every_numbers_row_is_reachable_at_80x24_cut_to_the_box() {
    let group = numbers_group();
    let key_w = group
        .rows
        .iter()
        .map(|(k, _)| k.chars().count())
        .max()
        .unwrap();
    let room = 60 - (INDENT + key_w + GAP);
    let mut app = app_at(0);
    let mut seen: Vec<String> = Vec::new();
    loop {
        seen.extend(audit::rows(&audit::draw(&app, 80, 24)));
        let before = app.modal.clone();
        app.on_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        if app.modal == before {
            break;
        }
    }
    assert!(seen.iter().any(|r| r.contains("│ numbers")));
    for (key, words) in &group.rows {
        let row = format!("  {key:<key_w$}  {}", kit::cut(words, room, "…"));
        assert!(seen.iter().any(|r| r.contains(&row)), "{row:?}");
    }
}
