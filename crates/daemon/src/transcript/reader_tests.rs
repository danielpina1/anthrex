//! Every test here builds a real file and reads it through `Tail`, rather than reading
//! the code.

use super::*;
use crate::transcript::tests::{CLAUDE_FIXTURE, CODEX_FIXTURE, parse_all};
use crate::transcript::{Record, parser_for};
use serde_json::json;
use std::fs::File;
use std::io::Write;

const SESSION: &str = "sess-reader";

fn claude() -> &'static dyn TranscriptParser {
    parser_for(proto::Runtime::Claude).unwrap()
}

/// A real Claude prompt line, shaped like the golden fixture's line 8: it carries the
/// `sessionId` detection keys on and the `origin.kind` a prompt needs.
fn prompt(text: &str) -> String {
    json!({"type":"user","sessionId":SESSION,"isSidechain":false,
        "origin":{"kind":"human"},"message":{"role":"user","content":text}})
    .to_string()
}

fn user_text(ordinal: u32, text: &str) -> Record {
    Record::UserText {
        session_id: Some(SESSION.into()),
        ordinal,
        text: text.into(),
    }
}

fn texts(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| match record {
            Record::UserText { text, .. } => text.clone(),
            other => panic!("expected only prompts, got {other:?}"),
        })
        .collect()
}

fn append(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

/// Reads until the tail has consumed the whole file, collecting every pass's records
/// and the last pass's outcome. Bounded by the file length: each pass consumes at least
/// one byte while any remain.
fn read_to_end(tail: &mut Tail, parser: &dyn TranscriptParser) -> (Vec<Record>, ReadOutcome) {
    let len = std::fs::metadata(tail.path()).unwrap().len();
    let mut records = Vec::new();
    loop {
        let before = tail.offset();
        let mut outcome = tail.read_more(parser);
        records.append(&mut outcome.records);
        if tail.offset() >= len {
            return (records, outcome);
        }
        assert!(
            tail.offset() > before,
            "a pass made no progress at {before}"
        );
    }
}

#[test]
fn a_truncated_final_line_is_held_not_parsed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let third = prompt("three");
    let (head, tail_half) = third.split_at(third.len() / 2);
    append(
        &path,
        format!("{}\n{}\n{head}", prompt("one"), prompt("two")).as_bytes(),
    );

    let mut tail = Tail::new(path.clone());
    let first = tail.read_more(claude());
    assert_eq!(
        first.records,
        vec![user_text(0, "one"), user_text(1, "two")]
    );
    assert_eq!(
        first.degraded, None,
        "a partial line is not yet, never corrupt"
    );

    append(&path, format!("{tail_half}\n").as_bytes());
    let second = tail.read_more(claude());
    assert_eq!(second.records, vec![user_text(2, "three")]);
    assert_eq!(second.degraded, None);
    assert!(!second.restarted);
}

#[test]
fn an_absent_file_degrades_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let mut tail = Tail::new(dir.path().join("never-written.jsonl"));
    for _ in 0..2 {
        let outcome = tail.read_more(claude());
        assert_eq!(outcome.degraded, Some(DegradeReason::Unreadable));
        assert!(outcome.records.is_empty());
    }
}

#[test]
fn a_directory_degrades_unreadable() {
    let dir = tempfile::tempdir().unwrap();
    let mut tail = Tail::new(dir.path().to_path_buf());
    let outcome = tail.read_more(claude());
    assert_eq!(outcome.degraded, Some(DegradeReason::Unreadable));
    assert!(outcome.records.is_empty());
}

/// Opening a FIFO for reading blocks until a writer appears; the reader opens with
/// `O_NONBLOCK`, so a pipe planted at the path costs one `Unreadable`, not a blocking
/// thread held forever.
#[test]
fn a_fifo_degrades_unreadable_without_blocking() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pipe.jsonl");
    let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done_tx.send(Tail::new(path).read_more(claude()));
    });
    let outcome = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reading a FIFO blocked");
    assert_eq!(outcome.degraded, Some(DegradeReason::Unreadable));
}

