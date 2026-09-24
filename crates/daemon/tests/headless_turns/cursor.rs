//! The delivery order and the conversation cursor (M8a.7's carries into M8a.18): every
//! sent turn is recorded before its message is written or its process spawned, the
//! recorded text is the clamped one, a fed Claude window's cursor is content-based from
//! its first turn, and a background turn's `result` never ends the delivered turn.

use super::support::headless::*;
use super::{
    CLAUDE_UUID, Cleanup, argvs, claude_recorder, codex_recorder, is_exit, is_turn_end, stdin_lines,
};
use daemon::headless::SessionEvent;
use daemon::headless::claude_stream::user_message;
use daemon::manager::{WindowManager, WindowSignalKind};
use daemon::run::messages::{MESSAGE_MAX_BYTES, clamp};
use proto::{Block, HookSource, Role, Runtime, Status};
use serde_json::json;
use std::path::Path;

const BG: &str = "<task-notification>agent done</task-notification>";

fn hook(m: &WindowManager, id: u32, payload: serde_json::Value) {
    m.handle_hook(id, HookSource::Claude, &payload).unwrap();
}

fn session_start(m: &WindowManager, id: u32) {
    hook(
        m,
        id,
        json!({"hook_event_name": "SessionStart", "session_id": CLAUDE_UUID, "source": "startup"}),
    );
}

fn prompt(m: &WindowManager, id: u32, text: &str) {
    hook(
        m,
        id,
        json!({"hook_event_name": "UserPromptSubmit", "session_id": CLAUDE_UUID, "prompt": text}),
    );
}

