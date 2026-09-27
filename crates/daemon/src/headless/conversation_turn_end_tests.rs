//! M8c.11: a turn the daemon knows has ended is closed in the conversation even when
//! nothing else closes it. Claude fires no `Stop` hook for an interrupted turn, so with
//! its hooks firing the `TurnEnded { Interrupted }` keeps its synthesised `Stop`; an
//! interrupted Codex turn ends with its process and no `turn.*` line, so a
//! `ProcessExited` while a sent turn is open synthesises one. A turn a `Stop` already
//! closed gets no second one. Every case runs through a real M6.5 `ConversationSet`.

use super::super::*;
use super::{exited, stream_hook};
use crate::conversation::{Caps, ConversationSet};
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::{FailureKind, TurnOutcome};
use crate::hooks::HookKind;
use proto::{Block, HookSource, Role, ToolState, TurnState};
use serde_json::{Value, json};
use std::time::Instant;

const SESSION: &str = "00000000-0000-4000-8000-000000000011";

/// Claude's `result` line for a turn interrupted mid-tool.
const INTERRUPTED: &str = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_tools"}"#;

/// A window's conversation and cursor, fed as the manager feeds them: real hooks through
/// `on_hook` and, for Claude with hooks firing, `observe_hook`; session events through
/// `map`. Every hook `map` synthesises is kept in `synthesised`.
struct Window {
    runtime: Runtime,
    hooks_fire: bool,
    set: ConversationSet,
    cursor: StreamCursor,
    /// Whether real hooks reach `observe_hook`, as the manager's wiring has them.
    feed: bool,
    synthesised: Vec<ParsedHook>,
}

impl Window {
    fn new(runtime: Runtime, hooks_fire: bool, cursor: StreamCursor, feed: bool) -> Self {
        Window {
            runtime,
            hooks_fire,
            set: ConversationSet::new(7, runtime),
            cursor,
            feed,
            synthesised: Vec::new(),
        }
    }

    fn apply(&mut self, input: ConversationInput) {
        for hook in &input.hooks {
            self.on_hook(hook);
        }
        self.set.enrich(&input.records, Caps::default());
    }

    fn on_hook(&mut self, hook: &ParsedHook) {
        self.set
            .on_hook(self.runtime, hook, None, 0, Instant::now(), Caps::default());
    }

    /// A real Claude hook, as `anthrex hook` delivers it.
    fn real_hook(&mut self, payload: Value) {
        let hook = crate::hooks::parse(HookSource::Claude, &payload).unwrap();
        self.on_hook(&hook);
        if self.feed {
            let input = observe_hook(self.runtime, &hook, &mut self.cursor);
            self.apply(input);
        }
    }

    fn sent(&mut self, text: &str) {
        let input = sent_turn(self.runtime, self.hooks_fire, text, &mut self.cursor);
        self.apply(input);
    }

    /// The hooks `map` synthesised for `event`, after applying them.
    fn event(&mut self, event: &SessionEvent) -> Vec<ParsedHook> {
        let input = map(self.runtime, self.hooks_fire, event, &mut self.cursor);
        let hooks = input.hooks.clone();
        self.synthesised.extend(hooks.iter().cloned());
        self.apply(input);
        hooks
    }

    fn claude_line(&mut self, line: &str) -> Vec<ParsedHook> {
        let mut hooks = Vec::new();
        for event in ClaudeStream::default().parse_line(line) {
            hooks.extend(self.event(&event));
        }
        hooks
    }

    fn conversation(&self) -> proto::Conversation {
        self.set.snapshot_or_empty(None)
    }
}

fn tool_use(id: &str) -> SessionEvent {
    SessionEvent::ToolUse {
        id: id.into(),
        name: "Bash".into(),
        input: json!({"command": "sleep 100"}),
        parent: None,
    }
}

fn turn_ended(outcome: TurnOutcome) -> SessionEvent {
    SessionEvent::TurnEnded {
        outcome,
        usage: None,
        denials: vec![],
    }
}

fn stop() -> ParsedHook {
    stream_hook(HookKind::Stop, Some(SESSION))
}

