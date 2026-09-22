//! Task M6.5.10 fix round 2 (re-review N1, N2): a session whose transcript already holds
//! its earlier turns when the reader opens it — Claude's `SessionStart` with `source:
//! "resume"`, from `/resume` inside a window or from milestone 6's restore relaunching
//! with `--resume` — must never have those old records laid onto its new turns.
//!
//! These run the real reader over real files, with hooks driven into the manager and a
//! subscriber on a real socket, because the fault is in what the reader does with a file
//! that is not empty, which no `ConversationSet`-level test can express. Every reply text
//! is distinct, so a transposition cannot pass by coincidence.

mod support;

use daemon::conversation::watch::TRANSCRIPT_POLL;
use daemon::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use proto::{Block, ClientMsg, DaemonMsg, DegradeReason, HookSource, PROTO_VERSION, Role};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use support::{Client, TestDaemon, claude_window, start_daemon};

/// One poll interval and one full pass (`conversation/watch.rs`,
/// `transcript/reader.rs`), twice, since a switch can take a pass on the old file first,
/// plus slack for a shared test runtime.
fn bound() -> Duration {
    (TRANSCRIPT_POLL + TRANSCRIPT_READ_TIMEOUT) * 2 + Duration::from_secs(2)
}

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

fn prompt_line(session: &str, text: &str) -> Value {
    json!({"type":"user","sessionId":session,"isSidechain":false,"origin":{"kind":"human"},
        "message":{"role":"user","content":text}})
}