fn init() -> String {
    format!(r#"{{"type":"system","subtype":"init","session_id":"{CLAUDE_UUID}","model":"m"}}"#)
}

fn prose(text: &str) -> String {
    json!({"type": "assistant", "parent_tool_use_id": null,
           "message": {"content": [{"type": "text", "text": text}]}})
    .to_string()
}

fn result() -> String {
    r#"{"type":"result","subtype":"success","is_error":false}"#.to_string()
}

/// A Claude that prints `steps[i]` once the file `s<i>` exists, then idles.
fn stepper(dir: &Path, steps: &[Vec<String>]) -> std::path::PathBuf {
    let mut body = String::new();
    for (i, lines) in steps.iter().enumerate() {
        let file = dir.join(format!("step{i}.jsonl"));
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
        body += &format!(
            "{}\ncat '{}'\n",
            gate(dir, &format!("s{i}")),
            file.display()
        );
    }
    body += "exec sleep 30";
    script(dir, "claude", &body)
}

/// The conversation as (prompt, reply) pairs; a turn with no reply pairs with "".
fn pairs(m: &WindowManager, id: u32) -> Vec<(String, String)> {
    let conversation = m.conversation_snapshot(id, None).expect("a conversation");
    assert_eq!(conversation.degraded, None, "{conversation:?}");
    let text = |blocks: &[Block]| {
        blocks
            .iter()
            .find_map(|b| match b {
                Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for turn in &conversation.turns {
        match turn.role {
            Role::User => out.push((text(&turn.blocks), String::new())),
            Role::Assistant => {
                let last = out.last_mut().expect("a reply follows a prompt");
                last.1 = text(&turn.blocks);
            }
            _ => {}
        }
    }
    out
}

fn owned(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(p, r)| (p.to_string(), r.to_string()))
        .collect()
}

/// A turn end on the feed, delivered (`Session`) or a background turn's (`Unprompted`).
fn any_turn_end(signal: &daemon::manager::WindowSignal) -> bool {
    matches!(
        &signal.kind,
        WindowSignalKind::Session(SessionEvent::TurnEnded { .. })
            | WindowSignalKind::Unprompted(SessionEvent::TurnEnded { .. })
    )
}

#[tokio::test]
async fn sent_turn_is_recorded_before_the_write() {
    // Codex (binding): `sent_turn` synthesises the prompt; the new process's prose must
    // land on the new turn, not on the previous one.
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    m.headless_send(info.id, "second").await.unwrap();
    next_signal(&mut feed, "the second exit", is_exit).await;
    assert_eq!(
        pairs(&m, info.id),
        owned(&[("first", "reply 1"), ("second", "reply 2")])
    );

    // Claude: the program prints its turn only after reading its input.
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), true);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    session_start(&m, info.id);
    prompt(&m, info.id, "first");
    open_gate(dir.path(), "go1");
    next_signal(&mut feed, "turn 1's end", is_turn_end).await;
    m.headless_send(info.id, "second").await.unwrap();
    prompt(&m, info.id, "second");
    open_gate(dir.path(), "go2");
    next_signal(&mut feed, "turn 2's end", is_turn_end).await;
    assert_eq!(
        pairs(&m, info.id),
        owned(&[("first", "reply 1"), ("second", "reply 2")])
    );
}

/// In the N1 order, a background turn runs just as the daemon delivers its next
/// message. Its `result` is the background turn's: it neither ends the delivered turn
/// on the window nor reaches the engine as that turn's end.
#[tokio::test]
async fn a_background_turns_result_does_not_end_the_delivered_turn() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stepper(
        dir.path(),
        &[
            vec![init(), prose("reply one"), result()],
            vec![init(), prose("bg reply"), result()],
            vec![init(), prose("reply two"), result()],
        ],
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;
    session_start(&m, id);
    prompt(&m, id, "first");
    open_gate(dir.path(), "s0");
    next_signal(&mut feed, "turn 1's end", is_turn_end).await;
    assert_eq!(find(&m, id).status, Status::Idle);

    m.headless_send(id, "second").await.unwrap();
    prompt(&m, id, BG);
    open_gate(dir.path(), "s1");
    let bg_end = next_signal(&mut feed, "the background turn's end", any_turn_end).await;
    assert!(
        matches!(bg_end.kind, WindowSignalKind::Unprompted(_)),
        "the background result is not the delivered turn's end: {bg_end:?}"
    );
    assert_eq!(
        find(&m, id).status,
        Status::Working,
        "the delivered turn is still open"
    );

    prompt(&m, id, "second");
    open_gate(dir.path(), "s2");
    let end = next_signal(&mut feed, "the delivered turn's end", any_turn_end).await;
    assert!(is_turn_end(&end), "{end:?}");
    assert_eq!(find(&m, id).status, Status::Idle);
    assert_eq!(
        pairs(&m, id),
        owned(&[
            ("first", "reply one"),
            (BG, "bg reply"),
            ("second", "reply two")
        ])
    );
}

#[tokio::test]
async fn sent_turn_records_the_clamped_text() {
    let long = format!("{}{}", "a".repeat(MESSAGE_MAX_BYTES), "z".repeat(100));
    let clamped = clamp(&long);
    assert_ne!(clamped, long);

    // Claude: the clamped text is what is written, and what a prompt hook echoing it is
    // matched against, so its turn is the delivered one.
    let dir = tempfile::tempdir().unwrap();
    let claude = claude_recorder(dir.path(), true);
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    session_start(&m, info.id);
    prompt(&m, info.id, "first");
    open_gate(dir.path(), "go1");
    next_signal(&mut feed, "turn 1's end", is_turn_end).await;
    m.headless_send(info.id, &long).await.unwrap();
    prompt(&m, info.id, &clamped);
    open_gate(dir.path(), "go2");
    let end = next_signal(&mut feed, "turn 2's end", any_turn_end).await;
    assert!(
        is_turn_end(&end),
        "the clamped prompt is the daemon's: {end:?}"
    );
    assert_eq!(
        stdin_lines(dir.path())[1],
        user_message(&clamped, Some(CLAUDE_UUID))
    );

    // Codex: the clamped text is the last argument and the recorded prompt.
    let dir = tempfile::tempdir().unwrap();
    let codex = codex_recorder(dir.path(), "");
    let m = manager(&codex, &codex, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "c", spec(Runtime::Codex, dir.path()), "first").await;
    next_signal(&mut feed, "the first exit", is_exit).await;
    m.headless_send(info.id, &long).await.unwrap();
    next_signal(&mut feed, "the second exit", is_exit).await;
    assert_eq!(argvs(dir.path())[1].last(), Some(&clamped));
    assert_eq!(pairs(&m, info.id)[1], (clamped, "reply 2".to_string()));
}

/// Ruling T7-N1 m1: a fed window's cursor is content-based from its first `sent_turn`,
/// so a background turn that runs before the first delivered one keeps both aligned.
#[tokio::test]
async fn a_fed_window_starts_its_cursor_in_content_mode() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stepper(
        dir.path(),
        &[
            vec![init(), prose("bg reply"), result()],
            vec![init(), prose("reply one"), result()],
        ],
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    session_start(&m, info.id);
    prompt(&m, info.id, BG);
    open_gate(dir.path(), "s0");
    next_signal(&mut feed, "the background turn's end", any_turn_end).await;
    prompt(&m, info.id, "first");
    open_gate(dir.path(), "s1");
    next_signal(&mut feed, "turn 1's end", is_turn_end).await;
    assert_eq!(
        pairs(&m, info.id),
        owned(&[(BG, "bg reply"), ("first", "reply one")])
    );

    // The first turn's prompt hook is lost: only that turn's prose goes, and the window
    // is not misaligned for the rest of the session.
    let dir = tempfile::tempdir().unwrap();
    let claude = stepper(
        dir.path(),
        &[
            vec![init(), prose("reply one"), result()],
            vec![init(), prose("reply two"), result()],
        ],
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    session_start(&m, info.id);
    open_gate(dir.path(), "s0");
    next_signal(&mut feed, "turn 1's end", any_turn_end).await;
    m.headless_send(info.id, "second").await.unwrap();
    prompt(&m, info.id, "second");
    open_gate(dir.path(), "s1");
    next_signal(&mut feed, "turn 2's end", any_turn_end).await;
    assert_eq!(pairs(&m, info.id), owned(&[("second", "reply two")]));
}

/// Ruling T7-N1 m2: prose is dropped only while every observed prompt's turn has ended.
/// A prompt hook applied before the previous `result` line keeps both replies; a prompt
/// hook that arrives after its own turn's prose drops that prose only.
#[tokio::test]
async fn a_late_prompt_hook_drops_only_its_own_turn() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stepper(
        dir.path(),
        &[
            vec![init(), prose("reply one")],
            vec![result()],
            vec![init(), prose("reply two"), result()],
            vec![init(), prose("reply three"), result()],
            vec![init(), prose("reply four"), result()],
        ],
    );
    let m = manager(&claude, &claude, |_| {});
    let _cleanup = Cleanup(m.clone());
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;
    let settle = async |m: &WindowManager, what: &str, want: &[(&str, &str)]| {
        let want = owned(want);
        wait_until(what, || pairs(m, id) == want).await;
    };
    session_start(&m, id);
    prompt(&m, id, "first");
    open_gate(dir.path(), "s0");
    settle(&m, "reply one", &[("first", "reply one")]).await;

    // The next prompt's hook is applied before turn 1's `result` is read.
    m.headless_send(id, "second").await.unwrap();
    prompt(&m, id, "second");
    open_gate(dir.path(), "s1");
    next_signal(&mut feed, "turn 1's end", is_turn_end).await;
    open_gate(dir.path(), "s2");
    next_signal(&mut feed, "turn 2's end", is_turn_end).await;
    assert_eq!(
        pairs(&m, id),
        owned(&[("first", "reply one"), ("second", "reply two")])
    );

    // Turn 3's prompt hook arrives only after its prose: that prose is dropped.
    m.headless_send(id, "third").await.unwrap();
    open_gate(dir.path(), "s3");
    next_signal(&mut feed, "turn 3's end", any_turn_end).await;
    prompt(&m, id, "third");
    // Turn 4 is placed as usual.
    m.headless_send(id, "fourth").await.unwrap();
    prompt(&m, id, "fourth");
    open_gate(dir.path(), "s4");
    next_signal(&mut feed, "turn 4's end", any_turn_end).await;
    assert_eq!(
        pairs(&m, id),
        owned(&[
            ("first", "reply one"),
            ("second", "reply two"),
            ("third", ""),
            ("fourth", "reply four"),
        ])
    );
}
