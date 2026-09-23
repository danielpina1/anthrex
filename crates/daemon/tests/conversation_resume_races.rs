//! Task M6.5.10 fix rounds 2 and 3: the races around opening a resumed session's
//! transcript at its end, made deterministic by driving the reader by hand
//! (`reader_step`, `Tail::read_more`, `apply_transcript`) instead of through its task.
//!
//! Hooks come in the order Claude produces them: `SessionStart`, then
//! `UserPromptSubmit`, then the prompt's transcript line, then `Stop`. What varies is where
//! the reader's steps fall among them. The rule under test (re-review 2, I1 and M3): a
//! turn gets its own reply, no prose, or the conversation is `Misaligned` — never an old
//! reply or a neighbouring turn's.

mod support;

use daemon::manager::ReaderStep;
use daemon::transcript::parser_for;
use daemon::transcript::reader::Tail;
use proto::{Block, DegradeReason, HookSource, Role};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use support::{TestDaemon, claude_window, start_daemon};
use tokio_util::sync::CancellationToken;

const S: &str = "sess-A";

fn append(path: &Path, lines: &[Value]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for line in lines {
        writeln!(file, "{line}").unwrap();
    }
}

fn exchange(prompt: &str, reply: &str) -> [Value; 2] {
    [
        json!({"type":"user","sessionId":S,"isSidechain":false,"origin":{"kind":"human"},
            "message":{"role":"user","content":prompt}}),
        json!({"type":"assistant","sessionId":S,"isSidechain":false,
            "message":{"role":"assistant","content":[{"type":"text","text":reply}]}}),
    ]
}

fn hook(d: &TestDaemon, id: u32, payload: Value) {
    d.manager
        .handle_hook(id, HookSource::Claude, &payload)
        .unwrap();
}

fn resume(d: &TestDaemon, id: u32, path: &Path) {
    hook(
        d,
        id,
        json!({"hook_event_name":"SessionStart","session_id":S,"source":"resume",
            "transcript_path":path.to_str().unwrap()}),
    );
}

fn prompt(d: &TestDaemon, id: u32, text: &str) {
    hook(
        d,
        id,
        json!({"hook_event_name":"UserPromptSubmit","session_id":S,"prompt":text}),
    );
}

fn stop(d: &TestDaemon, id: u32) {
    hook(d, id, json!({"hook_event_name":"Stop","session_id":S}));
}

/// A subscriber whose reader task exits at once, so this test owns every step. Called
/// before any `SessionStart`: the task's one step is then a `Wait`, and it leaves
/// `reader_running` set, so no second task starts.
async fn by_hand(d: &TestDaemon, id: u32) {
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let _ = d.manager.subscribe_conversation(id, None, None, cancelled);
    tokio::time::sleep(Duration::from_millis(100)).await;
}

fn step(d: &TestDaemon, id: u32) -> Tail {
    match d.manager.reader_step(id) {
        ReaderStep::Read(tail) => tail,
        _ => panic!("expected a read step"),
    }
}

fn read(tail: &mut Tail) -> daemon::transcript::reader::ReadOutcome {
    tail.read_more(parser_for(proto::Runtime::Claude).unwrap())
}

fn pass(d: &TestDaemon, id: u32) {
    let mut tail = step(d, id);
    let outcome = read(&mut tail);
    d.manager.apply_transcript(id, tail, outcome);
}

