//! Task M6.5.10 fix round 2 (re-review N1, N2): a session whose transcript already holds
//! its earlier turns when the reader opens it — Claude's `SessionStart` with `source:
//! "resume"`, from `/resume` inside a window or from milestone 6's restore relaunching
//! with `--resume` — must never have those old records laid onto its new turns.
//!
//! These run the real reader over real files, with hooks driven into the manager and a
//! subscriber on a real socket, because the fault is in what the reader does with a file
//! that is not empty, which no `ConversationSet`-level test can express. Every reply text
//! is distinct, so a transposition cannot pass by coincidence. The last test pins the
//! other flag a session switch hands the reader, `new_session` (re-review nit).

mod support;

use daemon::conversation::watch::TRANSCRIPT_POLL;
use daemon::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use proto::{Block, ClientMsg, DaemonMsg, DegradeReason, HookSource, PROTO_VERSION, Role};
use serde_json::{Value, json};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};
use support::{Client, TestDaemon, claude_window, start_daemon, start_daemon_configured};

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

/// Re-review nit: the reader's first `Tail` consumes the new-session flag. A `/clear`
/// made while no reader ran leaves the flag set; if the first `Tail` left it there, a
/// later move of the *same* session to another file would be taken for a new session
/// and skip its restart, so the old file's enrichment would outlive it.
#[tokio::test]
async fn the_readers_first_tail_consumes_the_new_session_flag() {
    let d = start_daemon().await;
    let id = claude_window(&d, "first-tail").await;
    let dir = tempfile::tempdir().unwrap();
    let (a, b, b2) = (
        dir.path().join("a.jsonl"),
        dir.path().join("b.jsonl"),
        dir.path().join("b2.jsonl"),
    );
    append(&a, &[]);
    append(
        &b,
        &[
            prompt_line("sess-B", "hello B"),
            reply_line("sess-B", "B reply from b"),
        ],
    );
    append(&b2, &[]);
    session_start(&d, id, "sess-A", "startup", &a);
    session_start(&d, id, "sess-B", "clear", &b);
    turn(&d, id, "sess-B", "hello B");

    let _c = subscribed(&d, id).await;
    until_reply(&d, id, 0).await;
    session_start(&d, id, "sess-B", "startup", &b2);
    until_reading(&d, id, &b2).await;
    assert_eq!(
        replies(&d, id),
        owned(&[&[]]),
        "the same session moved files: a restart, so b's prose is gone"
    );
}