/// Detection looks ahead `DETECT_LINES` lines (task M6.5.7 review F1), so one
/// unrecognised line is "not yet", and `DETECT_LINES` of them is `UnknownFormat`.
#[test]
fn an_unknown_first_line_degrades_unknown_format() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, b"{\"x\":1}\n");
    let mut tail = Tail::new(path.clone());
    let held = tail.read_more(claude());
    assert_eq!(held.degraded, None, "one line is not enough to give up on");
    assert!(held.records.is_empty());

    for n in 1..DETECT_LINES {
        append(&path, format!("{{\"x\":{}}}\n", n + 1).as_bytes());
    }
    let unknown = tail.read_more(claude());
    assert_eq!(unknown.degraded, Some(DegradeReason::UnknownFormat));
    assert!(unknown.records.is_empty());

    // A recognisable line after the verdict is never parsed: the tail does not retry.
    append(&path, format!("{}\n", prompt("late")).as_bytes());
    let offset = tail.offset();
    let again = tail.read_more(claude());
    assert_eq!(again.degraded, Some(DegradeReason::UnknownFormat));
    assert!(again.records.is_empty());
    assert_eq!(tail.offset(), offset, "nothing more was read");
}

/// Claude's first line is a bookkeeping record in every real file (task M6.5.7): held
/// lines are parsed once detection answers, the first included.
#[test]
fn lines_held_during_detection_are_parsed_once_it_answers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    // An old `summary` first line has no `sessionId`, so it alone detects nothing.
    append(&path, b"{\"type\":\"summary\",\"summary\":\"s\"}\n");
    let mut tail = Tail::new(path.clone());
    assert_eq!(tail.read_more(claude()), ReadOutcome::default());
    append(&path, format!("{}\n", prompt("first")).as_bytes());
    let outcome = tail.read_more(claude());
    assert_eq!(outcome.records, vec![user_text(0, "first")]);
    assert_eq!(outcome.degraded, None);
}

#[test]
fn a_line_of_the_wrong_shape_degrades_bad_record_but_keeps_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(
        &path,
        format!(
            "{}\n{{\"type\":\"assistant\"}}\n{}\n",
            prompt("before the bad line"),
            prompt("after the bad line")
        )
        .as_bytes(),
    );
    let outcome = Tail::new(path).read_more(claude());
    assert_eq!(
        outcome.records,
        vec![
            user_text(0, "before the bad line"),
            user_text(1, "after the bad line")
        ]
    );
    assert_eq!(outcome.degraded, Some(DegradeReason::BadRecord));
}

/// A line that is not JSON at all is broken the same way, and `BadRecord` stays on a
/// later clean pass: the record it cost does not come back.
#[test]
fn bad_record_is_sticky_until_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, format!("{}\n{{{{{{\n", prompt("one")).as_bytes());
    let mut tail = Tail::new(path.clone());
    assert_eq!(
        tail.read_more(claude()).degraded,
        Some(DegradeReason::BadRecord)
    );
    append(&path, format!("{}\n", prompt("two")).as_bytes());
    let clean = tail.read_more(claude());
    assert_eq!(clean.records, vec![user_text(1, "two")]);
    assert_eq!(clean.degraded, Some(DegradeReason::BadRecord));
}

/// Neither committed capture has a line the reader calls broken: `malformed` must not
/// flag a real record, and the tail must yield exactly what the parser does line by line.
#[test]
fn both_golden_fixtures_read_cleanly() {
    for (runtime, fixture) in [
        (proto::Runtime::Claude, CLAUDE_FIXTURE),
        (proto::Runtime::Codex, CODEX_FIXTURE),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        append(&path, fixture.as_bytes());
        let parser = parser_for(runtime).unwrap();
        let (records, outcome) = read_to_end(&mut Tail::new(path), parser);
        assert_eq!(outcome.degraded, None, "{runtime:?}");
        assert_eq!(records, parse_all(runtime, fixture), "{runtime:?}");
    }
}

