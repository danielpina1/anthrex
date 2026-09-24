//! Real hooks of a headless window (decision 27): they build its conversation and its
//! sub-agent rows, never its status, and its prompts feed the stream cursor (ruling
//! T7-N1).

use super::support::headless::*;
use super::support::{Client, start_daemon_configured};
use super::{CLAUDE_SESSION, is_exit};
use daemon::headless::SessionEvent;
use daemon::hooks::HookKind;
use daemon::manager::WindowSignalKind;
use proto::{Block, ClientMsg, DaemonMsg, HookSource, PROTO_VERSION, Role, Runtime, Status};
use serde_json::json;
use std::path::Path;
use tokio_util::sync::CancellationToken;

fn stream_fixture_claude(dir: &Path) -> std::path::PathBuf {
    script(
        dir,
        "claude",
        &format!(
            "cat '{}'; exec sleep 30",
            fixture("claude-2.1.278-stream.jsonl").display()
        ),
    )
}

#[tokio::test]
async fn real_hooks_do_not_change_a_headless_windows_status() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stream_fixture_claude(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let mut feed = m.signals();
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "hello").await;
    // Every turn the fixture holds has ended: nothing but hooks moves it from here.
    let mut parser = daemon::headless::claude_stream::ClaudeStream::default();
    let turns = std::fs::read_to_string(fixture("claude-2.1.278-stream.jsonl"))
        .unwrap()
        .lines()
        .flat_map(|line| parser.parse_line(line))
        .filter(|e| matches!(e, SessionEvent::TurnEnded { .. }))
        .count();
    assert!(turns > 1, "the fixture has several turns");
    for _ in 0..turns {
        next_signal(&mut feed, "a turn end", |s| {
            matches!(
                &s.kind,
                WindowSignalKind::Session(SessionEvent::TurnEnded { .. })
            )
        })
        .await;
    }
    assert_eq!(find(&m, info.id).status, Status::Idle);

    let hook = |payload: serde_json::Value| m.handle_hook(info.id, HookSource::Claude, &payload);
    hook(json!({"hook_event_name": "SessionStart", "session_id": CLAUDE_SESSION, "source": "startup"}))
        .unwrap();
    hook(json!({"hook_event_name": "UserPromptSubmit", "session_id": CLAUDE_SESSION, "prompt": "hello"}))
        .unwrap();
    hook(
        json!({"hook_event_name": "PreToolUse", "session_id": CLAUDE_SESSION,
                "tool_name": "Bash", "tool_input": {"command": "ls"}, "tool_use_id": "toolu_1"}),
    )
    .unwrap();
    assert_eq!(
        find(&m, info.id).status,
        Status::Idle,
        "a hook never moves the status"
    );
    let conversation = m
        .conversation_snapshot(info.id, None)
        .expect("the hooks built one");
    assert!(
        conversation
            .turns
            .iter()
            .flat_map(|t| &t.blocks)
            .any(|b| matches!(b, Block::ToolCall { name, .. } if name == "Bash")),
        "the PreToolUse reached the conversation: {conversation:?}"
    );

    hook(
        json!({"hook_event_name": "SubagentStart", "session_id": CLAUDE_SESSION,
                "agent_id": "agent-7", "agent_type": "Explore"}),
    )
    .unwrap();
    assert_eq!(find(&m, info.id).subagents.len(), 1, "a sub-agent row");
    assert_eq!(find(&m, info.id).status, Status::Idle);
    let signal = next_signal(&mut feed, "the SubagentStart feed signal", |s| {
        matches!(s.kind, WindowSignalKind::Hook { .. })
    })
    .await;
    assert_eq!(signal.window_id, info.id);
    assert_eq!(signal.pid, None);
    assert!(matches!(
        signal.kind,
        WindowSignalKind::Hook { kind: HookKind::SubagentStart, agent_id: Some(ref id) } if id == "agent-7"
    ));
    m.remove(info.id).unwrap();
}

#[tokio::test]
async fn a_headless_window_starts_no_transcript_reader() {
    let dir = tempfile::tempdir().unwrap();
    let claude = script(dir.path(), "claude", "exec sleep 30");
    let m = manager(&claude, &claude, |_| {});
    let info = create(&m, "w", spec(Runtime::Claude, dir.path()), "hello").await;
    let transcript = dir.path().join("transcript.jsonl");
    std::fs::write(&transcript, "").unwrap();
    m.handle_hook(
        info.id,
        HookSource::Claude,
        &json!({"hook_event_name": "SessionStart", "session_id": CLAUDE_SESSION,
                "source": "startup", "transcript_path": transcript.to_str().unwrap()}),
    )
    .unwrap();
    let reply = m.subscribe_conversation(info.id, None, None, CancellationToken::new());
    let DaemonMsg::ConversationSnapshot { conversation, .. } = reply else {
        panic!("a snapshot: {reply:?}");
    };
    assert_eq!(
        conversation.degraded, None,
        "not NoTranscriptPath, not anything"
    );
    assert!(!m.conversation_reader_running(info.id));
    m.unsubscribe_conversation(info.id, None);
    m.remove(info.id).unwrap();
}

