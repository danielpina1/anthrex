//! The mapping driven into a real milestone 6.5 `ConversationSet` from M8a.1's recorded
//! sessions.

use super::super::*;
use crate::conversation::{Caps, ConversationSet};
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::codex_stream;
use crate::headless::test_support::{CLAUDE_HOOKS, CLAUDE_INPUT, CLAUDE_STREAM, CODEX_EXEC, lines};
use proto::{Block, HookSource, Role, ToolState, TurnState};
use serde_json::{Value, json};
use std::time::Instant;

/// What the manager does with one `ConversationInput` (decision 27): hooks through
/// `on_hook`, then records through `enrich`.
fn apply(set: &mut ConversationSet, runtime: Runtime, input: ConversationInput) {
    for hook in &input.hooks {
        set.on_hook(
            runtime,
            hook,
            None,
            1_700_000_000,
            Instant::now(),
            Caps::default(),
        );
    }
    set.enrich(&input.records, Caps::default());
}

/// The first `codex exec` run of the fixture (the second is a separate session).
fn codex_first_run() -> Vec<&'static str> {
    let all = lines(CODEX_EXEC);
    let second = all
        .iter()
        .skip(1)
        .position(|l| l.contains("thread.started"))
        .unwrap()
        + 1;
    all[..second].to_vec()
}

/// The prompt of the fixture's first run, from its meta's command.
const CODEX_PROMPT: &str = "Create a.txt containing the line one, then run: git add a.txt && git commit -m \"add a\"  Reply done.";

fn tool_calls(blocks: &[Block]) -> Vec<&Block> {
    blocks
        .iter()
        .filter(|b| matches!(b, Block::ToolCall { .. }))
        .collect()
}

#[test]
fn a_codex_session_builds_a_real_conversation() {
    let mut set = ConversationSet::new(4, Runtime::Codex);
    let mut cursor = StreamCursor::default();
    apply(
        &mut set,
        Runtime::Codex,
        sent_turn(Runtime::Codex, false, CODEX_PROMPT, &mut cursor),
    );
    for line in codex_first_run() {
        for event in codex_stream::parse_line(line) {
            apply(
                &mut set,
                Runtime::Codex,
                map(Runtime::Codex, false, &event, &mut cursor),
            );
        }
    }

    let conversation = set.snapshot(None).expect("the synthesised hooks built one");
    assert_eq!(
        conversation.session_id.as_deref(),
        Some("00000000-0000-4000-8000-000000000157")
    );
    assert_eq!(conversation.degraded, None);
    let roles: Vec<Role> = conversation.turns.iter().map(|t| t.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant]);
    assert_eq!(
        conversation.turns[0].blocks,
        [Block::Text {
            text: CODEX_PROMPT.into()
        }]
    );
    let reply = &conversation.turns[1];
    assert_eq!(reply.state, TurnState::Complete);
    assert_eq!(
        reply.blocks[0],
        Block::Text {
            text: "I’ll create a.txt and commit it as requested.\n\n\ndone".into()
        }
    );
    let calls = tool_calls(&reply.blocks);
    let [
        Block::ToolCall {
            id,
            name,
            input,
            result,
            state,
            ..
        },
    ] = calls.as_slice()
    else {
        panic!("{calls:?}");
    };
    assert_eq!(id.as_deref(), Some("item_2"));
    assert_eq!(name, "Bash");
    assert_eq!(
        input
            .as_ref()
            .and_then(|i| i.get("command"))
            .and_then(Value::as_str),
        Some(
            "/bin/zsh -lc \"printf 'one\\\\n' > a.txt\ngit add a.txt && git commit -m \\\"add a\\\"\""
        )
    );
    assert_eq!(*state, ToolState::Ok);
    let result = result
        .as_ref()
        .expect("the synthesised PostToolUse gave it a result");
    assert!(result.ok);
    // M6.5 renders an object `tool_response` as its compact JSON (as it does for Claude's
    // own `{"stdout": …}`), so the one-line summary shows the table's `{"output": …}`;
    // the full text is the enriched `detail`.
    assert!(
        result
            .summary
            .starts_with(r#"{"output":"[task-b 9d0ec2a] add a\n"#),
        "{}",
        result.summary
    );
    assert_eq!(
        result.detail.as_deref(),
        Some(
            "[task-b 9d0ec2a] add a\n 1 file changed, 1 insertion(+)\n create mode 100644 a.txt\n"
        )
    );

    // Without the synthesised hooks, the same records build nothing (M6.5 decision 1:
    // hooks create turns, records only enrich them).
    let mut bare = ConversationSet::new(5, Runtime::Codex);
    let mut cursor = StreamCursor::default();
    let mut records = sent_turn(Runtime::Codex, true, CODEX_PROMPT, &mut cursor).records;
    for line in codex_first_run() {
        for event in codex_stream::parse_line(line) {
            let input = map(Runtime::Codex, true, &event, &mut cursor);
            assert!(input.hooks.is_empty());
            records.extend(input.records);
        }
    }
    assert!(records.len() >= 5, "{records:?}");
    bare.enrich(&records, Caps::default());
    assert!(bare.snapshot_or_empty(None).turns.is_empty());
}

