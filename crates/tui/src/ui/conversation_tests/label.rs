//! M9.15, decision 42i: a delivered message is a user turn whose text starts with
//! `[anthrex] Message from the orchestrator (` or `[anthrex] Message from the user (`.
//! Its header says who sent it; every other user turn keeps `you`. No protocol field
//! carries the source: the label comes from the prefix alone.

use super::*;

/// The header text of the turn at `at`: the first word on the header row whose time is
/// `hh_mm`.
fn header_label(buf: &Buffer, hh_mm: &str) -> String {
    let row = all_rows(buf)
        .into_iter()
        .find(|r| r.trim_end_matches(['│', ' ']).ends_with(hh_mm))
        .unwrap_or_else(|| panic!("no header at {hh_mm}:\n{}", text_of(buf)));
    row.trim_start_matches('│')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn delivered_message_turn_is_labelled_by_source() {
    let conversation = conversation(
        None,
        3,
        vec![
            // 36_000 s is 10:00 UTC, then one turn a minute.
            turn(1, Role::User, 36_000, vec![text("fix the parser")]),
            turn(
                2,
                Role::User,
                36_060,
                vec![text(
                    "[anthrex] Message from the orchestrator (change): t2 now owns lib.rs",
                )],
            ),
            turn(
                3,
                Role::User,
                36_120,
                vec![text("[anthrex] Message from the user (info): keep going")],
            ),
            // The prefix counts only at the start of the turn's text.
            turn(
                4,
                Role::User,
                36_180,
                vec![text(
                    "quoting: [anthrex] Message from the orchestrator (info): x",
                )],
            ),
            turn(5, Role::Assistant, 36_240, vec![text("done")]),
        ],
    );
    let app = app_showing(UiSettings::default(), Runtime::Claude, 90, 30, conversation);
    let buf = draw(&app, 90, 30);
    assert_eq!(header_label(&buf, "10:00"), "you");
    assert_eq!(header_label(&buf, "10:01"), "orchestrator");
    assert_eq!(header_label(&buf, "10:02"), "user");
    assert_eq!(header_label(&buf, "10:03"), "you");
    // The message text itself is drawn unchanged.
    assert!(
        text_of(&buf).contains("[anthrex] Message from the orchestrator (change): t2 now"),
        "{}",
        text_of(&buf)
    );
}