/// Re-review 2, I2: the reader stops while on A and keeps its `Tail`. With nobody
/// watching, the window `/clear`s to B and `/resume`s back to A, and the resumed turn is
/// taken. When a viewer returns, the kept `Tail` on A must not simply continue: its
/// offset and prompt count are from before the switches.
async fn a_stale_tail(next_prompt: &str) {
    let d = start_daemon_configured(true, |c| c.conversation.linger_secs = 0).await;
    let id = claude_window(&d, "stale").await;
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.jsonl"), dir.path().join("b.jsonl"));
    {
        let _c = subscribed(&d, id).await;
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
    }
    until("the reader stopped", || {
        !d.manager.conversation_reader_running(id)
    })
    .await;

    append(
        &b,
        &[
            prompt_line("sess-B", "hello B"),
            reply_line("sess-B", "B reply 1"),
        ],
    );
    session_start(&d, id, "sess-B", "clear", &b);
    turn(&d, id, "sess-B", "hello B");
    session_start(&d, id, "sess-A", "resume", &a);
    turn(&d, id, "sess-A", "continue");
    append(
        &a,
        &[
            prompt_line("sess-A", "continue"),
            reply_line("sess-A", "A reply 2 after resume"),
        ],
    );

    let _c = subscribed(&d, id).await;
    until_reading(&d, id, &a).await;
    turn(&d, id, "sess-A", next_prompt);
    append(
        &a,
        &[
            prompt_line("sess-A", next_prompt),
            reply_line("sess-A", "A reply 3 to next"),
        ],
    );
    // Two more passes, so anything the file can still move has moved.
    let read_again = Instant::now() + (TRANSCRIPT_POLL + TRANSCRIPT_READ_TIMEOUT) * 2;
    while Instant::now() < read_again {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let own = [
        "A reply 1",
        "B reply 1",
        "A reply 2 after resume",
        "A reply 3 to next",
    ];
    let got = replies(&d, id);
    assert_eq!(got.len(), own.len(), "{got:?}");
    for (i, (prose, mine)) in got.iter().zip(own).enumerate() {
        assert!(
            prose.is_empty() || prose == &[mine.to_string()],
            "turn {i} shows {prose:?}, not its own {mine:?} (all: {got:?})"
        );
    }
}

#[tokio::test]
async fn a_stale_tail_on_a_resumed_file_never_shifts_a_reply_repeated_prompt() {
    a_stale_tail("continue").await;
}

#[tokio::test]
async fn a_stale_tail_on_a_resumed_file_never_shifts_a_reply_new_prompt() {
    a_stale_tail("different").await;
}

/// Re-review 2, M1: Codex's `SessionStart` also has `source: "fork"`, and a fork's file
/// may start with the parent's history. Only `startup` and `clear` promise a fresh file;
/// any other source switching session or file is opened at its end.
async fn a_switch_with_source(source: Option<&str>) {
    let d = start_daemon().await;
    let id = claude_window(&d, "forked").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let (a, f) = (dir.path().join("a.jsonl"), dir.path().join("f.jsonl"));
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
        &f,
        &[
            prompt_line("sess-F", "continue"),
            reply_line("sess-F", "A reply 1 (copied into the fork)"),
        ],
    );
    let mut start = json!({"hook_event_name":"SessionStart","session_id":"sess-F",
        "transcript_path":f.to_str().unwrap()});
    if let Some(source) = source {
        start["source"] = json!(source);
    }
    hook(&d, id, start);
    until_reading(&d, id, &f).await;
    turn(&d, id, "sess-F", "continue");
    append(
        &f,
        &[
            prompt_line("sess-F", "continue"),
            reply_line("sess-F", "F reply 1"),
        ],
    );
    until_reply(&d, id, 1).await;
    assert_eq!(replies(&d, id), owned(&[&["A reply 1"], &["F reply 1"]]));
    assert_eq!(degraded(&d, id), None);
}

#[tokio::test]
async fn a_fork_repeating_the_parents_prompt_gets_its_own_reply() {
    a_switch_with_source(Some("fork")).await;
}

#[tokio::test]
async fn a_compact_onto_another_session_is_opened_at_its_end() {
    a_switch_with_source(Some("compact")).await;
}

#[tokio::test]
async fn an_unknown_source_is_opened_at_its_end() {
    a_switch_with_source(Some("something-new")).await;
}

#[tokio::test]
async fn an_absent_source_is_opened_at_its_end() {
    a_switch_with_source(None).await;
}

/// The same session and file again, whatever the source, is no switch: every turn keeps
/// its own reply and nothing is re-read.
#[tokio::test]
async fn the_same_session_and_file_again_changes_nothing() {
    let d = start_daemon().await;
    let id = claude_window(&d, "same").await;
    let _c = subscribed(&d, id).await;
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.jsonl");
    append(
        &a,
        &[
            prompt_line("sess-A", "q1"),
            reply_line("sess-A", "reply q1"),
        ],
    );
    session_start(&d, id, "sess-A", "startup", &a);
    turn(&d, id, "sess-A", "q1");
    until_reply(&d, id, 0).await;
    for (n, source) in [(2, "compact"), (3, "clear"), (4, "resume"), (5, "fork")] {
        session_start(&d, id, "sess-A", source, &a);
        let (q, r) = (format!("q{n}"), format!("reply q{n}"));
        turn(&d, id, "sess-A", &q);
        append(&a, &[prompt_line("sess-A", &q), reply_line("sess-A", &r)]);
        until_reply(&d, id, n - 1).await;
    }
    assert_eq!(
        replies(&d, id),
        owned(&[
            &["reply q1"],
            &["reply q2"],
            &["reply q3"],
            &["reply q4"],
            &["reply q5"]
        ])
    );
    assert_eq!(degraded(&d, id), None);
}