/// The last turn is a finished reply whose one call is no longer `Pending`.
fn assert_reply_closed(conversation: &proto::Conversation) {
    let reply = conversation.turns.last().expect("a reply");
    assert_eq!(reply.role, Role::Assistant);
    assert_eq!(reply.state, TurnState::Complete, "{conversation:?}");
    let states: Vec<ToolState> = reply
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::ToolCall { state, .. } => Some(*state),
            _ => None,
        })
        .collect();
    assert_eq!(states, [ToolState::Denied], "{conversation:?}");
}

/// A Claude turn with hooks firing, up to a tool call that never finishes: its prompt
/// and tool hooks are real, the stream adds nothing but records.
fn claude_turn_with_open_call(cursor: StreamCursor, feed: bool) -> Window {
    let mut w = Window::new(Runtime::Claude, true, cursor, feed);
    w.real_hook(
        json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"}),
    );
    w.sent("run the slow thing");
    w.real_hook(json!({
        "hook_event_name": "UserPromptSubmit", "session_id": SESSION,
        "prompt": "run the slow thing"
    }));
    w.claude_line(&format!(
        r#"{{"type":"system","subtype":"init","session_id":"{SESSION}","model":"m"}}"#
    ));
    w.real_hook(json!({
        "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
        "tool_input": {"command": "sleep 100"}, "tool_use_id": "toolu_slow"
    }));
    let running = w.conversation();
    assert_eq!(running.turns.last().unwrap().state, TurnState::Running);
    w
}

#[test]
fn an_interrupted_turn_ends_even_when_hooks_fire() {
    // The manager's wiring (a fed cursor, real hooks observed), a cursor that enters
    // content mode on its first prompt, and one no hook ever reaches.
    for (cursor, feed) in [
        (StreamCursor::fed(), true),
        (StreamCursor::default(), true),
        (StreamCursor::default(), false),
    ] {
        let mut w = claude_turn_with_open_call(cursor, feed);
        let hooks = w.claude_line(INTERRUPTED);
        assert_eq!(hooks, [stop()]);
        assert_reply_closed(&w.conversation());
    }
    // A completed or failed turn still gets nothing: its own `Stop` hook closes it.
    for outcome in [
        TurnOutcome::Completed,
        TurnOutcome::Failed {
            error: "x".into(),
            kind: FailureKind::Other,
        },
    ] {
        let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
        assert_eq!(w.event(&turn_ended(outcome)), []);
    }
}

#[test]
fn a_codex_process_exit_ends_the_open_turn() {
    let mut w = Window::new(Runtime::Codex, false, StreamCursor::default(), false);
    w.sent("run the slow thing");
    w.event(&SessionEvent::Init {
        session_id: SESSION.into(),
        model: None,
        mcp_ok: None,
    });
    w.event(&SessionEvent::TurnStarted);
    w.event(&tool_use("item_1"));
    assert_eq!(
        w.conversation().turns.last().unwrap().state,
        TurnState::Running
    );

    assert_eq!(w.event(&exited()), [stop()]);
    assert_reply_closed(&w.conversation());
    // Hostile: the same exit again ends nothing more.
    assert_eq!(w.event(&exited()), []);

    // The next turn is built as usual, and its call keeps its result.
    w.sent("again");
    w.event(&tool_use("item_1"));
    w.event(&SessionEvent::ToolResult {
        id: "item_1".into(),
        text: "done".into(),
        ok: true,
        parent: None,
    });
    assert_eq!(w.event(&turn_ended(TurnOutcome::Completed)), [stop()]);
    let conversation = w.conversation();
    let roles: Vec<Role> = conversation.turns.iter().map(|t| t.role).collect();
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    let Block::ToolCall { state, .. } = &conversation.turns[3].blocks[0] else {
        panic!("{conversation:?}");
    };
    assert_eq!(*state, ToolState::Ok);
    assert_eq!(
        w.synthesised
            .iter()
            .filter(|h| h.kind == HookKind::Stop)
            .count(),
        2
    );
}

