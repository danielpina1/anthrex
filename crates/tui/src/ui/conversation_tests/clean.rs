//! Review M4: text from the agent is cleaned before it is wrapped or drawn. Tabs expand
//! to `TAB_WIDTH` (4) spaces, an ANSI/CSI or OSC escape is removed whole (not just its
//! ESC byte), and any other control character is dropped — in prose, tool summaries and
//! detail lines alike. Every input is distinct, so each rule is checked on its own.

use super::*;

/// The drawn text of the first interior row containing `marker`, from the first
/// non-space cell to the last one before the border.
fn drawn_row(buf: &Buffer, marker: &str) -> String {
    let row = all_rows(buf)
        .into_iter()
        .find(|r| r.contains(marker))
        .unwrap_or_else(|| panic!("no row with {marker:?}:\n{}", text_of(buf)));
    row.trim_start_matches('│')
        .trim_end_matches('│')
        .trim()
        .to_owned()
}

fn noisy_conversation() -> Conversation {
    conversation(
        None,
        8,
        vec![turn(
            70,
            Role::Assistant,
            36_000,
            vec![
                text("tab\there"),
                text("\x1b[31mred\x1b[0m"),
                text("bell\x07"),
                text("title\x1b]0;pwned\x07end"),
                tool(
                    "Bash",
                    "sum\tmary \x1b[1mbold\x1b[22m",
                    json!({"command": "echo\tcol"}),
                    ToolState::Ok,
                    Some(700),
                    result("out\x1b[2Kput\u{9b}1m", Some("line\ttwo\x08!"), false),
                ),
            ],
        )],
    )
}

#[test]
fn prose_is_cleaned_before_it_is_wrapped_and_drawn() {
    let app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        noisy_conversation(),
    );
    let buf = draw(&app, 80, 24);
    assert_eq!(drawn_row(&buf, "tab"), "tab    here");
    assert_eq!(drawn_row(&buf, "red"), "red");
    assert_eq!(drawn_row(&buf, "bell"), "bell");
    assert_eq!(drawn_row(&buf, "title"), "titleend");
    let out = text_of(&buf);
    assert!(!out.contains("[31m") && !out.contains("[0m"), "{out}");
    assert!(!out.contains("pwned"), "{out}");
    // `rows()` holds the cleaned text, so the cursor and search see what is drawn.
    let texts: Vec<String> = app
        .conversation
        .rows()
        .into_iter()
        .filter_map(|r| match r {
            Row::Text { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["tab    here", "red", "bell", "titleend"]);
}

#[test]
fn a_lone_control_character_leaves_an_empty_row() {
    let app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        60,
        10,
        conversation(
            None,
            8,
            vec![turn(71, Role::Assistant, 36_000, vec![text("\x07")])],
        ),
    );
    let rows = app.conversation.rows();
    assert_eq!(
        rows[1],
        Row::Text {
            turn_id: 71,
            block: 0,
            line: 0,
            text: String::new(),
        }
    );
    let buf = draw(&app, 60, 10);
    assert_eq!(row_text(&buf, 2).trim_matches(['│', ' ']), "");
}

#[test]
fn summaries_and_detail_lines_are_cleaned_too() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        noisy_conversation(),
    );
    let folded = draw(&app, 80, 24);
    let tool = drawn_row(&folded, "Bash");
    assert!(tool.starts_with("▸ Bash  sum    mary bold"), "{tool:?}");
    app.conversation.set_cursor(Cursor::Block(70, 4));
    press(&mut app, KeyCode::Enter);
    let buf = draw(&app, 80, 24);
    // The input is shown as JSON, which already escapes a tab as the two characters `\t`.
    assert_eq!(drawn_row(&buf, "echo"), r#""command": "echo\tcol""#);
    assert_eq!(drawn_row(&buf, "out"), "output");
    assert_eq!(drawn_row(&buf, "line"), "line    two!");
}
