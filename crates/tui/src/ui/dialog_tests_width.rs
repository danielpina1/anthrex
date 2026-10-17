//! Milestone 9.18 decision 12: `wrap` measures display columns, not characters, so the
//! text it hands a dialog box fits the box a CJK path is drawn into.

use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

const CJK_PATH: &str = "/データ/作業/リポジトリ/チェックアウト";

#[test]
fn a_cjk_word_wraps_at_display_width() {
    assert!(CJK_PATH.width() > 20, "the fixture needs several lines");
    let lines = wrap(CJK_PATH, 10, 9);
    assert!(lines.len() > 1, "{lines:?}");
    for line in &lines {
        assert!(line.width() <= 10, "{line:?} is {} columns", line.width());
    }
    assert_eq!(lines.concat(), CJK_PATH);
}

#[test]
fn a_mixed_line_breaks_between_words_by_width() {
    let text = "cannot remove リポジトリ because 作業 holds work in チェックアウト now";
    let lines = wrap(text, 14, 20);
    for line in &lines {
        assert!(line.width() <= 14, "{line:?} is {} columns", line.width());
    }
    // Every word that fits whole stays whole: no line break falls inside one.
    for word in text.split_whitespace().filter(|w| w.width() <= 14) {
        assert!(
            lines
                .iter()
                .any(|line| line.split_whitespace().any(|w| w == word)),
            "{word:?} was split: {lines:?}"
        );
    }
    let strip = |s: &str| s.split_whitespace().collect::<String>();
    assert_eq!(strip(&lines.concat()), strip(text));
}

/// Decision 12: a character wider than the whole line is placed alone on its line, so
/// that line overflows by the character's own excess, and the call still ends.
#[test]
fn a_wide_character_is_placed_alone_and_ends() {
    assert_eq!(wrap("漢", 1, 3), vec!["漢".to_string()]);
    assert_eq!(wrap("漢字", 1, 5), vec!["漢".to_string(), "字".to_string()]);
}

#[test]
fn the_force_prompt_box_stays_aligned_with_a_cjk_path() {
    let message = format!(
        "worktree /Users/someone/anthrex/worktrees{CJK_PATH}/ブランチ has uncommitted \
         changes that removing it would delete"
    );
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| {
            render_force_remove(frame, "api", &message, frame.area(), Palette::PLAIN);
        })
        .unwrap();
    let buffer = terminal.backend().buffer();
    let area = buffer.area;
    // Task 10's review, m1: the whole path reaches the screen, in order, across the
    // rows it is cut into (the old wrap cut it inside a word, not losing its tail).
    let path = format!("/Users/someone/anthrex/worktrees{CJK_PATH}/ブランチ");
    let inside: String = (0..area.height)
        .map(|y| {
            let mut row = String::new();
            let mut x = 0;
            while x < area.width {
                let symbol = buffer[(x, y)].symbol();
                if !matches!(symbol, "│" | "┌" | "┐" | "└" | "┘" | "─") {
                    row.push_str(symbol);
                }
                // A wide symbol's next cell is hidden behind it.
                x += u16::try_from(symbol.width().max(1)).unwrap_or(1);
            }
            row.trim().to_string()
        })
        .collect();
    assert!(inside.contains(&path), "{path:?} not in {inside:?}");
    let mut rights = Vec::new();
    for y in 0..area.height {
        let cells: Vec<_> = (0..area.width)
            .filter(|&x| matches!(buffer[(x, y)].symbol(), "│" | "┐" | "┘"))
            .collect();
        if let Some(&right) = cells.last() {
            rights.push((y, right));
        }
    }
    assert!(rights.len() > 5, "the box was drawn: {rights:?}");
    let column = rights[0].1;
    for (y, x) in &rights {
        assert_eq!(*x, column, "row {y}'s right border drifted: {rights:?}");
    }
}

/// Task 10's review, m2: the hard cut measured each `char`, but a heart with VS16 is
/// one grapheme two columns wide, so pieces of 1-column hearts drew 6 columns.
#[test]
fn an_emoji_with_a_variation_selector_is_cut_by_its_grapheme_width() {
    let hearts = "\u{2764}\u{fe0f}".repeat(3);
    assert_eq!(hearts.width(), 6);
    for width in 1..=5 {
        let lines = wrap(&hearts, width, 10);
        assert_eq!(lines.concat(), hearts, "width {width}");
        for line in &lines {
            let alone = line.graphemes(true).count() == 1;
            assert!(
                line.width() <= width || alone,
                "width {width}: {line:?} is {} columns in {lines:?}",
                line.width()
            );
        }
    }
}
