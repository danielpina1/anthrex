//! The panel is asserted by exact rendered strings, the way the graph's
//! painter is: every test draws one `Inspection` into a real terminal buffer
//! and compares it character for character, so a border landing one column off
//! shows up as a diff rather than as a property several wrong layouts satisfy.

use super::*;
use crate::inspector::INSPECTOR_HEIGHT;
use crate::theme;
use proto::Status;
use ratatui::backend::TestBackend;
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
    // Seven fields and five rows: one column cannot show them all, so a second
    // is worth opening and two are all that is. Two columns want 31 columns of
    // interior, which 36 of panel gives them — fields fill left to right and
    // wrap, four rows deep.
    let inspection = inspection(
        "worker",
        vec![
            plain("kind", "Explore"),
            plain("model", "opus"),
            plain("state", "running"),
            plain("for", "1m"),
            plain("depth", "2"),
            plain("dir", "/r/shop"),
            plain("branch", "main"),
        ],
    );

    assert_eq!(
        panel(&inspection, 36, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────────╮",
            "│ ○ worker                         │",
            "│ kind    Explore  model  opus     │",
            "│ state   running  for    1m       │",
            "│ depth   2        dir    /r/shop  │",
            "│ branch  main                     │",
            "│                                  │",
            "╰──────────────────────────────────╯",
        ]
    );

    // Too narrow for the second column, the same fields fall back to one and
    // the two that no longer fit are dropped from the end.
    assert_eq!(
        panel(&inspection, 24, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────╮",
            "│ ○ worker             │",
            "│ kind   Explore       │",
            "│ model  opus          │",
            "│ state  running       │",
            "│ for    1m            │",
            "│ depth  2             │",
            "╰──────────────────────╯",
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

/// A sub-agent as the projection builds one: the title is the label, and the
/// `task` field is that same label again.
fn subagent(task: &str) -> Inspection {
    inspection(
        task,
        vec![
            Field {
                label: "task",
                value: task.to_owned(),
                wrap: true,
            },
            plain("spawned by", "1 api-worker"),
            plain("state", "running"),
            plain("for", "1m"),
            plain("model", "opus"),
            plain("kind", "Explore"),
            plain("depth", "1"),
        ],
    )
}

#[test]
fn the_task_field_wraps_across_the_remaining_rows() {
    // The title had to cut this label short, so the field earns its rows: it
    // leaves the column flow and takes the two the other fields did not, at the
    // panel's full width. This is the whole point of the panel — the label a
    // box could not show, read whole.
    let inspection = subagent("map every route the api exposes and note the ones without tests");

    assert_eq!(
        panel(&inspection, 44, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────────────────╮",
            "│ ○ map every route the api exposes and n… │",
            "│ spawned by  1 api-worker  state  running │",
            "│ for         1m            model  opus    │",
            "│ kind        Explore       depth  1       │",
            "│ task  map every route the api exposes    │",
            "│       and note the ones without tests    │",
            "╰──────────────────────────────────────────╯",
        ]
    );
}

#[test]
fn the_task_field_is_dropped_when_the_title_already_showed_it() {
    // The same panel with a label the title can hold whole. Printing it again a
    // row below would say the same words twice and cost the column flow the row
    // held back for it, so the field goes and the row comes back: `depth` moves
    // up into it.
    let inspection = subagent("grep handlers");

    assert_eq!(
        panel(&inspection, 44, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────────────────────────╮",
            "│ ○ grep handlers                          │",
            "│ spawned by  1 api-worker  state  running │",
            "│ for         1m            model  opus    │",
            "│ kind        Explore       depth  1       │",
            "│                                          │",
            "│                                          │",
            "╰──────────────────────────────────────────╯",
        ]
    );
    assert!(
        !panel(&inspection, 44, INSPECTOR_HEIGHT)
            .join("")
            .contains("task"),
        "the title said it; the field would only repeat it"
    );
}

#[test]
fn the_dropped_task_field_hands_its_reserved_row_back() {
    // The row held back for the wrapping field is only worth reserving while
    // the field will use it. At the width the pair above uses, seven fields in
    // two columns fit in four of the five rows either way, so keeping the row
    // reserved costs nothing visible there.
    //
    // At 24 columns they pack into one, and five rows show five fields where
    // four rows would show four: `kind` is on the panel exactly because the
    // dropped field gave its row back. Reserving a row for a field that is not
    // drawn loses `kind` off the bottom.
    let inspection = subagent("grep handlers");
    let flow_fields = inspection.fields.iter().filter(|f| !f.wrap).count();
    assert!(
        flow_fields > usize::from(INSPECTOR_HEIGHT - 3),
        "{flow_fields} flow fields must outnumber the five rows one column has"
    );

    assert_eq!(
        panel(&inspection, 24, INSPECTOR_HEIGHT),
        vec![
            "╭──────────────────────╮",
            "│ ○ grep handlers      │",
            "│ spawned by  1 api-w… │",
            "│ state       running  │",
            "│ for         1m       │",
            "│ model       opus     │",
            "│ kind        Explore  │",
            "╰──────────────────────╯",
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
    //
    // The width is chosen so the value gets its full eighteen columns rather
    // than being elided: this test is about the accounting, not the ellipsis.
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
    // panels must agree cell for cell from the end of the value onwards.
    // Counting the CJK value as nine characters would pull the trailing blanks
    // and the right border nine cells left of the ASCII one's.
    let ascii = inspection(
        "shop",
        vec![plain("dir", "abcdefghijklmnopqr"), plain("status", "idle")],
    );
    let wide_buffer = draw(&wide, 32, INSPECTOR_HEIGHT);
    let ascii_buffer = draw(&ascii, 32, INSPECTOR_HEIGHT);
    // The value fills cells 10 to 27 either way; 28 onwards is the room the
    // panel has left and the border that closes it.
    for x in 28..32 {
        assert_eq!(
            wide_buffer[(x, 2)].symbol(),
            ascii_buffer[(x, 2)].symbol(),
            "the tail of the value row at cell {x}"
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

    // The terminal would clip a panel that laid out too many rows, and the
    // buffer above would look the same either way. The layout itself has to
    // stop: six rows of interior are one title row and five field rows, and
    // nothing it produces may exceed that.
    assert_eq!(lines(&inspection, 12, 6).len(), 6);
}

#[test]
fn no_panic_at_any_size() {
    // Milestone 4.6's guard, for the panel: every size from nothing upwards,
    // with the field set most likely to break one — a value longer than any
    // panel here, and wide characters inside the *wrapping* field, which is
    // the one the panel cuts a column at a time. With the wide characters in a
    // plain field instead, this sweep walks straight past the width at which
    // `cut` used to stop advancing.
    let task = "日本語のタスク map every route the api exposes and note the ones without tests";
    let inspection = inspection(
        task,
        vec![
            Field {
                label: "task",
                value: task.into(),
                wrap: true,
            },
            plain("spawned by", "1 api-worker"),
            plain("dir", "日本語プロジェクト"),
        ],
    );
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    for width in 0..=40 {
        for height in 0..=12 {
            terminal
                .draw(|frame| render(frame, &inspection, Rect::new(0, 0, width, height)))
                .unwrap();
            // And at no size does the layout hand the terminal more rows than
            // the interior has, whether or not the terminal would clip them.
            let interior = (width.saturating_sub(4), height.saturating_sub(2));
            if interior.0 > 0 && interior.1 > 0 {
                assert!(
                    lines(
                        &inspection,
                        usize::from(interior.0),
                        usize::from(interior.1)
                    )
                    .len()
                        <= usize::from(interior.1),
                    "a {width}x{height} panel overflowed its interior"
                );
            }
        }
    }
}

#[test]
fn a_grapheme_wider_than_its_column_still_advances() {
    // `cut` used to return nothing at all when the first grapheme was wider
    // than the width it was given. `wrap_value`'s loop then re-tested an
    // unchanged string and pushed empty lines until memory ran out.
    //
    // An eleven-column panel reaches it: `task` and its two-space gap leave the
    // value a single column, and decision 6 collapses the inspector on height,
    // never on width, so that panel renders.
    assert_eq!(wrap_value("日本語のタスク", 1, 3), vec!["日", "本", "…"]);
}

#[test]
fn a_long_value_is_elided_rather_than_closing_its_column() {
    // A sub-agent whose parent's label is forty columns wide. Sizing a packing
    // by its natural widths alone refuses two columns here — forty-two columns
    // of interior cannot hold them — and falls back to one, which shows four of
    // the six fields and drops `kind` and `depth` off the bottom.
    //
    // A column that cannot say everything elides instead (decision 5), so two
    // columns are affordable and every field renders.
    let inspection = inspection(
        "grep the handlers",
        vec![
            plain("spawned by", "map every route the api exposes and no"),
            plain("state", "running"),
            plain("for", "40s"),
            plain("model", "sonnet-4-5"),
            plain("kind", "general-purpose"),
            plain("depth", "2"),
        ],
    );

    let rendered = panel(&inspection, 60, INSPECTOR_HEIGHT);
    assert_eq!(
        rendered,
        vec![
            "╭──────────────────────────────────────────────────────────╮",
            "│ ○ grep the handlers                                      │",
            "│ spawned by  map every route the api …  state  running    │",
            "│ for         40s                        model  sonnet-4-5 │",
            "│ kind        general-purpose            depth  2          │",
            "│                                                          │",
            "│                                                          │",
            "╰──────────────────────────────────────────────────────────╯",
        ]
    );
    let all = rendered.join("");
    assert!(
        all.contains("kind") && all.contains("depth"),
        "the fields past the long value must survive it: {rendered:#?}"
    );
}

#[test]
fn columns_stop_at_the_number_that_shows_every_field() {
    // Six short fields and five rows: two columns show them all, and a third
    // would show nothing more while taking room from the two. On a panel wide
    // enough for a column each, it must still open two — and elide nothing.
    let inspection = inspection(
        "worker",
        vec![
            plain("status", "working"),
            plain("for", "15m"),
            plain("model", "opus"),
            plain("runtime", "claude"),
            plain("dir", "/r/shop"),
            plain("branch", "main"),
        ],
    );

    let rendered = panel(&inspection, 76, INSPECTOR_HEIGHT);
    assert_eq!(
        rendered,
        vec![
            "╭──────────────────────────────────────────────────────────────────────────╮",
            "│ ○ worker                                                                 │",
            "│ status  working  for      15m                                            │",
            "│ model   opus     runtime  claude                                         │",
            "│ dir     /r/shop  branch   main                                           │",
            "│                                                                          │",
            "│                                                                          │",
            "╰──────────────────────────────────────────────────────────────────────────╯",
        ]
    );
    assert!(
        !rendered.join("").contains('…'),
        "nothing is elided when every field has its natural width: {rendered:#?}"
    );
}