fn reply_line(session: &str, text: &str) -> Value {
    json!({"type":"assistant","sessionId":session,"isSidechain":false,
        "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
}

fn hook(d: &TestDaemon, id: u32, payload: Value) {
    d.manager
        .handle_hook(id, HookSource::Claude, &payload)
        .unwrap();
}

fn session_start(d: &TestDaemon, id: u32, session: &str, source: &str, path: &Path) {
    hook(
        d,
        id,
        json!({"hook_event_name":"SessionStart","session_id":session,"source":source,
            "transcript_path":path.to_str().unwrap()}),
    );
}

fn turn(d: &TestDaemon, id: u32, session: &str, text: &str) {
    hook(
        d,
        id,
        json!({"hook_event_name":"UserPromptSubmit","session_id":session,"prompt":text}),
    );
    hook(
        d,
        id,
        json!({"hook_event_name":"Stop","session_id":session}),
    );
}

/// Each `Assistant` turn's prose, in order.
fn replies(d: &TestDaemon, id: u32) -> Vec<Vec<String>> {
    d.manager
        .conversation_snapshot(id, None)
        .map(|c| {
            c.turns
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
                .collect()
        })
        .unwrap_or_default()
}

fn degraded(d: &TestDaemon, id: u32) -> Option<DegradeReason> {
    d.manager.conversation_snapshot(id, None).unwrap().degraded
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + bound();
    while !ready() {
        assert!(Instant::now() < deadline, "{what} within {:?}", bound());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// Waits until the `n`-th `Assistant` turn (0-based) has any prose.
async fn until_reply(d: &TestDaemon, id: u32, n: usize) {
    until(&format!("reply {n} enriched"), || {
        replies(d, id).get(n).is_some_and(|r| !r.is_empty())
    })
    .await;
}

async fn until_reading(d: &TestDaemon, id: u32, path: &Path) {
    until(&format!("the reader on {}", path.display()), || {
        d.manager.conversation_transcript_read(id).as_deref() == Some(path)
    })
    .await;
}

async fn subscribed(d: &TestDaemon, id: u32) -> Client {
    let (mut c, _) = Client::connect(d, PROTO_VERSION).await;
    c.send(ClientMsg::SubscribeConversation {
        window_id: id,
        agent_id: None,
        from_rev: None,
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::ConversationSnapshot { .. }))
        .await;
    c
}

fn owned(v: &[&[&str]]) -> Vec<Vec<String>> {
    v.iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect()
}

/// N1: session A, `/clear` to session B, then `/resume` back to A, whose file already
/// holds A's first exchange. The resumed turn gets A's *new* reply, never "A reply 1".
async fn resume_inside_a_window(resumed_prompt: &str, resumed_reply: &str) {
    let d = start_daemon().await;
    let id = claude_window(&d, "resume").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.jsonl"), dir.path().join("b.jsonl"));

    append(
        &a,
        &[
            prompt_line("sess-A", "continue"),
            reply_line("sess-A", "A reply 1"),
        ],
    );
    session_start(&d, id, "sess-A", "startup", &a);
    turn(&d, id, "sess-A", "continue");
    until_reply(&d, id, 0).await;

    append(
        &b,
        &[
            prompt_line("sess-B", "hello B"),
            reply_line("sess-B", "B reply 1"),
        ],
    );
    session_start(&d, id, "sess-B", "clear", &b);
    turn(&d, id, "sess-B", "hello B");
    until_reply(&d, id, 1).await;

    session_start(&d, id, "sess-A", "resume", &a);
    until_reading(&d, id, &a).await;
    // As Claude does it: the hook first, then the transcript lines.
    turn(&d, id, "sess-A", resumed_prompt);
    append(
        &a,
        &[
            prompt_line("sess-A", resumed_prompt),
            reply_line("sess-A", resumed_reply),
        ],
    );
    until_reply(&d, id, 2).await;
    assert_eq!(
        replies(&d, id),
        owned(&[&["A reply 1"], &["B reply 1"], &[resumed_reply]])
    );
    assert_eq!(degraded(&d, id), None);
}

#[tokio::test]
async fn a_resumed_session_repeating_its_first_prompt_gets_its_new_reply() {
    resume_inside_a_window("continue", "A reply 2 after resume").await;
}

#[tokio::test]
async fn a_resumed_session_with_a_new_prompt_gets_its_reply() {
    resume_inside_a_window("go on", "A reply to go on").await;
}

/// N2: milestone 6's restore relaunches Claude with `--resume`, so a window's very first
/// `SessionStart` can be a resume onto a file holding the session's earlier turns.
async fn a_window_that_starts_resumed(prompt: &str, reply: &str) {
    let d = start_daemon().await;
    let id = claude_window(&d, "restored").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(
        &a,
        &[
            prompt_line("sess-A", "continue"),
            reply_line("sess-A", "OLD reply from before restart"),
        ],
    );
    session_start(&d, id, "sess-A", "resume", &a);
    until_reading(&d, id, &a).await;
    turn(&d, id, "sess-A", prompt);
    append(
        &a,
        &[prompt_line("sess-A", prompt), reply_line("sess-A", reply)],
    );
    until_reply(&d, id, 0).await;
    assert_eq!(replies(&d, id), owned(&[&[reply]]));
    assert_eq!(degraded(&d, id), None);
}

#[tokio::test]
async fn a_window_resumed_at_startup_repeating_a_prompt_gets_its_new_reply() {
    a_window_that_starts_resumed("continue", "NEW reply").await;
}

#[tokio::test]
async fn a_window_resumed_at_startup_with_a_new_prompt_gets_its_reply() {
    a_window_that_starts_resumed("keep going", "NEW reply to keep going").await;
}

/// The race the cure accepts: the resumed turn's own lines were already in the file when
/// the reader opened it. That turn gets no prose (it cannot be told apart from the old
/// records), never the old reply; the turn after it is enriched normally.
#[tokio::test]
async fn a_resumed_turn_written_before_the_open_gets_no_prose_never_the_old_reply() {
    let d = start_daemon().await;
    let id = claude_window(&d, "raced").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(
        &a,
        &[
            prompt_line("sess-A", "continue"),
            reply_line("sess-A", "OLD reply from before restart"),
        ],
    );
    turn(&d, id, "sess-A", "continue");
    append(
        &a,
        &[
            prompt_line("sess-A", "continue"),
            reply_line("sess-A", "raced reply"),
        ],
    );
    session_start(&d, id, "sess-A", "resume", &a);
    until_reading(&d, id, &a).await;

    turn(&d, id, "sess-A", "and then");
    append(
        &a,
        &[
            prompt_line("sess-A", "and then"),
            reply_line("sess-A", "reply after the open"),
        ],
    );
    until_reply(&d, id, 1).await;
    assert_eq!(replies(&d, id), owned(&[&[], &["reply after the open"]]));
    assert_eq!(degraded(&d, id), None);
}

/// The negative: a fresh session (`startup`) is read from the start as before, so a
/// file written ahead of its hooks still enriches from ordinal 0.
#[tokio::test]
async fn a_fresh_session_is_still_read_from_its_start() {
    let d = start_daemon().await;
    let id = claude_window(&d, "fresh").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(
        &a,
        &[
            prompt_line("sess-A", "first"),
            reply_line("sess-A", "first reply"),
        ],
    );
    session_start(&d, id, "sess-A", "startup", &a);
    turn(&d, id, "sess-A", "first");
    until_reply(&d, id, 0).await;
    assert_eq!(replies(&d, id), owned(&[&["first reply"]]));
}
