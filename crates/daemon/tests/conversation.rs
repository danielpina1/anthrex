//! Task M6.5.10: the conversation subscription, end to end inside one process — a real
//! manager, a real socket, the real server, and hooks driven straight into the manager
//! (`crates/cli/tests/conversation.rs` drives them through the real `anthrex` binary and
//! fake-agent instead).
//!
//! Every wait is a deadline loop on the condition it needs, bounded by the constants the
//! code runs against (`docs/timing-budgets.md` standing rule 1).

mod support;

use daemon::conversation::watch::TRANSCRIPT_POLL;
use daemon::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use proto::conversation::{GONE_SUBAGENT_UNKNOWN, GONE_WINDOW_REMOVED, GONE_WINDOW_UNKNOWN};
use proto::{
    Block, ClientMsg, DaemonMsg, DegradeReason, HookSource, PROTO_VERSION, Role, ToolState,
    TurnPatch,
};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use support::{
    Client, TestDaemon, claude_window, shell_spec, start_daemon, start_daemon_configured,
};

/// Scheduling slack on top of a derived bound: the reader and the connection task are
/// ordinary tokio tasks on a test runtime shared with everything else in the test.
const SLACK: Duration = Duration::from_secs(2);

/// The longest a subscribed window's reader takes to act on the file: one poll interval,
/// one full pass (`conversation/watch.rs`, `transcript/reader.rs`), plus slack.
fn reader_bound() -> Duration {
    TRANSCRIPT_POLL + TRANSCRIPT_READ_TIMEOUT + SLACK
}

fn hook(d: &TestDaemon, id: u32, payload: Value) {
    d.manager
        .handle_hook(id, HookSource::Claude, &payload)
        .unwrap();
}

fn prompt(text: &str) -> Value {
    json!({"hook_event_name":"UserPromptSubmit","session_id":"sess-1","prompt":text})
}

fn pre_tool(id: &str) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"sess-1","tool_name":"Read",
        "tool_use_id":id,"tool_input":{"file_path":"/repo/Cargo.toml"}})
}

fn post_tool(id: &str) -> Value {
    json!({"hook_event_name":"PostToolUse","session_id":"sess-1","tool_name":"Read",
        "tool_use_id":id,"tool_input":{"file_path":"/repo/Cargo.toml"},
        "tool_response":"[workspace]"})
}

fn stop() -> Value {
    json!({"hook_event_name":"Stop","session_id":"sess-1"})
}

async fn connect(d: &TestDaemon) -> Client {
    let (client, welcome) = Client::connect(d, PROTO_VERSION).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }), "{welcome:?}");
    client
}

async fn subscribe(c: &mut Client, window_id: u32, agent_id: Option<&str>) -> DaemonMsg {
    c.send(ClientMsg::SubscribeConversation {
        window_id,
        agent_id: agent_id.map(str::to_owned),
        from_rev: None,
    })
    .await;
    c.recv_until(|m| {
        matches!(
            m,
            DaemonMsg::ConversationSnapshot { .. }
                | DaemonMsg::ConversationGone { .. }
                | DaemonMsg::Error { .. }
        )
    })
    .await
}

