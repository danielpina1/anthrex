//! Task M6.5.10 fix round 1 (F1): a conversation, or a change to one, that does not fit
//! in one frame must cost the client that subscription and nothing more. Before the fix
//! the writer broke on `CodecError::TooLarge` and the client's whole connection ended.
//! Each test reproduces one of the review's three inputs over a real socket and then
//! checks the same connection still answers `ListWindows`.

mod support;

use daemon::conversation::watch::TRANSCRIPT_POLL;
use daemon::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use proto::conversation::GONE_TOO_LARGE;
use proto::{Block, ClientMsg, DaemonMsg, HookSource, PROTO_VERSION};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use support::{Client, TestDaemon, claude_window, start_daemon, start_daemon_configured};

fn hook(d: &TestDaemon, id: u32, payload: Value) {
    d.manager
        .handle_hook(id, HookSource::Claude, &payload)
        .unwrap();
}

async fn connect(d: &TestDaemon) -> Client {
    let (client, welcome) = Client::connect(d, PROTO_VERSION).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }), "{welcome:?}");
    client
}

async fn subscribe(c: &mut Client, window_id: u32) -> DaemonMsg {
    c.send(ClientMsg::SubscribeConversation {
        window_id,
        agent_id: None,
        from_rev: None,
    })
    .await;
    c.recv_until(|m| {
        matches!(
            m,
            DaemonMsg::ConversationSnapshot { .. } | DaemonMsg::ConversationGone { .. }
        )
    })
    .await
}

async fn still_answers(c: &mut Client, window_id: u32) {
    c.send(ClientMsg::ListWindows).await;
    match c
        .recv_until(|m| matches!(m, DaemonMsg::WindowsChanged { .. }))
        .await
    {
        DaemonMsg::WindowsChanged { windows } => {
            assert!(windows.iter().any(|w| w.id == window_id), "{windows:?}")
        }
        other => unreachable!("{other:?}"),
    }
}

fn assert_too_large(message: &DaemonMsg, id: u32) {
    match message {
        DaemonMsg::ConversationGone {
            window_id,
            agent_id,
            reason,
        } => {
            assert_eq!((*window_id, agent_id.as_deref()), (id, None));
            assert_eq!(reason, GONE_TOO_LARGE);
        }
        other => panic!("expected ConversationGone too large, got {other:?}"),
    }
}

fn pre_write(n: usize, input: Value) -> Value {
    json!({"hook_event_name":"PreToolUse","session_id":"s","tool_name":"Write",
        "tool_use_id":format!("tu-{n}"),"tool_input":input})
}

/// The review's first input: `max_bytes` at 32 MiB (the config refuses that now, but the
/// manager takes whatever it is given) and sixteen-plus 1 MiB prompts. Client A, already
/// subscribed, gets the coalesced delta that cannot fit; client B's snapshot cannot fit.
/// Both lose the subscription, keep the connection, and give their viewer back.
#[tokio::test(flavor = "current_thread")]
async fn a_conversation_past_a_frame_ends_the_subscription_not_the_connection() {
    let d = start_daemon_configured(true, |config| {
        config.conversation.max_bytes = 32 * 1024 * 1024;
        config.conversation.linger_secs = 0;
    })
    .await;
    let id = claude_window(&d, "huge").await;
    let mut a = connect(&d).await;
    assert!(matches!(
        subscribe(&mut a, id).await,
        DaemonMsg::ConversationSnapshot { .. }
    ));

    // No `.await` between the hooks: on this single-threaded runtime client A's task
    // answers them all with one delta.
    let mib = "p".repeat(1024 * 1024);
    for n in 0..17 {
        hook(
            &d,
            id,
            json!({"hook_event_name":"UserPromptSubmit","session_id":"s",
                "prompt":format!("{n}{mib}")}),
        );
    }
    let gone = a
        .recv_until(|m| {
            matches!(
                m,
                DaemonMsg::ConversationGone { .. } | DaemonMsg::ConversationDelta { .. }
            )
        })
        .await;
    assert_too_large(&gone, id);
    still_answers(&mut a, id).await;

    let mut b = connect(&d).await;
    assert_too_large(&subscribe(&mut b, id).await, id);
    still_answers(&mut b, id).await;

    // Neither holds a viewer any more, so the reader stops after its zero linger.
    let bound = TRANSCRIPT_POLL + TRANSCRIPT_READ_TIMEOUT + Duration::from_secs(2);
    let deadline = Instant::now() + bound;
    while d.manager.conversation_reader_running(id) {
        assert!(Instant::now() < deadline, "a viewer was never given back");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn tool_inputs_after(inputs: Vec<Value>) -> Vec<Option<Value>> {
    let d = start_daemon().await;
    let id = claude_window(&d, "inputs").await;
    hook(
        &d,
        id,
        json!({"hook_event_name":"UserPromptSubmit","session_id":"s","prompt":"go"}),
    );
    for (n, input) in inputs.into_iter().enumerate() {
        hook(&d, id, pre_write(n, input));
    }
    let mut c = connect(&d).await;
    let message = subscribe(&mut c, id).await;
    still_answers(&mut c, id).await;
    let DaemonMsg::ConversationSnapshot { conversation, .. } = message else {
        panic!("expected a snapshot, got {message:?}");
    };
    conversation
        .turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::ToolCall { input, summary, .. } => {
                assert!(!summary.is_empty());
                Some(input.clone())
            }
            _ => None,
        })
        .collect()
}

/// The review's second input, at the default config: three `Write`s of 6 MiB each in one
/// turn. Their inputs are not stored, so the snapshot is small and arrives.
#[tokio::test]
async fn huge_write_inputs_leave_a_snapshot_that_arrives() {
    let content = "w".repeat(6 * 1024 * 1024);
    let inputs = (0..3)
        .map(|n| json!({"file_path": format!("/repo/f{n}.rs"), "content": content}))
        .collect();
    assert_eq!(tool_inputs_after(inputs).await, vec![None, None, None]);
}

/// The review's third input, at the default config: a float array whose JSON is 7.6 MB
/// and whose encoding is 17.1 MB. `byte_size` now measures the encoding, so the input is
/// over `max_result_bytes` and is not stored.
#[tokio::test]
async fn a_float_array_input_leaves_a_snapshot_that_arrives() {
    let input = json!({"v": vec![0.5; 1_900_000]});
    assert_eq!(tool_inputs_after(vec![input]).await, vec![None]);
}
