//! The panel is asserted by exact rendered strings, the way the graph's
//! painter is: every test draws one `Inspection` into a real terminal buffer
//! and compares it character for character, so a border landing one column off
//! shows up as a diff rather than as a property several wrong layouts satisfy.

use super::*;
use crate::theme;
use proto::Status;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::{Terminal, layout::Rect};

fn idle_glyph() -> Span<'static> {
    Span::styled(
        theme::status_glyph(Status::Idle, 0),
        Style::default().fg(theme::status_color(Status::Idle)),
    )
}

fn inspection(name: &str, fields: Vec<Field>) -> Inspection {
    Inspection {
        glyph: idle_glyph(),
        name: name.to_owned(),
        fields,
    }
}

fn plain(label: &'static str, value: &str) -> Field {
    Field {
        label,
        value: value.to_owned(),
        wrap: false,
    }
}

fn draw(inspection: &Inspection, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| render(frame, inspection, Rect::new(0, 0, width, height)))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// One string per row of the buffer, cell by cell — what a reader watching the
/// terminal would see, with no style attached.
fn panel(inspection: &Inspection, width: u16, height: u16) -> Vec<String> {
    let buffer = draw(inspection, width, height);
    (0..height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn one_column_when_the_panel_is_narrow() {
    let inspection = inspection(
        "shop",
        vec![plain("path", "/r/shop"), plain("status", "idle")],
    );

    assert_eq!(
        panel(&inspection, 24, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────╮",
            "│ ○ shop               │",
            "│ path    /r/shop      │",
            "│ status  idle         │",
            "│                      │",
            "│                      │",
            "│                      │",
            "╰──────────────────────╯",
        ]
    );
}

#[test]
fn fields_pack_into_columns_and_wrap_to_the_next_row() {
    // Two columns are 27 columns of interior wide here and three are 42, so 28
    // is a width where exactly two fit: five fields become three rows, the
    // last of them half empty.
    let inspection = inspection(
        "worker",
        vec![
            plain("kind", "Explore"),
            plain("model", "opus"),
            plain("state", "running"),
            plain("for", "1m"),
            plain("depth", "2"),
        ],
    );

    assert_eq!(
        panel(&inspection, 32, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────╮",
            "│ ○ worker                     │",
            "│ kind   Explore  model  opus  │",
            "│ state  running  for    1m    │",
            "│ depth  2                     │",
            "│                              │",
            "│                              │",
            "╰──────────────────────────────╯",
        ]
    );
}

#[test]
fn a_long_value_is_elided_with_an_ellipsis() {
    let inspection = inspection(
        "shop",
        vec![
            plain("dir", "/very/long/path/that/does/not/fit"),
            plain("status", "idle"),
        ],
    );

    let rendered = panel(&inspection, 24, INSPECTOR_HEIGHT);
    assert_eq!(
        rendered,
        vec![
            "╭──────────────────────╮",
            "│ ○ shop               │",
            "│ dir     /very/long/… │",
            "│ status  idle         │",
            "│                      │",
            "│                      │",
            "│                      │",
            "╰──────────────────────╯",
        ]
    );
    // The elided row is the one that fills its column completely, so it is the
    // row that would push the border out if the arithmetic were off.
    for (index, row) in rendered.iter().enumerate() {
        assert_eq!(row.chars().count(), 24, "row {index}: {row:?}");
    }
}

#[test]
fn the_task_field_wraps_across_the_remaining_rows() {
    let inspection = inspection(
        "Explore",
        vec![
            plain("kind", "Explore"),
            Field {
                label: "task",
                value: "map every route the api exposes and note the ones without tests".into(),
                wrap: true,
            },
            plain("model", "opus"),
            plain("state", "running"),
            plain("for", "1m"),
            plain("spawned by", "1 api-worker"),
            plain("depth", "1"),
        ],
    );

    assert_eq!(
        panel(&inspection, 44, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────────────────╮",
            "│ ○ Explore                                │",
            "│ kind        Explore       model  opus    │",
            "│ state       running       for    1m      │",
            "│ spawned by  1 api-worker  depth  1       │",
            "│ task  map every route the api exposes    │",
            "│       and note the ones without tests    │",
            "╰──────────────────────────────────────────╯",
        ]
    );
}

#[test]
fn the_title_is_the_glyph_then_the_name_in_bold() {
    let inspection = inspection("shop", vec![plain("path", "/r/shop")]);
    let buffer = draw(&inspection, 24, INSPECTOR_HEIGHT);
    // The interior starts one column in from the border and one more from the
    // panel's own padding.
    let title_y = 1;

    let glyph = &buffer[(2, title_y)];
    assert_eq!(glyph.symbol(), theme::status_glyph(Status::Idle, 0));
    assert_eq!(glyph.fg, theme::status_color(Status::Idle));
    assert!(
        !glyph.modifier.contains(Modifier::BOLD),
        "the glyph carries the status colour, not the name's weight"
    );

    let name: String = (4..8).map(|x| buffer[(x, title_y)].symbol()).collect();
    assert_eq!(name, "shop");
    for x in 4..8 {
        assert!(
            buffer[(x, title_y)].modifier.contains(Modifier::BOLD),
            "the name is bold at column {x}"
        );
    }
    // A label below it is not, so this test cannot pass on a uniformly bold panel.
    assert!(!buffer[(2, 2)].modifier.contains(Modifier::BOLD));
}

#[test]
fn a_wide_character_value_keeps_the_border_aligned() {
    // Nine CJK characters are eighteen display columns (decision 14); counting
    // them as nine would leave the right border nine columns short of here.
    //
    // A wide grapheme owns two cells, and the buffer holds the grapheme in the
    // first and a blank in the second, so reading the row cell by cell puts a
    // space after every character. That is the terminal's own representation of
    // `日本語プロジェクト`, not an extra column: the row is still 32 cells.
    let wide = inspection(
        "shop",
        vec![plain("dir", "日本語プロジェクト"), plain("status", "idle")],
    );

    assert_eq!(
        panel(&wide, 32, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────╮",
            "│ ○ shop                       │",
            "│ dir     日 本 語 プ ロ ジ ェ ク ト    │",
            "│ status  idle                 │",
            "│                              │",
            "│                              │",
            "│                              │",
            "╰──────────────────────────────╯",
        ]
    );

    // Eighteen ASCII columns are the same eighteen display columns, so the two
    // panels must agree cell for cell everywhere but the value itself. Counting
    // the CJK value as nine characters would shift this row's trailing blanks
    // and its right border nine cells left of the ASCII one's.
    let narrow = inspection(
        "shop",
        vec![plain("dir", "abcdefghijklmnopqr"), plain("status", "idle")],
    );
    let wide_buffer = draw(&wide, 32, INSPECTOR_HEIGHT);
    let narrow_buffer = draw(&narrow, 32, INSPECTOR_HEIGHT);
    // The value fills columns 10 to 27 either way; 28 onwards is the padding
    // the panel has left and the border that closes it.
    for x in 28..32 {
        assert_eq!(
            wide_buffer[(x, 2)].symbol(),
            narrow_buffer[(x, 2)].symbol(),
            "the tail of the value row at column {x}"
        );
    }
}

#[test]
fn more_fields_than_fit_are_dropped_from_the_end() {
    const LABELS: [&str; 12] = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l"];
    let inspection = inspection(
        "node",
        LABELS
            .iter()
            .enumerate()
            .map(|(index, label)| plain(label, &(index + 1).to_string()))
            .collect(),
    );

    let rendered = panel(&inspection, 16, INSPECTOR_HEIGHT);
    assert_eq!(
        rendered,
        vec![
            "╭──────────────╮",
            "│ ○ node       │",
            "│ a  1  b  2   │",
            "│ c  3  d  4   │",
            "│ e  5  f  6   │",
            "│ g  7  h  8   │",
            "│ i  9  j  10  │",
            "╰──────────────╯",
        ],
        "ten fields fill the five rows; the last two are dropped rather than spilling"
    );
    assert_eq!(rendered.len(), usize::from(INSPECTOR_HEIGHT));
    let all = rendered.join("");
    assert!(!all.contains('k') && !all.contains('l'));
}

#[test]
fn no_panic_at_any_size() {
    // Milestone 4.6's guard, for the panel: every size from nothing upwards,
    // with the field set most likely to break one — a wrapping field, a wide
    // character and a value longer than any panel here.
    let inspection = inspection(
        "日本語プロジェクト",
        vec![
            plain("dir", "日本語プロジェクト"),
            Field {
                label: "task",
                value: "map every route the api exposes and note the ones without tests".into(),
                wrap: true,
            },
            plain("spawned by", "1 api-worker"),
        ],
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    for width in 0..=40 {
        for height in 0..=12 {
            terminal
                .draw(|frame| render(frame, &inspection, Rect::new(0, 0, width, height)))
                .unwrap();
        }
    }
}
