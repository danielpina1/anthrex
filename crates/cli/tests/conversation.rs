//! Task M6.5.10's end-to-end proof that the committed golden fixture, the Claude parser,
//! the enricher and the reader agree: fake-agent writes lines copied from
//! `crates/daemon/tests/fixtures/transcripts/claude-2.1.278.jsonl` — read from that file
//! at test time, not retyped — and fires the matching hooks through the real `anthrex
//! hook`, while a client subscribed over the real socket watches the conversation.
//!
//! It runs twice, once with each transcript line written before the hook it describes and
//! once after, and both must end in the same conversation: task M6.5.8's pending buffer
//! is what makes enrichment independent of arrival order, and this pins it at the wire.

mod support;

use daemon::conversation::watch::TRANSCRIPT_POLL;
use daemon::transcript::reader::TRANSCRIPT_READ_TIMEOUT;
use proto::{Block, ClientMsg, Conversation, DaemonMsg, Role, Runtime, ToolState};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use support::TestDaemon;

const FIXTURE: &str = include_str!("../../daemon/tests/fixtures/transcripts/claude-2.1.278.jsonl");

/// fake-agent's own bound on one hook step (`STEP_TIMEOUT`, `crates/fake-agent/src/main.rs`).
/// A binary's constant, so the coupling is this comment, the way `hook_command.rs`'s
/// `LIMIT` couples to `HOOK_DEADLINE` (`docs/timing-budgets.md`).
const FAKE_AGENT_STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// The pause between a transcript line and its hook: two poll intervals, so the reader
/// normally sees the first of the pair on its own before the second arrives. This is what
/// makes the two orders differ at the daemon; the assertion holds whatever the timing.
const BETWEEN: Duration = Duration::from_millis(2 * TRANSCRIPT_POLL.as_millis() as u64);

/// The fixture's 1-based line `n`, parsed.
fn fixture_line(n: usize) -> Value {
    serde_json::from_str(
        FIXTURE
            .lines()
            .nth(n - 1)
            .expect("the fixture has that line"),
    )
    .expect("every fixture line is JSON")
}

fn session_id() -> String {
    fixture_line(1)["sessionId"].as_str().unwrap().to_owned()
}

