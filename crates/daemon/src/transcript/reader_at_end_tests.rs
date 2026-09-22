//! Task M6.5.10 fix round 2 (N1/N2): a resumed session's file is opened at its end.
//! What it held when first read yields no records; what is written after yields them
//! with prompt ordinals counting from 0. Every test reads a real file through `Tail`.

use super::*;
use crate::transcript::tests::{CODEX_FIXTURE, first_line};
use crate::transcript::{Record, parser_for};
use serde_json::json;
use std::fs::File;
use std::io::Write;

const SESSION: &str = "sess-at-end";

fn claude() -> &'static dyn TranscriptParser {
    parser_for(proto::Runtime::Claude).unwrap()
}

fn prompt(text: &str) -> String {
    json!({"type":"user","sessionId":SESSION,"isSidechain":false,
        "origin":{"kind":"human"},"message":{"role":"user","content":text}})
    .to_string()
}

fn reply(text: &str) -> String {
    json!({"type":"assistant","sessionId":SESSION,"isSidechain":false,
        "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
    .to_string()
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

fn append(path: &Path, lines: &[String]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

#[test]
fn an_at_end_tail_skips_what_the_file_held_and_counts_from_zero_after() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[prompt("continue"), reply("old reply")]);
    let mut tail = Tail::at_end(path.clone());
    let first = tail.read_more(claude());
    assert!(
        first.opened_at_end,
        "the pass that measured the file says so"
    );
    assert_eq!(first.records, vec![], "nothing the file held is emitted");
    assert_eq!(first.degraded, None);

    // Prose written after the skip but before any new prompt has no turn to land on.
    append(
        &path,
        &[reply("stray"), prompt("continue"), reply("new reply")],
    );
    let second = tail.read_more(claude());
    assert!(!second.opened_at_end, "only the measuring pass");
    assert_eq!(
        second.records,
        vec![user_text(0, "continue"), assistant_text(0, "new reply")]
    );
}

#[test]
fn an_ordinary_tail_still_reads_from_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[prompt("first")]);
    let outcome = Tail::new(path).read_more(claude());
    assert!(!outcome.opened_at_end);
    assert_eq!(outcome.records, vec![user_text(0, "first")]);
}

/// A line still being written when the file was measured began before the end, so it
/// is old: its record, once complete, is skipped too, never emitted as the next turn's.
#[test]
fn a_line_straddling_the_measured_end_is_old() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[prompt("done")]);
    let straddling = prompt("half written");
    let (head, rest) = straddling.split_at(10);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(head.as_bytes())
        .unwrap();
    let mut tail = Tail::at_end(path.clone());
    assert_eq!(tail.read_more(claude()).records, vec![]);

    append(&path, &[rest.to_string(), prompt("after")]);
    assert_eq!(
        tail.read_more(claude()).records,
        vec![user_text(0, "after")]
    );
}

/// A broken line the skip passed over cost no record, so it is no `BadRecord`.
#[test]
fn a_broken_line_before_the_end_does_not_degrade() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[prompt("ok"), "{not json".into()]);
    let mut tail = Tail::at_end(path.clone());
    assert_eq!(tail.read_more(claude()).degraded, None);
    append(&path, &["{still not json".into()]);
    assert_eq!(
        tail.read_more(claude()).degraded,
        Some(DegradeReason::BadRecord),
        "a broken line after the end does"
    );
}

/// A restart keeps the tail an at-end one: the replacement file is measured again, and
/// the pass reports both, so the caller restarts and re-bases in one step.
#[test]
fn an_at_end_tail_measures_again_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[prompt("one"), prompt("two")]);
    let mut tail = Tail::at_end(path.clone());
    assert!(tail.read_more(claude()).opened_at_end);

    File::create(&path).unwrap().set_len(0).unwrap();
    append(&path, &[prompt("replaced")]);
    let outcome = tail.read_more(claude());
    assert!(outcome.restarted);
    assert!(outcome.opened_at_end);
    assert_eq!(outcome.records, vec![]);
    append(&path, &[prompt("fresh")]);
    assert_eq!(
        tail.read_more(claude()).records,
        vec![user_text(0, "fresh")]
    );
}

/// What the cursor carries across the whole file still comes from the skipped part:
/// Codex states its session id once, on the first line.
#[test]
fn a_codex_session_id_from_the_skipped_head_is_kept() {
    let parser = parser_for(proto::Runtime::Codex).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, &[first_line(CODEX_FIXTURE).to_string()]);
    let mut tail = Tail::at_end(path.clone());
    assert_eq!(tail.read_more(parser).records, vec![]);
    let line = json!({"timestamp":"t","type":"response_item","payload":{
        "type":"message","role":"user",
        "content":[{"type":"input_text","text":"resumed"}],
        "internal_chat_message_metadata_passthrough":{"content_item_kinds":["user.text"]}}});
    append(&path, &[line.to_string()]);
    match tail.read_more(parser).records.as_slice() {
        [
            Record::UserText {
                session_id: Some(_),
                ordinal: 0,
                text,
            },
        ] => assert_eq!(text, "resumed"),
        other => panic!("expected the resumed prompt with its session id, got {other:?}"),
    }
}