#[test]
fn a_turn_already_ended_by_its_own_stop_gets_no_second_one() {
    // Claude, hooks firing: a real `Stop` hook closed the turn before its `result`.
    for cursor in [StreamCursor::fed(), StreamCursor::default()] {
        let mut w = claude_turn_with_open_call(cursor, true);
        w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
        assert_eq!(w.claude_line(INTERRUPTED), []);
        // The next turn's prompt makes a later interruption count again.
        w.sent("next");
        w.real_hook(json!({
            "hook_event_name": "UserPromptSubmit", "session_id": SESSION, "prompt": "next"
        }));
        assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
    }
    // A `Stop` hook that comes after its turn's `result` does not hold over the next
    // prompt: that turn's interruption still ends it.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    w.claude_line(r#"{"type":"result","subtype":"success","is_error":false}"#);
    w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    w.sent("next");
    w.real_hook(json!({
        "hook_event_name": "UserPromptSubmit", "session_id": SESSION, "prompt": "next"
    }));
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
    // Claude without hooks: the synthesised `Stop` of the `result`, and nothing at exit.
    let mut w = Window::new(Runtime::Claude, false, StreamCursor::default(), false);
    w.sent("go");
    assert_eq!(w.claude_line(INTERRUPTED), [stop_without_session()]);
    assert_eq!(w.event(&exited()), []);
    // Codex: a turn its `turn.*` line ended gets nothing more from its process's exit.
    for outcome in [
        TurnOutcome::Completed,
        TurnOutcome::Failed {
            error: "usage limit".into(),
            kind: FailureKind::RateLimit,
        },
    ] {
        let mut w = Window::new(Runtime::Codex, false, StreamCursor::default(), false);
        w.sent("go");
        w.event(&tool_use("item_1"));
        assert_eq!(w.event(&turn_ended(outcome)), [stop_without_session()]);
        assert_eq!(w.event(&exited()), []);
        assert_eq!(
            w.synthesised
                .iter()
                .filter(|h| h.kind == HookKind::Stop)
                .count(),
            1
        );
    }
}

/// A `Stop` synthesised before any `Init` named the session.
fn stop_without_session() -> ParsedHook {
    stream_hook(HookKind::Stop, None)
}

#[test]
fn a_process_exit_with_no_turn_open_ends_nothing() {
    let mut w = Window::new(Runtime::Codex, false, StreamCursor::default(), false);
    assert_eq!(w.event(&exited()), []);
    w.event(&SessionEvent::Init {
        session_id: SESSION.into(),
        model: None,
        mcp_ok: None,
    });
    assert_eq!(w.event(&exited()), []);
    assert!(w.conversation().turns.is_empty());
}

/// Hostile: an interrupted `result` for a turn the daemon never sent. With Claude's hooks
/// firing its turn, if any, was built by its own prompt hook (a turn Claude Code started
/// by itself), and it has ended, so it gets a `Stop`; with no turn open that `Stop`
/// changes nothing.
#[test]
fn an_interrupted_turn_that_was_never_sent() {
    for (cursor, feed) in [
        (StreamCursor::fed(), true),
        (StreamCursor::default(), true),
        (StreamCursor::default(), false),
    ] {
        let mut w = Window::new(Runtime::Claude, true, cursor, feed);
        w.real_hook(
            json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"}),
        );
        assert_eq!(w.claude_line(INTERRUPTED), [stop_without_session()]);
        assert!(w.conversation().turns.is_empty());

        // A turn Claude Code started by itself, then interrupted.
        w.real_hook(json!({
            "hook_event_name": "UserPromptSubmit", "session_id": SESSION,
            "prompt": "<task-notification>agent done</task-notification>"
        }));
        w.real_hook(json!({
            "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
            "tool_input": {"command": "sleep 100"}, "tool_use_id": "toolu_bg"
        }));
        // The prompt hook named the session when it reached the cursor.
        let want = if feed { stop() } else { stop_without_session() };
        assert_eq!(w.claude_line(INTERRUPTED), [want]);
        assert_reply_closed(&w.conversation());
    }
}

/// The review's roles and states, to compare a conversation's shape at a glance.
fn shape(conversation: &proto::Conversation) -> Vec<(Role, TurnState)> {
    conversation
        .turns
        .iter()
        .map(|t| (t.role, t.state))
        .collect()
}

fn killed() -> SessionEvent {
    SessionEvent::ProcessExited {
        code: None,
        signal: Some(15),
    }
}