#[test]
fn an_oversized_file_degrades_too_large() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let file = File::create(&path).unwrap();
    file.set_len(TRANSCRIPT_MAX_BYTES + 1).unwrap();
    let mut tail = Tail::new(path);
    let outcome = tail.read_more(claude());
    assert_eq!(outcome.degraded, Some(DegradeReason::TooLarge));
    assert!(outcome.records.is_empty());
    assert_eq!(tail.offset(), 0, "not read at all");
}

#[test]
fn an_oversized_line_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let mut bytes = format!("{}\n", prompt("before the long line")).into_bytes();
    bytes.extend(std::iter::repeat_n(b'x', TRANSCRIPT_LINE_MAX + 1));
    bytes.push(b'\n');
    bytes.extend(format!("{}\n", prompt("after the long line")).into_bytes());
    append(&path, &bytes);

    let (records, outcome) = read_to_end(&mut Tail::new(path), claude());
    assert_eq!(
        records,
        vec![
            user_text(0, "before the long line"),
            user_text(1, "after the long line")
        ]
    );
    assert_eq!(outcome.degraded, Some(DegradeReason::BadRecord));
}

/// A line exactly at the limit is kept: the bound is "larger than", not "at".
#[test]
fn a_line_at_the_limit_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let short = prompt("");
    let text = "y".repeat(TRANSCRIPT_LINE_MAX - short.len());
    let line = prompt(&text);
    assert_eq!(line.len(), TRANSCRIPT_LINE_MAX);
    append(&path, format!("{line}\n").as_bytes());
    let (records, outcome) = read_to_end(&mut Tail::new(path), claude());
    assert_eq!(records, vec![user_text(0, &text)]);
    assert_eq!(outcome.degraded, None);
}

#[test]
fn a_file_being_appended_to_while_read_never_loses_a_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    std::fs::write(&path, b"").unwrap();
    let expected: Vec<String> = (0..200).map(|n| format!("line-{n:03}")).collect();

    let writer_path = path.clone();
    let writer_lines = expected.clone();
    let writer = std::thread::spawn(move || {
        for text in writer_lines {
            // Each line lands in two writes, so a pass can see half of one.
            let line = format!("{}\n", prompt(&text));
            let (a, b) = line.as_bytes().split_at(line.len() / 2);
            append(&writer_path, a);
            std::thread::sleep(Duration::from_micros(300));
            append(&writer_path, b);
        }
    });

    let mut tail = Tail::new(path);
    let mut seen = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while seen.len() < expected.len() {
        let outcome = tail.read_more(claude());
        assert_eq!(outcome.degraded, None);
        assert!(!outcome.restarted);
        seen.extend(texts(&outcome.records));
        assert!(
            Instant::now() < deadline,
            "only {} records arrived",
            seen.len()
        );
        std::thread::sleep(Duration::from_micros(200));
    }
    writer.join().unwrap();
    seen.extend(texts(&tail.read_more(claude()).records));
    assert_eq!(seen, expected, "every record exactly once, in order");
}

#[test]
fn a_shrunken_file_restarts_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(
        &path,
        format!(
            "{}\n{}\n{}\n",
            prompt("old-a"),
            prompt("old-b"),
            prompt("old-c")
        )
        .as_bytes(),
    );
    let mut tail = Tail::new(path.clone());
    assert_eq!(tail.read_more(claude()).records.len(), 3);

    File::create(&path).unwrap().set_len(0).unwrap();
    append(
        &path,
        format!("{}\n{}\n", prompt("new-a"), prompt("new-b")).as_bytes(),
    );
    let outcome = tail.read_more(claude());
    assert!(outcome.restarted);
    // Ordinals start from 0 again: the cursor restarted with the offset.
    assert_eq!(
        outcome.records,
        vec![user_text(0, "new-a"), user_text(1, "new-b")]
    );
    assert!(
        !tail.read_more(claude()).restarted,
        "only the pass that saw it"
    );
}