#[test]
fn a_claude_session_enriches_hook_built_turns() {
    let mut set = ConversationSet::new(3, Runtime::Claude);
    // The real hooks, as `anthrex hook` delivers them.
    for line in lines(CLAUDE_HOOKS) {
        let payload: Value = serde_json::from_str(line).unwrap();
        let hook = crate::hooks::parse(HookSource::Claude, &payload).expect("a hook");
        set.on_hook(
            Runtime::Claude,
            &hook,
            None,
            1_700_000_000,
            Instant::now(),
            Caps::default(),
        );
    }
    let hook_built = set.snapshot(None).unwrap();

    // The stream, turn by turn after each message the daemon sent.
    let prompts: Vec<String> = lines(CLAUDE_INPUT)[..3]
        .iter()
        .map(|l| {
            let v: Value = serde_json::from_str(l).unwrap();
            v["message"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    let mut turns: Vec<Vec<&str>> = vec![vec![]];
    for line in lines(CLAUDE_STREAM) {
        turns.last_mut().unwrap().push(line);
        if crate::headless::test_support::line_type(line) == "result" {
            turns.push(vec![]);
        }
    }
    assert_eq!(turns.len(), 4, "three recorded turns and nothing after");
    let mut parser = ClaudeStream::default();
    let mut cursor = StreamCursor::default();
    for (prompt, turn) in prompts.iter().zip(&turns) {
        let input = sent_turn(Runtime::Claude, true, prompt, &mut cursor);
        assert!(input.hooks.is_empty());
        apply(&mut set, Runtime::Claude, input);
        for line in turn {
            for event in parser.parse_line(line) {
                let input = map(Runtime::Claude, true, &event, &mut cursor);
                assert!(input.hooks.is_empty(), "hooks_fire: the real ones are used");
                apply(&mut set, Runtime::Claude, input);
            }
        }
    }

    let conversation = set.snapshot(None).unwrap();
    assert_eq!(conversation.degraded, None);
    // No turn is added or duplicated: the hooks built all six.
    let roles: Vec<Role> = conversation.turns.iter().map(|t| t.role).collect();
    assert_eq!(
        roles,
        hook_built.turns.iter().map(|t| t.role).collect::<Vec<_>>()
    );
    assert_eq!(
        roles,
        [
            Role::User,
            Role::Assistant,
            Role::User,
            Role::Assistant,
            Role::User,
            Role::Assistant
        ]
    );
    for (i, prompt) in prompts.iter().enumerate() {
        assert_eq!(
            conversation.turns[i * 2].blocks,
            [Block::Text {
                text: prompt.clone()
            }]
        );
    }

    // The prose lands at the front of each reply.
    assert_eq!(
        conversation.turns[1].blocks[0],
        Block::Text {
            text: "Done.".into()
        }
    );
    assert_eq!(
        conversation.turns[3].blocks[0],
        Block::Text {
            text: "Write blocked: path /tmp/fixture/outside.txt is outside allowed working directories."
                .into()
        }
    );
    assert!(!matches!(
        conversation.turns[5].blocks.first(),
        Some(Block::Text { .. })
    ));

    // Tool detail lands by id: the Bash call's full output and the Agent call's report.
    let detail = |id: &str| {
        conversation
            .turns
            .iter()
            .flat_map(|t| &t.blocks)
            .find_map(|b| match b {
                Block::ToolCall {
                    id: Some(call),
                    input,
                    result,
                    ..
                } if call == id => Some((
                    input.clone(),
                    result.as_ref().and_then(|r| r.detail.clone()),
                )),
                _ => None,
            })
    };
    let (input, bash) = detail("toolu_fixture01").unwrap();
    assert_eq!(
        input,
        Some(json!({"command": "ls", "description": "List files in current directory"}))
    );
    assert_eq!(bash.as_deref(), Some("README.md"));
    let (_, agent) = detail("toolu_fixture02").unwrap();
    assert!(
        agent
            .unwrap()
            .starts_with("[Subagent hand-back] The text below")
    );
    // Nothing about the hook-built detail was there before the stream.
    let before = hook_built
        .turns
        .iter()
        .flat_map(|t| &t.blocks)
        .find_map(|b| match b {
            Block::ToolCall {
                id: Some(call),
                result,
                ..
            } if call == "toolu_fixture01" => Some(result.as_ref().and_then(|r| r.detail.clone())),
            _ => None,
        });
    assert_eq!(before, Some(None));
}
