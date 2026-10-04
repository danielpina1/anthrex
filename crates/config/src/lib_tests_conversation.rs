//! The `[conversation]` table's tests, moved out of `lib_tests.rs` (600-line rule).

use super::*;

#[test]
fn the_conversation_table_is_read() {
    let (config, problems) = parse(
        r##"
[conversation]
max_turns = 40
max_bytes = 131072
max_result_bytes = 8192
linger_secs = 7

[conversation.badges]
force_ascii = true

[conversation.badges.claude]
glyph = "▲"
ascii = "(C)"
color = "#112233"

[conversation.badges.codex]
glyph = "▼"
ascii = "(X)"
color = "#445566"

[conversation.badges.shell]
glyph = "#"
ascii = "(S)"
color = "#778899"
"##,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(config.conversation.max_turns, 40);
    assert_eq!(config.conversation.max_bytes, 131072);
    assert_eq!(config.conversation.max_result_bytes, 8192);
    assert_eq!(config.conversation.linger_secs, 7);
    assert!(config.conversation.badges.force_ascii);
    assert_eq!(config.conversation.badges.claude.glyph, "\u{25b2}");
    assert_eq!(config.conversation.badges.claude.ascii, "(C)");
    assert_eq!(
        config.conversation.badges.claude.color,
        Rgb(0x11, 0x22, 0x33)
    );
    assert_eq!(config.conversation.badges.codex.glyph, "\u{25bc}");
    assert_eq!(config.conversation.badges.codex.ascii, "(X)");
    assert_eq!(
        config.conversation.badges.codex.color,
        Rgb(0x44, 0x55, 0x66)
    );
    assert_eq!(config.conversation.badges.shell.glyph, "#");
    assert_eq!(config.conversation.badges.shell.ascii, "(S)");
    assert_eq!(
        config.conversation.badges.shell.color,
        Rgb(0x77, 0x88, 0x99)
    );
}

#[test]
fn out_of_range_conversation_values_keep_their_defaults() {
    let (config, problems) = parse(
        r#"
[conversation]
max_turns = 0
max_bytes = 1024
max_result_bytes = 2000000
linger_secs = 301
"#,
    );
    assert_eq!(config.conversation, Conversation::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.max_turns".to_string(),
            "conversation.max_bytes".to_string(),
            "conversation.max_result_bytes".to_string(),
            "conversation.linger_secs".to_string(),
        ])
    );
}

/// Each bad key lives on a *different* badge (force_ascii, claude.glyph, codex.ascii,
/// shell.color), so a `read_badge` that wrote into the wrong runtime's struct -- or a
/// `report_unknown_conversation` that conflated the three -- fails this test.
#[test]
fn bad_badge_values_keep_their_defaults() {
    let (config, problems) = parse(
        r#"
[conversation.badges]
force_ascii = "yes"

[conversation.badges.claude]
glyph = "abc"

[conversation.badges.codex]
ascii = "◇◇"

[conversation.badges.shell]
color = "blue"
"#,
    );
    assert_eq!(config.conversation.badges, Badges::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.badges.force_ascii".to_string(),
            "conversation.badges.claude.glyph".to_string(),
            "conversation.badges.codex.ascii".to_string(),
            "conversation.badges.shell.color".to_string(),
        ])
    );
    assert_eq!(problems.len(), 4, "exactly one problem per bad key");
}

#[test]
fn a_two_column_glyph_is_accepted() {
    let (config, problems) = parse("[conversation.badges.claude]\nglyph = \"\u{1f9e0}\"\n");
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    assert_eq!(config.conversation.badges.claude.glyph, "\u{1f9e0}");

    let (config, problems) = parse("[conversation.badges.claude]\nglyph = \"\"\n");
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].key, "conversation.badges.claude.glyph");
    assert_eq!(config.conversation.badges.claude.glyph, "\u{25c6}");
}

#[test]
fn a_conversation_table_of_the_wrong_type_keeps_every_default() {
    let (config, problems) = parse("conversation = 3\n");
    assert_eq!(keys(&problems), HashSet::from(["conversation".to_string()]));
    assert_eq!(problems[0].message, "expected a table");
    assert_eq!(config.conversation, Config::default().conversation);
}

#[test]
fn unknown_conversation_keys_are_reported() {
    let (config, problems) = parse(
        r##"
[conversation]
max_lines = 1

[conversation.badges]
zsh = {}

[conversation.badges.claude]
colour = "#fff"
"##,
    );
    assert_eq!(config.conversation, Conversation::default());
    assert_eq!(
        keys(&problems),
        HashSet::from([
            "conversation.max_lines".to_string(),
            "conversation.badges.zsh".to_string(),
            "conversation.badges.claude.colour".to_string(),
        ])
    );
    for problem in &problems {
        assert_eq!(problem.message, "unknown key, ignored");
    }
}

#[test]
fn conversation_range_boundaries_are_inclusive() {
    let low = r#"
[conversation]
max_turns = 1
linger_secs = 0
max_result_bytes = 4096
"#;
    let (config, problems) = parse(low);
    assert!(
        problems.is_empty(),
        "boundary-low values should be valid: {problems:?}"
    );
    assert_eq!(config.conversation.max_turns, 1);
    assert_eq!(config.conversation.linger_secs, 0);
    assert_eq!(config.conversation.max_result_bytes, 4096);

    let high = r#"
[conversation]
max_turns = 10000
linger_secs = 300
max_result_bytes = 1048576
"#;
    let (config, problems) = parse(high);
    assert!(
        problems.is_empty(),
        "boundary-high values should be valid: {problems:?}"
    );
    assert_eq!(config.conversation.max_turns, 10000);
    assert_eq!(config.conversation.linger_secs, 300);
    assert_eq!(config.conversation.max_result_bytes, 1048576);
}

#[test]
fn badges_for_runtime_picks_the_right_one() {
    let (config, problems) = parse(
        r##"
[conversation.badges.claude]
glyph = "▲"
ascii = "(C)"
color = "#112233"

[conversation.badges.codex]
glyph = "▼"
ascii = "(X)"
color = "#445566"

[conversation.badges.shell]
glyph = "#"
ascii = "(S)"
color = "#778899"
"##,
    );
    assert!(problems.is_empty(), "unexpected problems: {problems:?}");
    let badges = &config.conversation.badges;
    assert_eq!(badges.for_runtime(proto::Runtime::Claude).glyph, "\u{25b2}");
    assert_eq!(badges.for_runtime(proto::Runtime::Codex).glyph, "\u{25bc}");
    assert_eq!(badges.for_runtime(proto::Runtime::Shell).glyph, "#");
}

/// Task M6.5.10 fix round 1 (F1): a whole conversation is sent in one frame, so
/// `max_bytes` stops at `proto::MAX_FRAME` less the headroom. The last legal value is
/// accepted and the next one refused.
#[test]
fn max_bytes_cannot_exceed_what_one_frame_carries() {
    let ceiling = proto::MAX_FRAME as u64 - crate::CONVERSATION_MAX_BYTES_HEADROOM;
    assert_eq!(ceiling, 15_728_640);
    let (config, problems) = parse(&format!("[conversation]\nmax_bytes = {ceiling}\n"));
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(config.conversation.max_bytes, ceiling);

    let (config, problems) = parse(&format!("[conversation]\nmax_bytes = {}\n", ceiling + 1));
    assert_eq!(
        keys(&problems),
        HashSet::from(["conversation.max_bytes".to_string()])
    );
    assert_eq!(
        config.conversation.max_bytes,
        Conversation::default().max_bytes
    );
}