async fn subscribed_rev(c: &mut Client, window_id: u32) -> u64 {
    match subscribe(c, window_id, None).await {
        DaemonMsg::ConversationSnapshot { conversation, .. } => conversation.rev,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

async fn next_delta(c: &mut Client) -> DaemonMsg {
    c.recv_until(|m| matches!(m, DaemonMsg::ConversationDelta { .. }))
        .await
}

/// Waits for `pred` on the window's root conversation, as the manager holds it.
async fn wait_root(
    d: &TestDaemon,
    id: u32,
    within: Duration,
    what: &str,
    pred: impl Fn(&proto::Conversation) -> bool,
) -> proto::Conversation {
    let deadline = Instant::now() + within;
    loop {
        if let Some(c) = d.manager.conversation_snapshot(id, None)
            && pred(&c)
        {
            return c;
        }
        assert!(
            Instant::now() < deadline,
            "{what} within {within:?}: {:?}",
            d.manager.conversation_snapshot(id, None)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_hook_builds_a_conversation_a_client_can_read() {
    let d = start_daemon().await;
    let id = claude_window(&d, "hooks").await;
    for payload in [
        prompt("read the manifest"),
        pre_tool("tu-1"),
        post_tool("tu-1"),
        stop(),
    ] {
        hook(&d, id, payload);
    }
    let conversation = d.manager.conversation_snapshot(id, None).unwrap();
    assert_eq!(conversation.turns.len(), 2, "{conversation:?}");
    assert_eq!(conversation.turns[0].role, Role::User);
    let assistant = &conversation.turns[1];
    assert_eq!(assistant.role, Role::Assistant);
    let calls: Vec<_> = assistant
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::ToolCall { state, summary, .. } => Some((*state, summary.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(calls.len(), 1, "{assistant:?}");
    assert_eq!(calls[0].0, ToolState::Ok);
    assert!(!calls[0].1.is_empty());
}

#[tokio::test]
async fn a_shell_window_has_an_empty_degraded_conversation() {
    let d = start_daemon().await;
    let id = d
        .manager
        .create(shell_spec("sh"), std::env::temp_dir(), None, 80, 24)
        .await
        .unwrap()
        .id;
    let mut c = connect(&d).await;
    match subscribe(&mut c, id, None).await {
        DaemonMsg::ConversationSnapshot {
            window_id,
            agent_id,
            conversation,
        } => {
            assert_eq!((window_id, agent_id.as_deref()), (id, None));
            assert_eq!(conversation.window_id, id);
            assert!(conversation.turns.is_empty());
            assert_eq!(conversation.degraded, Some(DegradeReason::NoTranscriptPath));
        }
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_window_answers_conversation_gone() {
    let d = start_daemon().await;
    let mut c = connect(&d).await;
    match subscribe(&mut c, 999, None).await {
        DaemonMsg::ConversationGone {
            window_id, reason, ..
        } => {
            assert_eq!(window_id, 999);
            assert_eq!(reason, GONE_WINDOW_UNKNOWN);
        }
        other => panic!("expected ConversationGone, got {other:?}"),
    }
}

#[tokio::test]
async fn an_unknown_subagent_answers_conversation_gone() {
    let d = start_daemon().await;
    let id = claude_window(&d, "no-such-agent").await;
    let mut c = connect(&d).await;
    match subscribe(&mut c, id, Some("nope")).await {
        DaemonMsg::ConversationGone {
            window_id,
            agent_id,
            reason,
        } => {
            assert_eq!((window_id, agent_id.as_deref()), (id, Some("nope")));
            assert_eq!(reason, GONE_SUBAGENT_UNKNOWN);
        }
        other => panic!("expected ConversationGone, got {other:?}"),
    }
    // It subscribed nothing: no reader was started for it.
    assert!(!d.manager.conversation_reader_running(id));
}

#[tokio::test]
async fn removing_a_window_ends_its_conversations() {
    let d = start_daemon().await;
    let id = claude_window(&d, "removed").await;
    hook(&d, id, prompt("hello"));
    let mut c = connect(&d).await;
    subscribed_rev(&mut c, id).await;
    d.manager.remove(id).unwrap();
    let gone = tokio::time::timeout(
        TRANSCRIPT_POLL * 4,
        c.recv_until(|m| matches!(m, DaemonMsg::ConversationGone { .. })),
    )
    .await
    .expect("no ConversationGone within TRANSCRIPT_POLL * 4");
    match gone {
        DaemonMsg::ConversationGone {
            window_id,
            agent_id,
            reason,
        } => {
            assert_eq!((window_id, agent_id), (id, None));
            assert_eq!(reason, GONE_WINDOW_REMOVED);
        }
        other => unreachable!("{other:?}"),
    }
}

/// Decision 9: with no subscriber the transcript is never opened. The path sits in a
/// directory nobody may enter, so any attempt to open it degrades `Unreadable`; as root,
/// which may enter anything, the path does not exist instead, which degrades the same way.
#[tokio::test]
async fn the_reader_runs_only_while_subscribed() {
    let d = start_daemon().await;
    let id = claude_window(&d, "lazy").await;
    let dir = tempfile::tempdir().unwrap();
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    let as_root = unsafe { libc::geteuid() } == 0;
    let path = if as_root {
        dir.path().join("absent.jsonl")
    } else {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        locked.join("t.jsonl")
    };
    hook(
        &d,
        id,
        json!({"hook_event_name":"SessionStart","session_id":"sess-1",
            "transcript_path":path.to_str().unwrap()}),
    );

    let quiet_until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < quiet_until {
        let c = d.manager.conversation_snapshot(id, None).unwrap();
        assert_ne!(
            c.degraded,
            Some(DegradeReason::Unreadable),
            "read unsubscribed"
        );
        assert!(!d.manager.conversation_reader_running(id));
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let mut c = connect(&d).await;
    subscribed_rev(&mut c, id).await;
    wait_root(&d, id, reader_bound(), "Unreadable once subscribed", |c| {
        c.degraded == Some(DegradeReason::Unreadable)
    })
    .await;

    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
}

/// A delta whose two revisions are the same number is a shape this project has shipped
/// before, so `from_rev != to_rev` is asserted outright. The three hooks run back to back
/// with no `.await` between them, and this test's runtime is single-threaded, so the
/// connection's task cannot run until all three have landed: it answers them with one
/// delta, computed from current state, rather than three.
#[tokio::test(flavor = "current_thread")]
async fn a_delta_follows_a_snapshot_on_the_same_connection() {
    let d = start_daemon().await;
    let id = claude_window(&d, "delta").await;
    let mut c = connect(&d).await;
    let r = subscribed_rev(&mut c, id).await;

    hook(&d, id, prompt("read the manifest"));
    hook(&d, id, pre_tool("tu-1"));
    hook(&d, id, post_tool("tu-1"));

    match next_delta(&mut c).await {
        DaemonMsg::ConversationDelta {
            window_id,
            agent_id,
            from_rev,
            to_rev,
            turns,
            ..
        } => {
            assert_eq!((window_id, agent_id), (id, None));
            assert_ne!(from_rev, to_rev);
            assert_eq!(from_rev, r);
            assert_eq!(to_rev, r + 3);
            assert!(turns.iter().any(|p| matches!(p, TurnPatch::Upsert(_))));
        }
        other => unreachable!("{other:?}"),
    }
}

#[tokio::test]
async fn two_clients_see_the_same_conversation() {
    let d = start_daemon().await;
    let id = claude_window(&d, "shared").await;
    let mut a = connect(&d).await;
    let mut b = connect(&d).await;
    let ra = subscribed_rev(&mut a, id).await;
    let rb = subscribed_rev(&mut b, id).await;
    assert_eq!(ra, rb);

    hook(&d, id, prompt("for both"));
    let expected = d.manager.conversation_snapshot(id, None).unwrap().rev;
    for client in [&mut a, &mut b] {
        match next_delta(client).await {
            DaemonMsg::ConversationDelta {
                from_rev, to_rev, ..
            } => {
                assert_eq!((from_rev, to_rev), (ra, expected));
            }
            other => unreachable!("{other:?}"),
        }
    }
}

/// Task M6.5.6's F4, settled by carrying `session_id` on the delta: a client holding a
/// snapshot from before the `SessionStart`, and following only deltas after it, learns
/// the new session id.
#[tokio::test]
async fn a_new_session_id_reaches_a_client_that_only_follows_deltas() {
    let d = start_daemon().await;
    let id = claude_window(&d, "session").await;
    hook(&d, id, prompt("before the session"));
    let mut c = connect(&d).await;
    match subscribe(&mut c, id, None).await {
        DaemonMsg::ConversationSnapshot { conversation, .. } => {
            assert_eq!(conversation.session_id, None)
        }
        other => panic!("expected a snapshot, got {other:?}"),
    }
    hook(
        &d,
        id,
        json!({"hook_event_name":"SessionStart","session_id":"sess-42"}),
    );
    match next_delta(&mut c).await {
        DaemonMsg::ConversationDelta {
            session_id, turns, ..
        } => {
            assert_eq!(session_id.as_deref(), Some("sess-42"));
            assert!(turns.is_empty(), "nothing but the session changed");
        }
        other => unreachable!("{other:?}"),
    }
}

/// Transcript records enrich the root conversation only (task M6.5.8's routing): the
/// file the reader tails is the root's, and a sub-agent's transcript is a separate file
/// this milestone does not read. The sub-agent's conversation stays hook-only.
#[tokio::test]
async fn a_transcript_enriches_the_root_conversation_only() {
    let d = start_daemon().await;
    let id = claude_window(&d, "enriched").await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let line = |v: Value| format!("{v}\n");
    let transcript = line(json!({"type":"user","sessionId":"sess-1",
            "origin":{"kind":"human"},"message":{"role":"user","content":"go"}}))
        + &line(json!({"type":"assistant","sessionId":"sess-1",
            "message":{"role":"assistant","content":[{"type":"text","text":"on it"}]}}));
    std::fs::write(&path, transcript).unwrap();

    hook(
        &d,
        id,
        json!({"hook_event_name":"SessionStart","session_id":"sess-1",
            "transcript_path":path.to_str().unwrap()}),
    );
    hook(&d, id, prompt("go"));
    hook(
        &d,
        id,
        json!({"hook_event_name":"SubagentStart","session_id":"sess-1",
            "agent_id":"agent-1","agent_type":"Explore"}),
    );
    let mut c = connect(&d).await;
    subscribed_rev(&mut c, id).await;
    wait_root(&d, id, reader_bound(), "the assistant's prose", |c| {
        c.turns.iter().any(|t| {
            t.role == Role::Assistant
                && matches!(t.blocks.first(), Some(Block::Text { text }) if text == "on it")
        })
    })
    .await;
    let sub = d
        .manager
        .conversation_snapshot(id, Some("agent-1"))
        .unwrap();
    assert!(
        sub.turns
            .iter()
            .flat_map(|t| &t.blocks)
            .all(|b| !matches!(b, Block::Text { text } if text == "on it")),
        "{sub:?}"
    );
}

/// The linger is `conversation.linger_secs`; at 0 the reader stops at its next step
/// after the last subscriber leaves. Leaving here is a disconnect, so this also proves
/// the connection gives its subscriptions back, and a second `SubscribeConversation` for
/// a key already held did not count a second viewer that nothing would ever release.
#[tokio::test]
async fn a_disconnected_subscriber_stops_the_reader_after_the_linger() {
    let d = start_daemon_configured(true, |config| config.conversation.linger_secs = 0).await;
    let id = claude_window(&d, "linger").await;
    let mut c = connect(&d).await;
    subscribed_rev(&mut c, id).await;
    subscribed_rev(&mut c, id).await;
    assert!(d.manager.conversation_reader_running(id));
    drop(c);

    let bound = Duration::from_secs(0) + reader_bound();
    let deadline = Instant::now() + bound;
    while d.manager.conversation_reader_running(id) {
        assert!(
            Instant::now() < deadline,
            "reader still running after {bound:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // And a new subscriber starts it again.
    let mut again = connect(&d).await;
    subscribed_rev(&mut again, id).await;
    assert!(d.manager.conversation_reader_running(id));
}

/// `UnsubscribeConversation` gives the viewer back the same way a disconnect does.
#[tokio::test]
async fn unsubscribing_stops_the_reader_after_the_linger() {
    let d = start_daemon_configured(true, |config| config.conversation.linger_secs = 0).await;
    let id = claude_window(&d, "unsub").await;
    let mut c = connect(&d).await;
    subscribed_rev(&mut c, id).await;
    c.send(ClientMsg::UnsubscribeConversation {
        window_id: id,
        agent_id: None,
    })
    .await;
    let deadline = Instant::now() + reader_bound();
    while d.manager.conversation_reader_running(id) {
        assert!(Instant::now() < deadline, "reader still running");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