fn notification_turn(w: &mut Window, tool_use_id: &str) {
    w.real_hook(json!({
        "hook_event_name": "UserPromptSubmit", "session_id": SESSION,
        "prompt": "<task-notification>agent done</task-notification>"
    }));
    w.real_hook(json!({
        "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
        "tool_input": {"command": "sleep 100"}, "tool_use_id": tool_use_id
    }));
}

/// M8c.11 review I1: hooks and results arrive by different paths, so a turn Claude Code
/// started by itself can have its prompt and tool hooks applied before the interrupted
/// turn's `result` is read. That `result` ends the interrupted turn only: its `Stop`
/// must not close the later turn, which is still working.
#[test]
fn an_interrupt_never_closes_a_later_turn_already_open() {
    for cursor in [StreamCursor::fed(), StreamCursor::default()] {
        let mut w = claude_turn_with_open_call(cursor, true);
        notification_turn(&mut w, "toolu_bg");
        let before = shape(&w.conversation());
        assert_eq!(
            before[before.len() - 2..],
            [
                (Role::User, TurnState::Complete),
                (Role::Assistant, TurnState::Running)
            ]
        );
        assert_eq!(w.claude_line(INTERRUPTED), []);
        assert_eq!(shape(&w.conversation()), before);
        // The later turn's own interruption still ends it.
        assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
        assert_reply_closed(&w.conversation());
    }
}

/// M8c.11 review I2: an engine kill (a stall, a cancel, a give-up) SIGTERMs a Claude
/// process mid-turn, and no `result` is printed. Its exit ends the open turn, and the
/// resumed session's next turn builds as usual.
#[test]
fn a_killed_claude_turn_ends() {
    for (cursor, feed) in [
        (StreamCursor::fed(), true),
        (StreamCursor::default(), true),
        (StreamCursor::default(), false),
    ] {
        let mut w = claude_turn_with_open_call(cursor, feed);
        assert_eq!(w.event(&killed()), [stop()]);
        assert_reply_closed(&w.conversation());
        // Hostile: the same exit again ends nothing more.
        assert_eq!(w.event(&killed()), []);

        w.cursor.process_replaced();
        w.sent("next");
        w.real_hook(json!({
            "hook_event_name": "UserPromptSubmit", "session_id": SESSION, "prompt": "next"
        }));
        w.real_hook(json!({
            "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
            "tool_input": {"command": "ls"}, "tool_use_id": "toolu_next"
        }));
        let conversation = w.conversation();
        let tail = shape(&conversation);
        assert_eq!(
            tail[tail.len() - 4..],
            [
                (Role::User, TurnState::Complete),
                (Role::Assistant, TurnState::Complete),
                (Role::User, TurnState::Complete),
                (Role::Assistant, TurnState::Running),
            ],
            "{conversation:?}"
        );
        let Block::ToolCall { state, .. } = &conversation.turns.last().unwrap().blocks[0] else {
            panic!("{conversation:?}");
        };
        assert_eq!(*state, ToolState::Pending);
    }
    // A kill of a turn whose prompt hook opened it, in a fed cursor, and a kill of a
    // turn Claude Code started by itself.
    let mut w = Window::new(Runtime::Claude, true, StreamCursor::fed(), true);
    notification_turn(&mut w, "toolu_bg");
    assert_eq!(w.event(&killed()), [stop()]);
    assert_reply_closed(&w.conversation());
    // Claude without hooks: the sent turn's exit synthesises the `Stop` too.
    let mut w = Window::new(Runtime::Claude, false, StreamCursor::default(), false);
    w.sent("go");
    w.event(&tool_use("toolu_slow"));
    assert_eq!(w.event(&killed()), [stop_without_session()]);
    assert_reply_closed(&w.conversation());
}

