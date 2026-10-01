//! M9.0.7.4, decision 30: the title without ` rev N `, a user turn led by the sequence
//! separator `›`, and no `0.0s` duration.

use super::*;

#[test]
fn the_title_has_no_rev() {
    let mut app = app_showing(
        UiSettings::default(),
        Runtime::Claude,
        80,
        24,
        main_conversation(),
    );
    let title = row_text(&draw(&app, 80, 24), 0);
    assert!(title.contains("orchestrator · claude-opus-5"), "{title}");
    assert!(!title.contains("rev"), "{title}");
    assert!(!title.contains("214"), "{title}");
    descend_twice(&mut app);
    let title = row_text(&draw(&app, 80, 24), 0);
    assert!(title.contains("orchestrator › Explore › Review"), "{title}");
    assert!(!title.contains("rev"), "{title}");
}

#[test]
fn a_user_turn_leads_with_the_separator() {
    let conv = conversation(
        None,
        3,
        vec![turn(1, Role::User, 36_000, vec![text("fix the bug")])],
    );
    let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, conv.clone());
    let out = text_of(&draw(&app, 80, 24));
    assert!(out.contains("› fix the bug"), "{out}");
    assert!(!out.contains("▸ fix the bug"), "{out}");
    let header = out.lines().find(|l| l.contains("10:00")).unwrap();
    assert_eq!(
        header.trim_start_matches('│').split_whitespace().next(),
        Some("you"),
        "{header}"
    );
    let app = app_showing(ascii_settings(), Runtime::Claude, 80, 24, conv);
    let out = text_of(&draw(&app, 80, 24));
    assert!(out.contains("> fix the bug"), "{out}");
}

#[test]
fn zero_durations_are_hidden() {
    let call = |ms| {
        conversation(
            None,
            3,
            vec![turn(
                1,
                Role::Assistant,
                36_000,
                vec![tool(
                    "Read",
                    "src/lib.rs",
                    json!({"file_path": "src/lib.rs"}),
                    ToolState::Ok,
                    Some(ms),
                    None,
                )],
            )],
        )
    };
    let read_row = |ms| {
        let app = app_showing(UiSettings::default(), Runtime::Claude, 80, 24, call(ms));
        let buf = draw(&app, 80, 24);
        all_rows(&buf)
            .into_iter()
            .find(|r| r.contains("Read"))
            .unwrap_or_else(|| panic!("no Read row:\n{}", text_of(&buf)))
    };
    let fast = read_row(20);
    assert!(!fast.contains("0.0s"), "{fast}");
    assert!(fast.trim_end_matches(['│', ' ']).ends_with('✓'), "{fast}");
    assert!(!read_row(49).contains("0.0s"));
    assert!(read_row(50).contains("✓ 0.1s"));
    let slow = read_row(1200);
    assert!(slow.contains("✓ 1.2s"), "{slow}");
}
