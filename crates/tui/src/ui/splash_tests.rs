//! The empty main pane's splash: the octopus mascot above the "no agents" hint.

use crate::app::App;
use crate::settings::UiSettings;
use crate::theme::{self, Role};
use crate::ui::badge::BadgeSet;
use ratatui::buffer::Buffer;
use ratatui::{Terminal, backend::TestBackend};

/// The terminal pane of an app with no windows, drawn `width` × `height`.
fn draw(width: u16, height: u16, ascii: bool) -> Buffer {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
    if ascii {
        app.settings.badges = BadgeSet::from_config(&app.settings.badges_config, true);
    }
    let _ = app.set_terminal_size(width, height);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|f| super::super::terminal::render(f, &app, f.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

/// The row containing `needle`, and its index.
fn row_with(rows: &[String], needle: &str) -> (usize, String) {
    rows.iter()
        .enumerate()
        .find(|(_, r)| r.contains(needle))
        .map(|(i, r)| (i, r.clone()))
        .unwrap_or_else(|| panic!("no row contains {needle:?}:\n{}", rows.join("\n")))
}

/// Blank columns left and right of a row's content, inside the pane's border.
fn margins(row: &str) -> (usize, usize) {
    let inner: Vec<char> = row.chars().skip(1).collect();
    let inner = &inner[..inner.len() - 1];
    let left = inner.iter().take_while(|c| **c == ' ').count();
    let right = inner.iter().rev().take_while(|c| **c == ' ').count();
    (left, right)
}

#[test]
fn the_empty_pane_shows_the_octopus_centred_above_the_hint() {
    let buffer = draw(60, 16, false);
    let rows = rows(&buffer);
    let (head, head_row) = row_with(&rows, "▄▄████▄▄");
    let (name, name_row) = row_with(&rows, &format!("anthrex v{}", env!("CARGO_PKG_VERSION")));
    let (hint, _) = row_with(&rows, "No agents. Press C-b c to create one");
    assert!(head < name && name < hint, "{}", rows.join("\n"));

    let (left, right) = margins(&head_row);
    assert!(left.abs_diff(right) <= 1, "head off-centre: {head_row:?}");
    let (left, right) = margins(&name_row);
    assert!(left.abs_diff(right) <= 1, "name off-centre: {name_row:?}");
    // Vertically centred too: as many blank rows above the art as below the hint.
    let above = head - 1;
    let below = rows.len() - 2 - hint;
    assert!(above.abs_diff(below) <= 1, "{}", rows.join("\n"));
}

/// Each arm is an agent: its tip wears a status colour, the body the accent.
#[test]
fn the_arm_tips_wear_the_status_colours() {
    let buffer = draw(60, 16, false);
    let rows = rows(&buffer);
    let (y, _) = row_with(&rows, "●   ●    ●   ●");
    let tips: Vec<_> = (0..buffer.area.width)
        .filter(|&x| buffer[(x, y as u16)].symbol() == "●")
        .map(|x| buffer[(x, y as u16)].fg)
        .collect();
    let want: Vec<_> = [Role::Done, Role::Working, Role::Attention, Role::Paused]
        .into_iter()
        .map(theme::fg)
        .collect();
    assert_eq!(tips, want);

    let (y, row) = row_with(&rows, "▄▄████▄▄");
    let x = row.chars().position(|c| c == '█').unwrap() as u16;
    assert_eq!(buffer[(x, y as u16)].fg, theme::fg(Role::Accent));
}

/// Too short for the art: the one-line hint, as before.
#[test]
fn a_short_pane_falls_back_to_the_hint() {
    let rows = rows(&draw(60, 6, false));
    let all = rows.join("\n");
    assert!(!all.contains('█') && !all.contains('●'), "{all}");
    row_with(&rows, "No agents. Press C-b c to create one");
}

/// Too narrow for the art: the hint alone.
#[test]
fn a_narrow_pane_falls_back_to_the_hint() {
    let all = rows(&draw(10, 16, false)).join("\n");
    assert!(!all.contains('█') && !all.contains('●'), "{all}");
}

#[test]
fn ascii_draws_an_ascii_octopus() {
    let rows = rows(&draw(60, 16, true));
    let all = rows.join("\n");
    row_with(&rows, "(  o  o  )");
    row_with(&rows, "No agents. Press C-b c to create one");
    assert!(all.is_ascii(), "{all}");
}

/// Centring aligns the art only while every row is exactly `WIDTH` columns.
#[test]
fn every_art_row_is_the_same_width() {
    use unicode_width::UnicodeWidthStr;
    let rows = super::BODY.iter().chain(&super::BODY_ASCII);
    for row in rows.chain([&super::TIPS_ROW, &super::TIPS_ROW_ASCII]) {
        assert_eq!(row.width(), usize::from(super::WIDTH), "{row:?}");
    }
}

/// Prints the splash, for eyeballing: `cargo test -p anthrex-tui splash_dump -- --ignored --nocapture`.
#[test]
#[ignore]
fn splash_dump() {
    for ascii in [false, true] {
        println!("{}", rows(&draw(64, 16, ascii)).join("\n"));
    }
}
