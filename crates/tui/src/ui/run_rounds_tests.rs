//! Milestone 9.3 task 10b: the run view of a run of two rounds (decision 32, KG §6):
//! each round's stages under its separator, round 1's rows muted, in the compact list at
//! 80×24 and on the canvas at 120×40; a round folds on Enter.

use super::tests::{MUL, app_of, key, rows_in, run_view, select};
use crate::app::App;
use crate::theme::{Role, fg};
use crate::tree::NodeKey;
use crate::tree::stage_fixtures::two_round_fixture;
use crate::ui::{audit, overview};
use crossterm::event::KeyCode;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

pub(crate) fn rounds_view(ascii: bool, w: u16, h: u16) -> App {
    let mut app = app_of(two_round_fixture(), ascii);
    let _ = app.set_terminal_size(w, h);
    run_view(app, MUL)
}

/// The canvas (the list or the graph) as drawn at `w`×`h`, and its rect.
fn canvas(app: &App, w: u16, h: u16) -> (Buffer, Rect) {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    let view = overview::view(app, layout.main);
    (audit::draw(app, w, h), view.canvas)
}

fn trimmed(buffer: &Buffer, rect: Rect) -> Vec<String> {
    let mut rows = rows_in(buffer, rect);
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// Every cell of `row` (canvas-relative) from column `from` whose symbol is not blank,
/// with its foreground.
fn inked(
    buffer: &Buffer,
    rect: Rect,
    row: u16,
    from: u16,
) -> Vec<(String, Option<ratatui::style::Color>)> {
    (rect.x + from..rect.right())
        .map(|x| &buffer[(x, rect.y + row)])
        .filter(|cell| !cell.symbol().trim().is_empty())
        .map(|cell| (cell.symbol().to_owned(), Some(cell.fg)))
        .collect()
}

fn all_muted(cells: &[(String, Option<ratatui::style::Color>)]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|(_, color)| *color == Some(fg(Role::Muted)))
}

#[test]
fn earlier_rounds_stages_sit_muted_under_their_separator() {
    // 80×24: the compact list.
    let app = rounds_view(false, 80, 24);
    let (buffer, rect) = canvas(&app, 80, 24);
    assert_eq!(
        trimmed(&buffer, rect),
        vec![
            "▌◉ run 0723  1/3",
            "  ✓ round 1 · Add mul()",
            "    ✓ stage 1/2  tier 3 ✓ 38s",
            "      ✓ t1 add mul…  merged",
            "  ◉ round 2 · also report the product",
            "    ⠋ stage 2/2  tier 3 running",
            "      ◐ t2 report_…  in review · …  after t1",
            "      ◌ t3 docs fo…  waiting        after t2",
        ]
    );
    for row in 1..=3 {
        assert!(all_muted(&inked(&buffer, rect, row, 0)), "row {row} muted");
    }
    for row in 4..=7 {
        let cells = inked(&buffer, rect, row, 0);
        assert!(
            cells
                .iter()
                .any(|(_, color)| *color != Some(fg(Role::Muted))),
            "row {row} is round 2's, not muted: {cells:?}"
        );
    }
    let app = rounds_view(true, 80, 24);
    let (buffer, rect) = canvas(&app, 80, 24);
    assert_eq!(
        trimmed(&buffer, rect)[1..5],
        [
            "  + round 1 - Add mul()",
            "    + stage 1/2  tier 3 + 38s",
            "      + t1 add m...  merged",
            "  @ round 2 - also report the product",
        ]
    );

    // 120×40: the canvas, round 1's box muted inside its muted border.
    let app = rounds_view(false, 120, 40);
    let (buffer, rect) = canvas(&app, 120, 40);
    let rows = trimmed(&buffer, rect);
    let boxes: Vec<&str> = rows.iter().map(|row| row.trim_end()).collect();
    assert_eq!(
        boxes[..9],
        [
            "                      ╭────────────────────────────╮   ╭────────────────────────────",
            "                    ┌─┤ ✓ round 1 · Add mul()      ├───┤ ✓ stage 1/2  tier 3 ✓ 38s",
            "                    │ ╰────────────────────────────╯   ╰────────────────────────────",
            "╭─────────────────╮ │",
            "▌ ◉ run 0723  1/3 ├─┤",
            "╰─────────────────╯ │",
            "                    │ ╭────────────────────────────╮   ╭────────────────────────────",
            "                    └─┤ ◉ round 2 · also report t… ├───┤ ⠋ stage 2/2  tier 3 running",
            "                      ╰────────────────────────────╯   ╰────────────────────────────",
        ]
    );
    assert!(
        all_muted(&inked(&buffer, rect, 1, 22)),
        "round 1 and its stage"
    );
    let round_two = inked(&buffer, rect, 7, 22);
    let text: Vec<_> = round_two.iter().filter(|(s, _)| s != "│").collect();
    assert!(
        text.iter()
            .any(|(_, color)| *color != Some(fg(Role::Muted))),
        "round 2's box is not muted: {text:?}"
    );
    let app = rounds_view(true, 120, 40);
    let (buffer, rect) = canvas(&app, 120, 40);
    let rows = trimmed(&buffer, rect);
    assert!(rows[1].contains("+ round 1 - Add mul()"), "{}", rows[1]);
    assert!(
        rows[7].contains("@ round 2 - also report..."),
        "{}",
        rows[7]
    );
}

#[test]
fn enter_folds_a_round() {
    let mut app = rounds_view(false, 80, 24);
    let round = NodeKey::Round {
        run: MUL.into(),
        n: 1,
    };
    select(&mut app, round.clone());
    key(&mut app, KeyCode::Enter);
    assert!(app.tree.is_collapsed(&round), "Enter folds round 1");
    // As a folded stage, no mark: its stage and task are gone.
    let (buffer, rect) = canvas(&app, 80, 24);
    assert_eq!(
        trimmed(&buffer, rect)[..3],
        [
            "◉ run 0723  1/3",
            "▌ ✓ round 1 · Add mul()",
            "  ◉ round 2 · also report the product",
        ]
    );
    key(&mut app, KeyCode::Enter);
    assert!(!app.tree.is_collapsed(&round), "Enter unfolds it");
    assert!(app.run_view.is_some(), "and the view stays open");
}
