use crate::transcript::tests::{
    CARGO_TOML, CODEX_FIXTURE, PROMPT, assert_carries_no_turn_boundary, first_line, parse_all,
    parse_lines,
};
use crate::transcript::{Cursor, Record, Version, parser_for};
use serde_json::{Value, json};

const RUNTIME: proto::Runtime = proto::Runtime::Codex;
/// `session_meta.payload.id`: the thread id, the same value Codex's `notify` reports as
/// `thread-id` and its `event_msg` lines carry as `thread_id`.
const SESSION: &str = "00000000-0000-4000-8000-000000000001";

fn parser() -> &'static dyn crate::transcript::TranscriptParser {
    parser_for(RUNTIME).unwrap()
}

fn session_line() -> &'static str {
    first_line(CODEX_FIXTURE)
}

fn user_text(ordinal: u32, text: &str) -> Record {
    Record::UserText {
        session_id: Some(SESSION.into()),
        ordinal,
        text: text.into(),
        human: true,
    }
}

fn assistant_text(ordinal: u32, text: &str) -> Record {
    Record::AssistantText {
        session_id: Some(SESSION.into()),
        ordinal,
        text: text.into(),
    }
}

fn response_item(payload: Value) -> String {
    json!({"timestamp":"t","type":"response_item","payload":payload}).to_string()
}

/// A real prompt, shaped like the fixture's line 9.
fn prompt_line(text: &str) -> String {
    response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_text","text":text}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}))
}

/// An assistant message, shaped like the fixture's line 12.
fn reply_line(text: &str) -> String {
    response_item(json!({"type":"message","role":"assistant",
        "content":[{"type":"output_text","text":text}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["unknown"]}}))
}

fn output_line(call_id: &str, texts: &[&str]) -> String {
    let output: Vec<Value> = texts
        .iter()
        .map(|t| json!({"type":"input_text","text":t}))
        .collect();
    response_item(json!({"type":"custom_tool_call_output","call_id":call_id,"output":output}))
}

#[test]
fn the_golden_fixture_is_detected() {
    assert_eq!(parser().detect(session_line()), Some(Version(1)));
}

#[test]
fn the_golden_fixture_yields_the_records_it_contains() {
    // Only the `response_item` stream: the `event_msg`/`item_completed` copies of the
    // same prompt, replies and commands must not appear a second time.
    let expected = vec![
        user_text(0, PROMPT),
        assistant_text(0, "Starting the check.\n"),
        Record::ToolDetail {
            tool_use_id: "call_fixture_1".into(),
            input: Some(Value::String(
                "text(await tools.exec_command({cmd:\"cat Cargo.toml\",max_output_tokens:4000}));\n"
                    .into(),
            )),
            detail: None,
            ok: None,
        },
        Record::ToolDetail {
            tool_use_id: "call_fixture_1".into(),
            input: None,
            detail: Some(format!("{CARGO_TOML}\n")),
            ok: Some(true),
        },
        Record::ToolDetail {
            tool_use_id: "call_fixture_2".into(),
            input: Some(Value::String(
                "text(await tools.exec_command({cmd:\"cat .gitignore\",max_output_tokens:4000}));\n"
                    .into(),
            )),
            detail: None,
            ok: None,
        },
        Record::ToolDetail {
            tool_use_id: "call_fixture_2".into(),
            input: None,
            detail: Some("target/\n.superpowers/\n.DS_Store\n.worktrees/\n__pycache__/\n".into()),
            ok: Some(true),
        },
        assistant_text(0, "ready"),
    ];
    let records = parse_all(RUNTIME, CODEX_FIXTURE);
    assert_eq!(records, expected);

    let texts: Vec<&str> = records
        .iter()
        .filter_map(|r| match r {
            Record::AssistantText { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, ["Starting the check.\n", "ready"]);
    let prompts = records
        .iter()
        .filter(|r| matches!(r, Record::UserText { .. }))
        .count();
    assert_eq!(prompts, 1, "the injected AGENTS.md message is not a prompt");
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
        r#"{"type":"response_item"}"#,
        r#"{"type":"response_item","payload":7}"#,
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":7}}"#,
        r#"{"type":"response_item","payload":{"type":"custom_tool_call","input":"x"}}"#,
        r#"{"type":"response_item","payload":{"type":"custom_tool_call_output","output":[]}}"#,
        r#"{"type":"session_meta","payload":{"id":7}}"#,
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
    let records = parse_all(RUNTIME, CODEX_FIXTURE);
    assert!(!records.is_empty());
    records.iter().for_each(assert_carries_no_turn_boundary);
}

#[test]
fn injected_context_and_developer_messages_are_never_prompts() {
    // Lines 3 to 6 of the fixture: three developer messages and the user-role
    // AGENTS.md/environment message. None yields a record or opens a turn.
    let lines: Vec<&str> = CODEX_FIXTURE.lines().take(6).collect();
    assert!(parse_lines(RUNTIME, &lines).is_empty());
    let prompt = prompt_line("after context");
    let mut all = lines.clone();
    all.push(&prompt);
    assert_eq!(
        parse_lines(RUNTIME, &all),
        vec![user_text(0, "after context")]
    );
}

#[test]
fn each_real_prompt_opens_the_next_ordinal() {
    let lines = [
        session_line().to_string(),
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
fn a_prompt_keeps_only_its_user_text_items() {
    let line = response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_text","text":"<environment_context>x</environment_context>"},
                   {"type":"input_text","text":"the real ask"}],
        "internal_chat_message_metadata_passthrough":
            {"content_item_kinds":["environments.environment_context","user.text"]}}));
    let records = parse_lines(RUNTIME, &[session_line(), &line]);
    assert_eq!(records, vec![user_text(0, "the real ask")]);
}

