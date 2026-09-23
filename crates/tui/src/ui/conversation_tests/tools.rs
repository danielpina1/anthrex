//! Task M6.5.13: what an unfolded tool call shows — a tinted diff for an edit, the input
//! and result for anything else, and decision A9's truncation row — and the rows that
//! say a cap bit or the transcript degraded.

use super::*;

#[test]
fn unfolding_an_edit_shows_a_tinted_diff() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        30,
        main_conversation(),
    );
    unfold(&mut app, 22, 2);
    let buf = draw(&app, 80, 30);
    let out = text_of(&buf);
    assert!(out.contains("▾ Edit  crates/parse/src/lib.rs"), "{out}");

    let removed = format!("-{OLD}");
    let (x, y) = find(&buf, &removed).unwrap_or_else(|| panic!("no removed row:\n{out}"));
    assert_eq!(buf[(x, y)].fg, Color::Red, "the - cell is not red");
    assert_eq!(buf[(x + 1, y)].fg, Color::Red);
    // The removed row ends with the old string: a transposed diff would show more here.
    let after = row_text(&buf, y)[..]
        .chars()
        .skip(x as usize + removed.chars().count())
        .collect::<String>();
    assert!(after.trim_end_matches('│').trim().is_empty(), "{after:?}");
    // A dimmed line number column sits left of the sign.
    let number = row_text(&buf, y)
        .chars()
        .take(x as usize)
        .collect::<String>();
    assert!(number.trim_start_matches('│').trim() == "1", "{number:?}");
    let digit = number.chars().position(|c| c == '1').unwrap() as u16;
    assert_eq!(buf[(digit, y)].fg, theme::DIM);

    let added = format!("+{NEW}");
    let (x, y2) = find(&buf, &added).unwrap_or_else(|| panic!("no added row:\n{out}"));
    assert_eq!(buf[(x, y2)].fg, Color::Green, "the + cell is not green");
    assert_eq!(buf[(x + 1, y2)].fg, Color::Green);
    assert_eq!(y2, y + 1, "the added line follows the removed one");
    // The result still follows the diff, muted.
    let (x, y3) = find(&buf, "edited the parser").unwrap();
    assert_eq!(y3, y2 + 1);
    assert_eq!(buf[(x, y3)].fg, theme::DIM);
    assert!(
        !out.contains("\"old_string\""),
        "the JSON was drawn instead:\n{out}"
    );
}

#[test]
fn unfolding_a_non_edit_shows_input_and_result() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        30,
        main_conversation(),
    );
    unfold(&mut app, 22, 3);
    let buf = draw(&app, 80, 30);
    let out = text_of(&buf);
    assert!(out.contains("▾ Bash  cargo test -p parse"), "{out}");
    let (x, y) = find(&buf, "cargo test -p parse_bash_cmd").unwrap_or_else(|| panic!("{out}"));
    assert_eq!(buf[(x, y)].fg, theme::DIM);
    let (x, y2) = find(&buf, "412 passed, 3 failed").unwrap_or_else(|| panic!("{out}"));
    assert!(y2 > y);
    assert_eq!(buf[(x, y2)].fg, theme::DIM);
    assert!(out.contains("failures: parse::nested"), "{out}");
    // Only the Bash call unfolded.
    assert!(!out.contains("parse_needle"), "{out}");
}

#[test]
fn the_dropped_and_degraded_rows_say_which_key_and_why() {
    let mut conv = main_conversation();
    conv.dropped_turns = 12;
    conv.dropped_by = Some(DropCause::Turns);
    conv.degraded = Some(DegradeReason::BadRecord);
    let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, conv.clone());
    let buf = draw(&app, 80, 24);
    let out = text_of(&buf);
    let dropped = "⋯ 12 earlier turns dropped (conversation.max_turns)";
    let (x, y) = find(&buf, dropped).unwrap_or_else(|| panic!("{out}"));
    assert_eq!(y, 1, "the dropped row is first");
    assert_eq!(buf[(x, y)].fg, theme::DIM);
    let degraded = "⚠ transcript partly unreadable — timeline only";
    let (x, y) = find(&buf, degraded).unwrap_or_else(|| panic!("{out}"));
    assert_eq!(
        buf[(x, y)].fg,
        theme::status_color(proto::Status::Attention)
    );
    assert!(!out.contains("max_bytes"), "{out}");

    conv.dropped_turns = 9;
    conv.dropped_by = Some(DropCause::Bytes);
    let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, conv);
    let out = text_of(&draw(&app, 80, 24));
    assert!(
        out.contains("⋯ 9 earlier turns dropped (conversation.max_bytes)"),
        "{out}"
    );
    assert!(!out.contains("max_turns"), "{out}");
}

#[test]
fn a_truncated_result_says_so_when_unfolded() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        30,
        main_conversation(),
    );
    let folded = text_of(&draw(&app, 80, 30));
    assert!(!folded.contains("truncated"), "{folded}");
    unfold(&mut app, 22, 3);

    let rows = app.conversation.rows();
    let last_detail = rows
        .iter()
        .rev()
        .find(|r| {
            matches!(
                r,
                Row::ToolDetail {
                    turn_id: 22,
                    block: 3,
                    ..
                }
            )
        })
        .unwrap();
    let Row::ToolDetail { text, kind, .. } = last_detail else {
        unreachable!()
    };
    assert_eq!(*kind, DetailKind::Truncated);
    assert_eq!(text, "truncated (conversation.max_result_bytes)");

    let buf = draw(&app, 80, 30);
    let out = text_of(&buf);
    let (x, y) = find(&buf, "⋯ truncated (conversation.max_result_bytes)")
        .unwrap_or_else(|| panic!("{out}"));
    assert_eq!(buf[(x, y)].fg, theme::DIM);
    assert!(
        row_text(&buf, y - 1).contains("failures: parse::nested"),
        "{out}"
    );
    assert!(row_text(&buf, y + 1).contains("spawned"), "{out}");
}