const BG: &str = "<task-notification>agent done</task-notification>";
const BG2: &str = "<task-notification>second agent done</task-notification>";

/// One Claude turn's stream: `system/init`, one line of prose, a success `result`.
fn turn_lines(reply: &str) -> String {
    [
        format!(
            r#"{{"type":"system","subtype":"init","session_id":"{CLAUDE_SESSION}","model":"m"}}"#
        ),
        json!({"type": "assistant", "parent_tool_use_id": null,
               "message": {"content": [{"type": "text", "text": reply}]}})
        .to_string(),
        r#"{"type":"result","subtype":"success","is_error":false}"#.to_string(),
    ]
    .join("\n")
        + "\n"
}

/// Ruling T7-N1's binding on the manager: every real hook of a headless Claude window
/// goes to `observe_hook` with the window's one cursor, so each prompt keeps its own
/// reply — an unprompted background turn between two engine turns, and one Claude runs
/// after the engine's next turn was already sent.
#[tokio::test]
async fn a_headless_claude_prompt_hook_feeds_the_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let mut body = String::new();
    for (step, reply) in [
        ("a", "reply one"),
        ("b", "bg reply"),
        ("c", "bg reply 2"),
        ("d", "reply two"),
    ] {
        let file = dir.path().join(format!("{step}.jsonl"));
        std::fs::write(&file, turn_lines(reply)).unwrap();
        body += &format!("{}\ncat '{}'\n", gate(dir.path(), step), file.display());
    }
    body += "exec sleep 30";
    let claude = script(dir.path(), "claude", &body);
    let d =
        start_daemon_configured(false, |c| c.claude_bin = claude.to_str().unwrap().into()).await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    let mut feed = d.manager.signals();
    let info = create(&d.manager, "w", spec(Runtime::Claude, dir.path()), "first").await;
    let id = info.id;

    let mut hook = async |payload: serde_json::Value| {
        client
            .send(ClientMsg::HookEvent {
                window_id: id,
                source: HookSource::Claude,
                payload,
            })
            .await;
        client
            .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "hook"))
            .await;
    };
    let prompt = |text: &str| json!({"hook_event_name": "UserPromptSubmit", "session_id": CLAUDE_SESSION, "prompt": text});
    let stop = || json!({"hook_event_name": "Stop", "session_id": CLAUDE_SESSION});
    let turn_ended = async |feed: &mut tokio::sync::broadcast::Receiver<_>| {
        next_signal(feed, "a turn end", |s: &daemon::manager::WindowSignal| {
            matches!(
                s.kind,
                WindowSignalKind::Session(SessionEvent::TurnEnded { .. })
            )
        })
        .await;
    };

    hook(json!({"hook_event_name": "SessionStart", "session_id": CLAUDE_SESSION, "source": "startup"}))
        .await;
    hook(prompt("first")).await;
    open_gate(dir.path(), "a");
    turn_ended(&mut feed).await;
    hook(stop()).await;

    hook(prompt(BG)).await;
    open_gate(dir.path(), "b");
    turn_ended(&mut feed).await;
    hook(stop()).await;

    d.manager.headless_send(id, "second").await.unwrap();
    hook(prompt(BG2)).await;
    open_gate(dir.path(), "c");
    turn_ended(&mut feed).await;
    hook(stop()).await;

    hook(prompt("second")).await;
    open_gate(dir.path(), "d");
    turn_ended(&mut feed).await;
    hook(stop()).await;

    let conversation = d.manager.conversation_snapshot(id, None).unwrap();
    assert_eq!(conversation.degraded, None);
    let text = |blocks: &[Block]| {
        blocks
            .iter()
            .find_map(|b| match b {
                Block::Text { text } => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_default()
    };
    let pairs: Vec<(String, String)> = conversation
        .turns
        .chunks(2)
        .map(|pair| {
            assert_eq!(pair[0].role, Role::User, "{conversation:?}");
            assert_eq!(pair[1].role, Role::Assistant, "{conversation:?}");
            (text(&pair[0].blocks), text(&pair[1].blocks))
        })
        .collect();
    let want: Vec<(String, String)> = [
        ("first", "reply one"),
        (BG, "bg reply"),
        (BG2, "bg reply 2"),
        ("second", "reply two"),
    ]
    .iter()
    .map(|(p, r)| (p.to_string(), r.to_string()))
    .collect();
    assert_eq!(pairs, want);
    // The feed saw no exit: one Claude process served every turn.
    assert!(!matches!(feed.try_recv(), Ok(ref s) if is_exit(s)));
}
