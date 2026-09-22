use crate::transcript::tests::{
    CARGO_TOML, CLAUDE_FIXTURE, PROMPT, assert_carries_no_turn_boundary, first_line, parse_all,
    parse_lines,
};
use crate::transcript::{Cursor, Record, Version, parser_for};
use serde_json::json;

const RUNTIME: proto::Runtime = proto::Runtime::Claude;
const SESSION: &str = "00000000-0000-4000-8000-000000000002";

fn parser() -> &'static dyn crate::transcript::TranscriptParser {
    parser_for(RUNTIME).unwrap()
}

fn user_text(ordinal: u32, text: &str) -> Record {
    Record::UserText {
        session_id: Some(SESSION.into()),
        ordinal,
        text: text.into(),
    }
}

fn assistant_text(ordinal: u32, text: &str) -> Record {
    Record::AssistantText {
        session_id: Some(SESSION.into()),
        ordinal,
        text: text.into(),
    }
}

/// A real prompt, shaped like the fixture's line 8.
fn prompt_line(text: &str) -> String {
    json!({"type":"user","sessionId":SESSION,"isSidechain":false,
        "origin":{"kind":"human"},"promptSource":"typed","turnOrigin":"human",
        "message":{"role":"user","content":text}})
    .to_string()
}

/// An assistant text block, shaped like the fixture's line 31.
fn reply_line(text: &str) -> String {
    json!({"type":"assistant","sessionId":SESSION,"isSidechain":false,
        "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
    .to_string()
}

#[test]
fn the_golden_fixture_is_detected() {
    assert_eq!(
        parser().detect(first_line(CLAUDE_FIXTURE)),
        Some(Version(1))
    );
}

#[test]
fn the_golden_fixture_yields_the_records_it_contains() {
    let expected = vec![
        user_text(0, PROMPT),
        assistant_text(0, "Starting the check."),
        Record::ToolDetail {
            tool_use_id: "toolu_fixture_1".into(),
            input: Some(json!({"command": "cat /repo/Cargo.toml",
                "description": "Read workspace Cargo.toml"})),
            detail: None,
            ok: None,
        },
        Record::ToolDetail {
            tool_use_id: "toolu_fixture_2".into(),
            input: Some(json!({"command": "cat /repo/.gitignore",
                "description": "Read gitignore"})),
            detail: None,
            ok: None,
        },
        Record::ToolDetail {
            tool_use_id: "toolu_fixture_1".into(),
            input: None,
            detail: Some(CARGO_TOML.into()),
            ok: Some(true),
        },
        Record::ToolDetail {
            tool_use_id: "toolu_fixture_2".into(),
            input: None,
            detail: Some("target/\n.superpowers/\n.DS_Store\n.worktrees/\n__pycache__/".into()),
            ok: Some(true),
        },
        assistant_text(0, "ready"),
    ];
    let records = parse_all(RUNTIME, CLAUDE_FIXTURE);
    assert_eq!(records, expected);

    // Named fields, not only structure: the two replies differ from each other and
    // from the prompt, and the two tools differ in id, input and detail.
    let texts: Vec<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Starting the check.", "ready"]);
    assert!(!texts.contains(&PROMPT));
}

#[test]
fn an_unrecognised_first_line_is_not_detected() {
    for line in ["{}", "not json", "", r#"{"type":"something-else"}"#] {
        assert_eq!(parser().detect(line), None, "detected {line:?}");
    }
}

#[test]
fn a_line_of_the_wrong_shape_is_none_not_an_error() {
    let mut cursor = Cursor::default();
    assert!(
        parser()
            .record(Version(1), r#"{"type":"assistant"}"#, &mut cursor)
            .is_empty()
    );
    assert!(parser().record(Version(1), "{{{", &mut cursor).is_empty());
    for line in [
        "",
        "null",
        "[]",
        r#"{"type":"user"}"#,
        r#"{"type":"user","message":{"content":7}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"text"}]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","input":{}}]}}"#,
        r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#,
    ] {
        assert!(
            parser().record(Version(1), line, &mut cursor).is_empty(),
            "{line}"
        );
    }
    assert_eq!(cursor, Cursor::default());
}

#[test]
fn a_record_never_carries_a_turn_boundary() {
    let records = parse_all(RUNTIME, CLAUDE_FIXTURE);
    assert!(!records.is_empty());
    records.iter().for_each(assert_carries_no_turn_boundary);
}

#[test]
fn a_line_with_two_tool_results_yields_two_records() {
    let line = json!({"type":"user","sessionId":SESSION,"message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"toolu_a","content":"first out","is_error":false},
        {"type":"tool_result","tool_use_id":"toolu_b","content":"second out","is_error":true},
    ]}})
    .to_string();
    assert_eq!(
        parse_lines(RUNTIME, &[&line]),
        vec![
            Record::ToolDetail {
                tool_use_id: "toolu_a".into(),
                input: None,
                detail: Some("first out".into()),
                ok: Some(true),
            },
            Record::ToolDetail {
                tool_use_id: "toolu_b".into(),
                input: None,
                detail: Some("second out".into()),
                ok: Some(false),
            },
        ]
    );
}

#[test]
fn one_assistant_line_may_carry_prose_and_a_call() {
    let first = prompt_line("go");
    let line = json!({"type":"assistant","sessionId":SESSION,"message":{"role":"assistant",
        "content":[{"type":"text","text":"looking"},
                   {"type":"tool_use","id":"toolu_c","name":"Read","input":{"file_path":"a"}}]}})
    .to_string();
    assert_eq!(
        parse_lines(RUNTIME, &[&first, &line]),
        vec![
            user_text(0, "go"),
            assistant_text(0, "looking"),
            Record::ToolDetail {
                tool_use_id: "toolu_c".into(),
                input: Some(json!({"file_path":"a"})),
                detail: None,
                ok: None,
            },
        ]
    );
}

#[test]
fn each_real_prompt_opens_the_next_ordinal() {
    let lines = [
        prompt_line("one"),
        reply_line("first reply"),
        prompt_line("two"),
        reply_line("second reply"),
        prompt_line("three"),
        reply_line("third reply"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_lines(RUNTIME, &lines),
        vec![
            user_text(0, "one"),
            assistant_text(0, "first reply"),
            user_text(1, "two"),
            assistant_text(1, "second reply"),
            user_text(2, "three"),
            assistant_text(2, "third reply"),
        ]
    );
}

#[test]
fn a_string_user_line_without_the_human_origin_is_not_a_prompt() {
    let not_typed = json!({"type":"user","sessionId":SESSION,
        "message":{"role":"user","content":"injected"}})
    .to_string();
    let lines = [
        prompt_line("real"),
        not_typed,
        reply_line("answer"),
        prompt_line("second real"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_lines(RUNTIME, &lines),
        vec![
            user_text(0, "real"),
            assistant_text(0, "answer"),
            user_text(1, "second real"),
        ]
    );
}

#[test]
fn prose_before_any_prompt_has_no_turn_to_land_on() {
    let reply = reply_line("orphan");
    assert!(parse_lines(RUNTIME, &[&reply]).is_empty());
}

#[test]
fn a_sidechain_line_is_not_this_conversation() {
    let side = json!({"type":"user","sessionId":SESSION,"isSidechain":true,
        "origin":{"kind":"human"},"message":{"role":"user","content":"sub prompt"}})
    .to_string();
    let side_reply = json!({"type":"assistant","sessionId":SESSION,"isSidechain":true,
        "message":{"role":"assistant","content":[{"type":"text","text":"sub reply"}]}})
    .to_string();
    let first = prompt_line("main");
    assert_eq!(
        parse_lines(RUNTIME, &[&first, &side, &side_reply]),
        vec![user_text(0, "main")]
    );
}

#[test]
fn a_result_whose_content_is_a_block_list_joins_its_text() {
    let line = json!({"type":"user","sessionId":SESSION,"message":{"role":"user","content":[
        {"type":"tool_result","tool_use_id":"toolu_d","content":[
            {"type":"text","text":"part one"},{"type":"image"},{"type":"text","text":"part two"}]}
    ]}})
    .to_string();
    assert_eq!(
        parse_lines(RUNTIME, &[&line]),
        vec![Record::ToolDetail {
            tool_use_id: "toolu_d".into(),
            input: None,
            detail: Some("part one\npart two".into()),
            ok: None,
        }]
    );
}

#[test]
fn detect_accepts_any_claude_envelope_but_not_a_bare_type() {
    // The discriminator is the top-level camelCase `sessionId` beside a string `type`.
    assert_eq!(
        parser().detect(r#"{"type":"a-type-the-capture-does-not-show","sessionId":"s"}"#),
        Some(Version(1))
    );
    assert_eq!(parser().detect(r#"{"type":"last-prompt"}"#), None);
    assert_eq!(parser().detect(r#"{"sessionId":"s"}"#), None);
    assert_eq!(parser().detect(r#"{"type":1,"sessionId":"s"}"#), None);
    assert_eq!(
        parser().detect(r#"{"type":"x","sessionId":"s","payload":{}}"#),
        None
    );
    // Line 6 of the fixture, a file-history-snapshot, is the one Claude record type the
    // capture shows without a `sessionId`; a file may start with it.
    let snapshot = CLAUDE_FIXTURE.lines().nth(5).unwrap();
    assert_eq!(parser().detect(snapshot), Some(Version(1)));
}

/// Review F2: a human-origin prompt whose content is a block list (a prompt with a pasted
/// image, say) still opens the next turn, so every later ordinal stays right.
#[test]
fn a_human_prompt_whose_content_is_a_list_still_opens_a_turn() {
    let listed = json!({"type":"user","sessionId":SESSION,"origin":{"kind":"human"},
        "message":{"role":"user","content":[
            {"type":"text","text":"two (with image)"},
            {"type":"image","source":{"type":"base64","data":"x"}}]}})
    .to_string();
    let image_only = json!({"type":"user","sessionId":SESSION,"origin":{"kind":"human"},
        "message":{"role":"user","content":[
            {"type":"image","source":{"type":"base64","data":"y"}}]}})
    .to_string();
    let lines = [
        prompt_line("one"),
        reply_line("r1"),
        listed,
        reply_line("r2"),
        prompt_line("three"),
        reply_line("r3"),
        image_only,
        reply_line("r4"),
        prompt_line("five"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_lines(RUNTIME, &lines),
        vec![
            user_text(0, "one"),
            assistant_text(0, "r1"),
            user_text(1, "two (with image)"),
            assistant_text(1, "r2"),
            user_text(2, "three"),
            assistant_text(2, "r3"),
            // The image-only prompt opens turn 3 but has no text to offer.
            assistant_text(3, "r4"),
            user_text(4, "five"),
        ]
    );
}

/// Review F3: a meta or compact-summary line is something Claude wrote, even when it
/// carries the human origin, and never opens a turn.
#[test]
fn meta_and_compact_summary_lines_are_never_prompts() {
    let meta = json!({"type":"user","sessionId":SESSION,"isMeta":true,
        "origin":{"kind":"human"},
        "message":{"role":"user","content":"<local-command-caveat>Caveat</local-command-caveat>"}})
    .to_string();
    let summary = json!({"type":"user","sessionId":SESSION,"isCompactSummary":true,
        "origin":{"kind":"human"},
        "message":{"role":"user","content":"This session is being continued"}})
    .to_string();
    let lines = [
        prompt_line("one"),
        reply_line("r1"),
        meta,
        summary,
        prompt_line("two"),
        reply_line("r2"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_lines(RUNTIME, &lines),
        vec![
            user_text(0, "one"),
            assistant_text(0, "r1"),
            user_text(1, "two"),
            assistant_text(1, "r2"),
        ]
    );
}

/// Review F10: the id is what joins a call to the timeline, so a call without an
/// `input` still yields its `ToolDetail`.
#[test]
fn a_tool_use_without_input_keeps_its_id() {
    let line = json!({"type":"assistant","sessionId":SESSION,"message":{"role":"assistant",
        "content":[{"type":"tool_use","id":"toolu_e","name":"N"}]}})
    .to_string();
    assert_eq!(
        parse_lines(RUNTIME, &[&line]),
        vec![Record::ToolDetail {
            tool_use_id: "toolu_e".into(),
            input: None,
            detail: None,
            ok: None,
        }]
    );
}