fn prompt_text() -> String {
    fixture_line(8)["message"]["content"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn prose() -> String {
    fixture_line(31)["message"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn tool_use() -> Value {
    fixture_line(32)["message"]["content"][0].clone()
}

fn tool_detail() -> String {
    let result = &fixture_line(34)["message"]["content"][0];
    assert_eq!(result["tool_use_id"], tool_use()["id"]);
    result["content"].as_str().unwrap().to_owned()
}

#[derive(Clone, Copy, Debug)]
enum Order {
    TranscriptFirst,
    HookFirst,
}

/// One transcript line and the hook describing it, in `order`, with a pause between.
fn pair(order: Order, line: usize, hook: &str, payload: Value) -> Vec<Value> {
    let transcript = json!({"transcript": fixture_line(line)});
    let hook = json!({"hook": hook, "payload": payload});
    let wait = json!({"wait_ms": BETWEEN.as_millis() as u64});
    match order {
        Order::TranscriptFirst => vec![transcript, wait.clone(), hook, wait],
        Order::HookFirst => vec![hook, wait.clone(), transcript, wait],
    }
}

fn script(order: Order) -> Vec<Value> {
    let session = session_id();
    let call = tool_use();
    let mut steps = vec![
        // Held until the test has subscribed, so the reader is running throughout.
        json!({"read_line": true}),
        // The fixture's first line, a bookkeeping record: detection alone.
        json!({"transcript": fixture_line(1)}),
        json!({"hook": "SessionStart", "payload": {"session_id": session}}),
    ];
    steps.extend(pair(
        order,
        8,
        "UserPromptSubmit",
        json!({"session_id": session, "prompt": prompt_text()}),
    ));
    steps.push(json!({"transcript": fixture_line(31)}));
    steps.extend(pair(
        order,
        32,
        "PreToolUse",
        json!({"session_id": session, "tool_name": call["name"], "tool_use_id": call["id"],
            "tool_input": call["input"]}),
    ));
    steps.extend(pair(
        order,
        34,
        "PostToolUse",
        json!({"session_id": session, "tool_name": call["name"], "tool_use_id": call["id"],
            "tool_input": call["input"],
            "tool_response": {"stdout": "[workspace]", "stderr": "", "interrupted": false}}),
    ));
    steps.push(json!({"hook": "Stop", "payload": {"session_id": session}}));
    steps.push(json!({"read_line": true}));
    steps
}

/// How many hook steps `script` has, for the bound below.
const HOOKS: u32 = 5;

/// Whether the conversation shows everything the fixture adds on top of the hooks.
fn enriched(c: &Conversation) -> bool {
    let Some(assistant) = c.turns.iter().find(|t| t.role == Role::Assistant) else {
        return false;
    };
    let prose_first =
        matches!(assistant.blocks.first(), Some(Block::Text { text }) if *text == prose());
    let detail = assistant.blocks.iter().any(|b| {
        matches!(b, Block::ToolCall { id: Some(id), state: ToolState::Ok, result: Some(r), .. }
            if id.as_str() == tool_use()["id"] && r.detail.as_deref() == Some(tool_detail().as_str()))
    });
    prose_first && detail && c.degraded.is_none()
}

/// Wall-clock fields differ between runs by construction; everything else must not.
fn comparable(mut c: Conversation) -> Conversation {
    c.rev = 0;
    for turn in &mut c.turns {
        turn.at_unix_secs = 0;
        for block in &mut turn.blocks {
            if let Block::ToolCall { duration_ms, .. } = block {
                *duration_ms = None;
            }
        }
    }
    c
}

fn run(order: Order) -> Conversation {
    let dir = tempfile::tempdir().unwrap();
    let transcript = dir.path().join("session.jsonl");
    let daemon = TestDaemon::start_configured(&script(order), |command| {
        command.env("FAKE_AGENT_TRANSCRIPT", &transcript);
    });
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "enriched");
    client.send(ClientMsg::SubscribeConversation {
        window_id: id,
        agent_id: None,
        from_rev: None,
    });
    client.receive(|m| matches!(m, DaemonMsg::ConversationSnapshot { .. }));
    client.input(id, b"go\r");

    // Derived from what it waits on (docs/timing-budgets.md standing rule 1): the
    // brief's HANDSHAKE_TIMEOUT + TRANSCRIPT_POLL + TRANSCRIPT_READ_TIMEOUT + 5s, plus
    // the script's own run time, which the brief's bound did not have to cover: every
    // hook step bounded by fake-agent's STEP_TIMEOUT, and every pause.
    let bound = proto::HANDSHAKE_TIMEOUT
        + TRANSCRIPT_POLL
        + TRANSCRIPT_READ_TIMEOUT
        + Duration::from_secs(5)
        + FAKE_AGENT_STEP_TIMEOUT * HOOKS
        + BETWEEN * 6;
    let deadline = Instant::now() + bound;
    let mut last = None;
    loop {
        // Re-subscribing to a key the connection holds answers with a fresh snapshot
        // and counts no second viewer.
        client.send(ClientMsg::SubscribeConversation {
            window_id: id,
            agent_id: None,
            from_rev: None,
        });
        let DaemonMsg::ConversationSnapshot { conversation, .. } =
            client.receive(|m| matches!(m, DaemonMsg::ConversationSnapshot { .. }))
        else {
            unreachable!()
        };
        if enriched(&conversation)
            && conversation
                .turns
                .iter()
                .all(|t| t.state == proto::TurnState::Complete)
        {
            return comparable(conversation);
        }
        assert!(
            Instant::now() < deadline,
            "{order:?}: not enriched within {bound:?}: {last:#?}"
        );
        last = Some(conversation);
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_fake_agent_transcript_enriches_the_conversation() {
    let transcript_first = run(Order::TranscriptFirst);
    let hook_first = run(Order::HookFirst);
    assert_eq!(transcript_first.session_id, Some(session_id()));
    assert_eq!(
        transcript_first, hook_first,
        "the arrival order changed the conversation"
    );
}

/// A Claude prompt line in the fixture's shape (its line 8): a `sessionId`, which
/// detection keys on, and `origin.kind: "human"`, which makes it a prompt.
fn prompt_line(session: &str, text: &str) -> Value {
    json!({"type":"user","sessionId":session,"isSidechain":false,"origin":{"kind":"human"},
        "message":{"role":"user","content":text}})
}

/// An assistant prose line in the fixture's shape (its line 31).
fn reply_line(session: &str, text: &str) -> Value {
    json!({"type":"assistant","sessionId":session,"isSidechain":false,
        "message":{"role":"assistant","content":[{"type":"text","text":text}]}})
}

/// Review F2: session A runs two turns, then `/clear` starts session B in a new file whose
/// prompts count from 0, and B runs two turns. Session A's lines are written by
/// fake-agent as it goes; session B's file is written before B's hooks fire, so every one
/// of B's records is read ahead of its `UserPromptSubmit` and waits in task M6.5.8's
/// pending buffer across the switch. Returns each `Assistant` turn's prose, in order.
fn clear_run(b_first_prompt: &str) -> (Vec<Vec<String>>, Option<proto::DegradeReason>) {
    let dir = tempfile::tempdir().unwrap();
    let a_path = dir.path().join("a.jsonl");
    let b_path = dir.path().join("b.jsonl");
    let b_lines = [
        prompt_line("sess-B", b_first_prompt),
        reply_line("sess-B", "B reply 1"),
        prompt_line("sess-B", "second B"),
        reply_line("sess-B", "B reply 2"),
    ];
    let b_text: String = b_lines.iter().map(|l| format!("{l}\n")).collect();
    std::fs::write(&b_path, b_text).unwrap();

    let hook = |name: &str, payload: Value| json!({"hook": name, "payload": payload});
    let prompt = |session: &str, text: &str| {
        hook(
            "UserPromptSubmit",
            json!({"session_id": session, "prompt": text}),
        )
    };
    let stop = |session: &str| hook("Stop", json!({"session_id": session}));
    let mut script = vec![
        json!({"read_line": true}),
        hook("SessionStart", json!({"session_id": "sess-A"})),
    ];
    for (text, reply) in [("fix the bug", "A reply 1"), ("second A", "A reply 2")] {
        script.push(json!({"transcript": prompt_line("sess-A", text)}));
        script.push(json!({"transcript": reply_line("sess-A", reply)}));
        script.push(prompt("sess-A", text));
        script.push(stop("sess-A"));
    }
    script.push(json!({"wait_ms": BETWEEN.as_millis() as u64}));
    script.push(hook(
        "SessionStart",
        json!({"session_id": "sess-B", "source": "clear",
            "transcript_path": b_path.to_str().unwrap()}),
    ));
    script.push(json!({"wait_ms": BETWEEN.as_millis() as u64}));
    for text in [b_first_prompt, "second B"] {
        script.push(prompt("sess-B", text));
        script.push(stop("sess-B"));
    }
    script.push(json!({"read_line": true}));

    let daemon = TestDaemon::start_configured(&script, |command| {
        command.env("FAKE_AGENT_TRANSCRIPT", &a_path);
    });
    let mut client = daemon.client();
    let id = client.create(Runtime::Claude, "cleared");
    client.send(ClientMsg::SubscribeConversation {
        window_id: id,
        agent_id: None,
        from_rev: None,
    });
    client.receive(|m| matches!(m, DaemonMsg::ConversationSnapshot { .. }));
    client.input(id, b"go\r");

    // Nine hook steps at fake-agent's STEP_TIMEOUT, two pauses, one poll and one pass,
    // plus the same 5 s slack the test above takes from the brief.
    let bound = FAKE_AGENT_STEP_TIMEOUT * 9
        + BETWEEN * 2
        + TRANSCRIPT_POLL
        + TRANSCRIPT_READ_TIMEOUT
        + Duration::from_secs(5);
    let deadline = Instant::now() + bound;
    loop {
        client.send(ClientMsg::SubscribeConversation {
            window_id: id,
            agent_id: None,
            from_rev: None,
        });
        let DaemonMsg::ConversationSnapshot { conversation, .. } =
            client.receive(|m| matches!(m, DaemonMsg::ConversationSnapshot { .. }))
        else {
            unreachable!()
        };
        let replies: Vec<Vec<String>> = conversation
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
        let settled = conversation.session_id.as_deref() == Some("sess-B")
            && replies.len() == 4
            && replies.iter().all(|r| !r.is_empty());
        if settled || Instant::now() >= deadline {
            return (replies, conversation.degraded);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn expected_replies() -> Vec<Vec<String>> {
    ["A reply 1", "A reply 2", "B reply 1", "B reply 2"]
        .iter()
        .map(|r| vec![r.to_string()])
        .collect()
}

#[test]
fn a_cleared_session_enriches_both_sessions_turns() {
    let (replies, degraded) = clear_run("hello B");
    assert_eq!(replies, expected_replies());
    assert_eq!(degraded, None);
}

/// B's first prompt repeats A's: without a per-session base, B's ordinal 0 would line up
/// with A's first turn and put "B reply 1" there.
#[test]
fn a_cleared_session_repeating_a_prompt_keeps_each_reply_on_its_own_turn() {
    let (replies, degraded) = clear_run("fix the bug");
    assert_eq!(replies, expected_replies());
    assert_eq!(degraded, None);
}