#[test]
fn a_nonzero_exit_code_is_not_ok() {
    let line = output_line(
        "call_x",
        &[
            "Script completed\n",
            r#"{"exit_code":2,"output":"no such file\n"}"#,
        ],
    );
    assert_eq!(
        parse_lines(RUNTIME, &[&line]),
        vec![Record::ToolDetail {
            tool_use_id: "call_x".into(),
            input: None,
            detail: Some("no such file\n".into()),
            ok: Some(false),
        }]
    );
}

#[test]
fn an_output_without_an_exit_code_keeps_its_text_and_claims_nothing() {
    let line = output_line("call_y", &["Script failed\n", "boom"]);
    assert_eq!(
        parse_lines(RUNTIME, &[&line]),
        vec![Record::ToolDetail {
            tool_use_id: "call_y".into(),
            input: None,
            detail: Some("Script failed\nboom".into()),
            ok: None,
        }]
    );
}

#[test]
fn records_before_the_session_meta_have_no_session() {
    let prompt = prompt_line("early");
    assert_eq!(
        parse_lines(RUNTIME, &[&prompt]),
        vec![Record::UserText {
            session_id: None,
            ordinal: 0,
            text: "early".into(),
            human: true,
        }]
    );
}

#[test]
fn prose_before_any_prompt_has_no_turn_to_land_on() {
    let reply = reply_line("orphan");
    assert!(parse_lines(RUNTIME, &[session_line(), &reply]).is_empty());
}

#[test]
fn detect_keys_on_session_meta_with_a_cli_version() {
    assert_eq!(
        parser().detect(r#"{"type":"session_meta","payload":{"id":"i","cli_version":"9.9.9"}}"#),
        Some(Version(1))
    );
    assert_eq!(
        parser().detect(r#"{"type":"session_meta","payload":{"id":"i"}}"#),
        None
    );
    assert_eq!(
        parser().detect(r#"{"type":"session_meta","payload":{"cli_version":"9.9.9"}}"#),
        None
    );
    assert_eq!(
        parser().detect(r#"{"type":"response_item","payload":{"id":"i","cli_version":"1"}}"#),
        None
    );
}

/// Review F6: two `user.text` items are one prompt, joined by a newline.
#[test]
fn a_prompt_split_across_two_text_items_is_joined() {
    let line = response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_text","text":"a"},{"type":"input_text","text":"b"}],
        "internal_chat_message_metadata_passthrough":
            {"content_item_kinds":["user.text","user.text"]}}));
    assert_eq!(
        parse_lines(RUNTIME, &[session_line(), &line]),
        vec![user_text(0, "a\nb")]
    );
}

/// Review F5: a user message that is a prompt by any `user.*` kind opens a turn even
/// when its kinds are missing, shorter than its content, or name no text at all.
#[test]
fn a_prompt_opens_a_turn_whatever_its_kinds_say() {
    let short_kinds = response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_image","image_url":"x"},{"type":"input_text","text":"two"}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.image"]}}));
    let no_kinds = response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_text","text":"four"}]}));
    let image_only = response_item(json!({"type":"message","role":"user",
        "content":[{"type":"input_image","image_url":"y"}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.image"]}}));
    let lines = [
        session_line().to_string(),
        prompt_line("one"),
        reply_line("r1"),
        short_kinds,
        reply_line("r2"),
        prompt_line("three"),
        reply_line("r3"),
        no_kinds,
        reply_line("r4"),
        image_only,
        reply_line("r5"),
        prompt_line("six"),
    ];
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    assert_eq!(
        parse_lines(RUNTIME, &lines),
        vec![
            user_text(0, "one"),
            assistant_text(0, "r1"),
            user_text(1, "two"),
            assistant_text(1, "r2"),
            user_text(2, "three"),
            assistant_text(2, "r3"),
            user_text(3, "four"),
            assistant_text(3, "r4"),
            // The image-only prompt opens turn 4 but has no text to offer.
            assistant_text(4, "r5"),
            user_text(5, "six"),
        ]
    );
}