/// Each `Assistant` turn's prose, and the degrade reason.
fn replies(d: &TestDaemon, id: u32) -> (Vec<Vec<String>>, Option<DegradeReason>) {
    let c = d.manager.conversation_snapshot(id, None).unwrap();
    let prose = c
        .turns
        .iter()
        .filter(|t| t.role == Role::Assistant)
        .map(|t| {
            t.blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
        .collect();
    (prose, c.degraded)
}

/// No turn shows a reply that is not its own. `own[i]` is turn `i`'s own reply.
fn assert_never_misattributed(d: &TestDaemon, id: u32, own: &[&str]) {
    let (prose, degraded) = replies(d, id);
    assert_eq!(prose.len(), own.len(), "{prose:?}");
    for (i, (got, mine)) in prose.iter().zip(own).enumerate() {
        assert!(
            got.is_empty() || got == &[mine.to_string()],
            "turn {i} shows {got:?}, not its own {mine:?} (all: {prose:?}, {degraded:?})"
        );
    }
}

/// The window resumes onto a file holding an old exchange; returns the file.
async fn resumed_window(d: &TestDaemon) -> (u32, tempfile::TempDir, std::path::PathBuf) {
    let id = claude_window(d, "races").await;
    by_hand(d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(&a, &exchange("continue", "OLD reply"));
    resume(d, id, &a);
    (id, dir, a)
}

/// Then the next turn, repeating the prompt, so a shifted ordinal would line up.
fn next_turn(d: &TestDaemon, id: u32, a: &Path) {
    prompt(d, id, "continue");
    append(a, &exchange("continue", "second reply R2"));
    stop(d, id);
    pass(d, id);
}

/// Re-review 2, I1: the prompt's hook lands before the reader's first step on the file,
/// and its line after the measure. The step saw the turn already counted, so the old
/// code based the session past it and its reply landed on the next turn.
#[tokio::test]
async fn a_prompt_hooked_before_the_first_step_never_gets_a_neighbours_reply() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    prompt(&d, id, "continue");
    pass(&d, id);
    append(&a, &exchange("continue", "resumed reply R1"));
    stop(&d, id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_never_misattributed(&d, id, &["resumed reply R1", "second reply R2"]);
    assert_eq!(replies(&d, id).1, Some(DegradeReason::Misaligned));
}

/// Re-review 2, M3: the prompt's hook lands while the measuring pass is in flight, and
/// its line after the measure.
#[tokio::test]
async fn a_prompt_hooked_during_the_measuring_pass_degrades() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    let mut tail = step(&d, id);
    let outcome = read(&mut tail);
    prompt(&d, id, "continue");
    d.manager.apply_transcript(id, tail, outcome);
    append(&a, &exchange("continue", "resumed reply R1"));
    stop(&d, id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_never_misattributed(&d, id, &["resumed reply R1", "second reply R2"]);
    assert_eq!(replies(&d, id).1, Some(DegradeReason::Misaligned));
}

/// M3 again, with the prompt's line written before the measure, so it is skipped.
#[tokio::test]
async fn a_prompt_written_before_the_measure_and_hooked_during_the_pass_degrades() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    let mut tail = step(&d, id);
    append(&a, &exchange("continue", "resumed reply R1"));
    let outcome = read(&mut tail);
    prompt(&d, id, "continue");
    d.manager.apply_transcript(id, tail, outcome);
    stop(&d, id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_never_misattributed(&d, id, &["resumed reply R1", "second reply R2"]);
    assert_eq!(replies(&d, id).1, Some(DegradeReason::Misaligned));
}

/// The whole resumed turn — hook and lines — lands before the reader's first step (a
/// late open). Replaces fix round 2's "race" test, which sent the prompt before the
/// `SessionStart`, an order Claude never produces.
#[tokio::test]
async fn a_whole_turn_before_the_first_step_never_gets_the_old_reply() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    prompt(&d, id, "continue");
    append(&a, &exchange("continue", "resumed reply R1"));
    stop(&d, id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_never_misattributed(&d, id, &["resumed reply R1", "second reply R2"]);
}

/// The ordinary case in the same harness: the reader opens the file before the user
/// types, and every turn gets its own reply with nothing degraded.
#[tokio::test]
async fn a_prompt_after_the_open_gets_its_own_reply() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    pass(&d, id);
    prompt(&d, id, "continue");
    append(&a, &exchange("continue", "resumed reply R1"));
    stop(&d, id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_eq!(
        replies(&d, id),
        (
            vec![
                vec!["resumed reply R1".to_string()],
                vec!["second reply R2".to_string()]
            ],
            None
        )
    );
}

/// Re-review 2, M2: a pass on the resumed file panics and takes its `Tail` with it. The
/// rebuilt tail must still skip what the file held, not re-read it from the start at
/// the open-time base.
#[tokio::test]
async fn a_lost_pass_on_a_resumed_file_never_rereads_it_from_the_start() {
    let d = start_daemon().await;
    let (id, _dir, a) = resumed_window(&d).await;
    pass(&d, id);
    prompt(&d, id, "continue");
    append(&a, &exchange("continue", "resumed reply R1"));
    stop(&d, id);
    pass(&d, id);
    assert_eq!(
        replies(&d, id).0,
        vec![vec!["resumed reply R1".to_string()]]
    );

    drop(step(&d, id));
    d.manager.transcript_lost(id);
    pass(&d, id);
    next_turn(&d, id, &a);
    assert_never_misattributed(&d, id, &["resumed reply R1", "second reply R2"]);
}
