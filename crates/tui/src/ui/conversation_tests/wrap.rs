//! Task M6.5.13: prose wraps to the interior, by display width, and the view scrolls to
//! keep the selection on screen. Bounds, stated once: a `W`-column area has a `W - 2`
//! interior, and prose wraps at `W - 2 - TEXT_INDENT`, that is `W - 6`, columns.

use super::*;

/// Words of distinct lengths, so each wrap point falls somewhere different.
fn prose(chars: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.chars().count() < chars {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&format!("w{i}{}", "z".repeat(i % 7)));
        i += 1;
    }
    out.chars().take(chars).collect::<String>()
}

fn prose_app(body: &str, width: u16, height: u16) -> App {
    app_showing(
        UiSettings::default(),
        Runtime::Claude,
        width,
        height,
        conversation(
            None,
            3,
            vec![turn(90, Role::Assistant, 36_000, vec![text(body)])],
        ),
    )
}

fn text_rows(app: &App) -> Vec<String> {
    app.conversation
        .rows()
        .into_iter()
        .filter_map(|r| match r {
            Row::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

/// Every text row fits `wrap` columns, the border is intact on every interior row, and
/// the text rows drawn at the indent are, in order, exactly what `rows()` says — so the
/// cursor and search see what is drawn.
fn assert_wrapped_inside(app: &App, width: u16, height: u16, wrap: usize) -> Buffer {
    let buf = draw(app, width, height);
    let rows = text_rows(app);
    for row in &rows {
        assert!(
            row.width() <= wrap,
            "{row:?} is {} columns, over {wrap}",
            row.width()
        );
    }
    for y in 1..height - 1 {
        assert_eq!(
            buf[(width - 1, y)].symbol(),
            "│",
            "row {y} overran:\n{}",
            text_of(&buf)
        );
        assert_eq!(buf[(0, y)].symbol(), "│");
    }
    let text_x = 1 + TEXT_INDENT as u16;
    let drawn: Vec<String> = (1..height - 1)
        .map(|y| {
            (text_x..width - 1)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
        })
        .filter(|line| !line.trim().is_empty())
        .skip(1) // the turn header
        .take(rows.len())
        .map(|line| line.trim_end().to_owned())
        .collect();
    // A 2-column character's second cell reads as a space.
    let expected: Vec<String> = rows
        .iter()
        .map(|r| r.replace('界', "界 ").trim_end().to_owned())
        .collect();
    assert_eq!(drawn, expected, "\n{}", text_of(&buf));
    buf
}

#[test]
fn long_prose_wraps_to_the_interior() {
    // Area 60: interior 58, wrap width 54. 300 does not divide by 54; the next two tests
    // cover a length that does, and 2-column characters.
    let body = prose(300);
    assert_eq!(body.chars().count(), 300);
    let app = prose_app(&body, 60, 20);
    let rows = text_rows(&app);
    assert!(rows.len() > 1, "{rows:?}");
    assert_wrapped_inside(&app, 60, 20, 54);
    // Nothing is lost at a wrap: only the spaces it breaks at go.
    assert_eq!(rows.join(" "), body);
}

#[test]
fn a_word_longer_than_the_width_breaks_and_an_exact_multiple_leaves_no_empty_row() {
    // 108 = 2 × 54 columns exactly, with no space to break at: two full rows.
    let exact = "abcdefghij".repeat(11)[..108].to_owned();
    let app = prose_app(&exact, 60, 20);
    let rows = text_rows(&app);
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(rows.concat(), exact);
    assert_wrapped_inside(&app, 60, 20, 54);

    // 109 does not divide: a third row of one column.
    let over = format!("{exact}Q");
    let app = prose_app(&over, 60, 20);
    assert_eq!(
        text_rows(&app),
        vec![
            exact[..54].to_owned(),
            exact[54..].to_owned(),
            "Q".to_owned()
        ]
    );
    assert_wrapped_inside(&app, 60, 20, 54);
}

#[test]
fn wide_characters_wrap_by_display_width() {
    // Area 61: wrap width 55, an odd number, so a row of 2-column characters holds 27 of
    // them (54 columns), never 28 (56). Counting chars instead of columns would put all
    // 40 (80 columns) on one row and clip the rest.
    let body = "界".repeat(40);
    let app = prose_app(&body, 61, 20);
    let rows = text_rows(&app);
    assert_eq!(
        rows.iter().map(|r| r.chars().count()).collect::<Vec<_>>(),
        vec![27, 13]
    );
    let buf = assert_wrapped_inside(&app, 61, 20, 55);
    assert_eq!(text_of(&buf).matches('界').count(), 40, "{}", text_of(&buf));
}

#[test]
fn the_view_follows_the_selection_down_a_long_conversation() {
    let turns = (0..30)
        .map(|i| {
            turn(
                100 + i,
                Role::User,
                36_000,
                vec![text(&format!("prompt number {i:02}"))],
            )
        })
        .collect();
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        60,
        12,
        conversation(None, 9, turns),
    );
    // No cursor follows the newest row.
    let out = text_of(&draw(&app, 60, 12));
    assert!(out.contains("prompt number 29"), "{out}");
    assert!(!out.contains("prompt number 00"), "{out}");
    press(&mut app, KeyCode::Char('g'));
    let buf = draw(&app, 60, 12);
    let out = text_of(&buf);
    assert!(out.contains("prompt number 00"), "{out}");
    let (_, y) = find(&buf, "you").unwrap();
    assert!(buf[(2, y)].modifier.contains(Modifier::REVERSED), "{out}");
}