#[test]
fn a_claude_exit_with_no_turn_open_ends_nothing() {
    for (cursor, feed) in [
        (StreamCursor::fed(), true),
        (StreamCursor::default(), true),
        (StreamCursor::default(), false),
    ] {
        let mut w = Window::new(Runtime::Claude, true, cursor.clone(), feed);
        assert_eq!(w.event(&killed()), []);
        w.real_hook(
            json!({"hook_event_name": "SessionStart", "session_id": SESSION, "source": "startup"}),
        );
        assert_eq!(w.event(&killed()), []);
        // A turn that ended normally: its `Stop` hook, then its `result`.
        let mut w = claude_turn_with_open_call(cursor, feed);
        w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
        assert_eq!(
            w.claude_line(r#"{"type":"result","subtype":"success","is_error":false}"#),
            []
        );
        assert_eq!(w.event(&exited()), []);
    }
    // A turn its real `Stop` hook closed, killed before its `result`: the exit ends the
    // turn in the cursor but synthesises no second `Stop`.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    assert_eq!(w.event(&killed()), []);
    assert_eq!(w.event(&killed()), []);
    // Claude without hooks, after its turn's `result`.
    let mut w = Window::new(Runtime::Claude, false, StreamCursor::default(), false);
    w.sent("go");
    w.claude_line(r#"{"type":"result","subtype":"success","is_error":false}"#);
    assert_eq!(w.event(&exited()), []);
}

/// M8c.11 review m1: a real `Stop` does not always end the turn. The user's own blocking
/// `Stop` hook makes Claude go on, and M6.5 opens a new reply on its next tool hook; an
/// interruption of that continuation must still end it.
#[test]
fn a_continuation_after_a_blocking_stop_hook_still_ends_when_interrupted() {
    for cursor in [StreamCursor::fed(), StreamCursor::default()] {
        let mut w = claude_turn_with_open_call(cursor, true);
        w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
        w.real_hook(json!({
            "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
            "tool_input": {"command": "sleep 100"}, "tool_use_id": "toolu_cont"
        }));
        assert_eq!(
            w.conversation().turns.last().unwrap().state,
            TurnState::Running
        );
        assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
        let last = w.conversation().turns.last().unwrap().clone();
        assert_eq!(
            (last.role, last.state),
            (Role::Assistant, TurnState::Complete)
        );
    }
    // The same through a `PostToolUse` alone (its `PreToolUse` came before the `Stop`).
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    w.real_hook(json!({
        "hook_event_name": "PostToolUse", "session_id": SESSION, "tool_name": "Bash",
        "tool_use_id": "toolu_slow", "tool_response": {"stdout": "x"}
    }));
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
}

/// M8c.11 review m1: a `Stop` hook that comes after its turn's `result` (its ack missed
/// `anthrex hook`'s deadline) does not hold over a later turn whose prompt hook was lost,
/// nor over a new process of the session.
#[test]
fn a_late_stop_hook_does_not_hold_over_a_lost_prompt() {
    let success = r#"{"type":"result","subtype":"success","is_error":false}"#;
    let late_stop = |w: &mut Window| {
        w.claude_line(success);
        w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    };
    // The next turn's prompt hook is lost; its tool hook arrives.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    late_stop(&mut w);
    w.sent("next");
    w.real_hook(json!({
        "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
        "tool_input": {"command": "sleep 100"}, "tool_use_id": "toolu_next"
    }));
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
    assert_eq!(
        w.conversation().turns.last().unwrap().state,
        TurnState::Complete
    );
    // A new process, and its first turn's prompt hook is lost too.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    late_stop(&mut w);
    w.cursor.process_replaced();
    w.sent("next");
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
}

/// M8c.11 review m2: a turn's `result` clears the real `Stop` that closed it, so the next
/// turn, whose prompt hook was lost, still ends when interrupted.
#[test]
fn a_turn_end_clears_its_real_stop() {
    let success = r#"{"type":"result","subtype":"success","is_error":false}"#;
    // The review's order: a tool hook comes between the two `result`s.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    w.claude_line(success);
    w.sent("next");
    w.real_hook(json!({
        "hook_event_name": "PreToolUse", "session_id": SESSION, "tool_name": "Bash",
        "tool_input": {"command": "sleep 100"}, "tool_use_id": "toolu_next"
    }));
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
    // Nothing but prose between them: only the `result` can have cleared it.
    let mut w = claude_turn_with_open_call(StreamCursor::fed(), true);
    w.real_hook(json!({"hook_event_name": "Stop", "session_id": SESSION}));
    w.claude_line(success);
    w.sent("next");
    w.event(&SessionEvent::AssistantText {
        text: "thinking".into(),
        parent: None,
    });
    assert_eq!(w.claude_line(INTERRUPTED), [stop()]);
}