/// A file replaced by another of at least the same length did not shrink, but it is
/// not the file the offset belongs to either: the inode says so.
#[test]
fn a_replaced_file_restarts_the_tail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    append(&path, format!("{}\n", prompt("old")).as_bytes());
    let mut tail = Tail::new(path.clone());
    assert_eq!(tail.read_more(claude()).records, vec![user_text(0, "old")]);

    let replacement = dir.path().join("next.jsonl");
    append(
        &replacement,
        format!("{}\n{}\n", prompt("new-a"), prompt("new-b")).as_bytes(),
    );
    std::fs::rename(&replacement, &path).unwrap();
    let outcome = tail.read_more(claude());
    assert!(outcome.restarted);
    assert_eq!(
        outcome.records,
        vec![user_text(0, "new-a"), user_text(1, "new-b")]
    );
}

#[test]
fn the_read_budget_bounds_one_pass() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let pad = "p".repeat(1000);
    let mut bytes = Vec::new();
    let mut expected = Vec::new();
    let mut n = 0;
    while bytes.len() < 4 * 1024 * 1024 {
        let text = format!("record-{n:05}-{pad}");
        bytes.extend(format!("{}\n", prompt(&text)).into_bytes());
        expected.push(text);
        n += 1;
    }
    append(&path, &bytes);

    let mut tail = Tail::new(path);
    let first = tail.read_more(claude());
    assert!(tail.offset() > 0);
    assert!(
        tail.offset() <= TRANSCRIPT_READ_BUDGET as u64,
        "the first pass consumed {} bytes",
        tail.offset()
    );
    let mut seen = texts(&first.records);
    let mut passes = 1;
    while tail.offset() < bytes.len() as u64 {
        let before = tail.offset();
        seen.extend(texts(&tail.read_more(claude()).records));
        assert!(tail.offset() - before <= TRANSCRIPT_READ_BUDGET as u64);
        passes += 1;
    }
    assert!(passes >= 4, "4 MiB took only {passes} passes");
    assert_eq!(seen, expected, "no record lost or duplicated");
}

/// A pass whose deadline has already passed stops after one chunk, keeping its place.
#[test]
fn a_pass_past_its_deadline_returns_what_it_has() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let pad = "q".repeat(1000);
    let mut bytes = Vec::new();
    let mut n = 0;
    while bytes.len() < TRANSCRIPT_READ_BUDGET {
        bytes.extend(format!("{}\n", prompt(&format!("r-{n:05}-{pad}"))).into_bytes());
        n += 1;
    }
    append(&path, &bytes);
    let mut tail = Tail::new(path);
    let outcome = tail.read_more_until(claude(), Instant::now());
    assert_eq!(tail.offset(), CHUNK as u64);
    assert!(!outcome.records.is_empty());
    let (rest, _) = read_to_end(&mut tail, claude());
    assert_eq!(outcome.records.len() + rest.len(), n);
}

/// Codex's broken shape: a `response_item` with no readable payload. Its bookkeeping
/// lines (`event_msg`, `turn_context`) are not broken, which the fixture test covers.
#[test]
fn a_codex_response_item_without_a_payload_degrades_bad_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let meta = CODEX_FIXTURE.lines().next().unwrap();
    append(
        &path,
        format!("{meta}\n{{\"type\":\"response_item\",\"payload\":7}}\n").as_bytes(),
    );
    let codex = parser_for(proto::Runtime::Codex).unwrap();
    let outcome = Tail::new(path).read_more(codex);
    assert_eq!(outcome.degraded, Some(DegradeReason::BadRecord));
}
