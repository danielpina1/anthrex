//! Pure parsing and classification of agent hook payloads.

use crate::status::StatusEvent;
use proto::{HookSource, Runtime};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookKind {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PermissionRequest,
    Notification,
    Stop,
    SubagentStart,
    SubagentStop,
    SessionEnd,
    TurnComplete,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedHook {
    pub source: HookSource,
    pub kind: HookKind,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub agent_type: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<Value>,
    pub notification_type: Option<String>,
    /// Claude's `transcript_path`; `None` for every runtime that does not send one.
    pub transcript_path: Option<String>,
    /// The runtime's own id for this tool use, when it sends one, so a `PostToolUse` can
    /// be matched to its `PreToolUse` exactly rather than by name.
    pub tool_use_id: Option<String>,
    /// The bounded result `anthrex hook` now forwards (decision 5a).
    pub tool_response: Option<Value>,
    /// `Some(true)` when the CLI truncated it, `Some(false)` when it did not, `None` when
    /// an older CLI **or** a hook with no `tool_response` produced this payload. `None`
    /// here means "unknown", not "not truncated".
    pub tool_result_truncated: Option<bool>,
    /// `Some(true)` when `tool_response` was an array, number, bool or null that had to be
    /// rendered to text to fit `TOOL_RESULT_SUMMARY_MAX`, `Some(false)` otherwise, `None`
    /// under the same two circumstances as `tool_result_truncated` above. An object
    /// `tool_response` never sets this to `true` (`crates/cli`'s `bound_object` always
    /// keeps it as an object), so `ToolResult.ok` never needs to read it.
    pub tool_result_stringified: Option<bool>,
    /// `UserPromptSubmit`'s own text.
    pub prompt: Option<String>,
    /// `SessionStart`'s `source` (`startup`, `resume`, `clear`, `compact` for Claude).
    /// `resume` means the session's transcript already holds its earlier turns (task
    /// M6.5.10 fix round 2, N1/N2).
    pub session_source: Option<String>,
}

pub fn accepts(runtime: Runtime, source: HookSource) -> bool {
    matches!(
        (runtime, source),
        (Runtime::Claude, HookSource::Claude)
            | (
                Runtime::Codex,
                HookSource::CodexNotify | HookSource::CodexHook
            )
    )
}

pub fn parse(source: HookSource, payload: &Value) -> Option<ParsedHook> {
    let object = payload.as_object()?;
    let kind = if source == HookSource::CodexNotify {
        if object.get("type")?.as_str()? != "agent-turn-complete" {
            return None;
        }
        HookKind::TurnComplete
    } else {
        match object.get("hook_event_name")?.as_str()? {
            "SessionStart" => HookKind::SessionStart,
            "UserPromptSubmit" => HookKind::UserPromptSubmit,
            "PreToolUse" => HookKind::PreToolUse,
            "PostToolUse" => HookKind::PostToolUse,
            "PermissionRequest" => HookKind::PermissionRequest,
            "Notification" => HookKind::Notification,
            "Stop" => HookKind::Stop,
            "SubagentStart" => HookKind::SubagentStart,
            "SubagentStop" => HookKind::SubagentStop,
            "SessionEnd" => HookKind::SessionEnd,
            _ => return None,
        }
    };
    let string = |key| object.get(key).and_then(Value::as_str).map(str::to_owned);
    let boolean = |key| object.get(key).and_then(Value::as_bool);

    Some(ParsedHook {
        source,
        kind,
        session_id: string(if source == HookSource::CodexNotify {
            "thread-id"
        } else {
            "session_id"
        }),
        agent_id: string("agent_id"),
        agent_type: string("agent_type"),
        tool_name: string("tool_name"),
        tool_input: object.get("tool_input").cloned(),
        notification_type: string("notification_type"),
        transcript_path: string("transcript_path"),
        // Both spellings are tried; whichever the installed runtime actually sends is
        // recorded under "Implementation notes" from the capture in task M6.5.7. With
        // both present, `tool_use_id` wins.
        tool_use_id: string("tool_use_id").or_else(|| string("toolUseId")),
        tool_response: object.get("tool_response").cloned(),
        tool_result_truncated: boolean("tool_result_truncated"),
        tool_result_stringified: boolean("tool_result_stringified"),
        prompt: string("prompt"),
        session_source: string("source"),
    })
}

impl ParsedHook {
    pub fn status_event(&self) -> Option<StatusEvent> {
        match self.kind {
            HookKind::SessionStart => Some(StatusEvent::SessionStart),
            HookKind::UserPromptSubmit => Some(StatusEvent::UserPromptSubmit),
            HookKind::PreToolUse => Some(StatusEvent::PreToolUse),
            HookKind::PostToolUse => Some(StatusEvent::PostToolUse),
            HookKind::PermissionRequest => Some(StatusEvent::PermissionRequest),
            HookKind::Notification => match self.notification_type.as_deref() {
                Some("permission_prompt") => Some(StatusEvent::PermissionPrompt),
                Some("idle_prompt") => Some(StatusEvent::IdlePrompt),
                _ => None,
            },
            HookKind::Stop => Some(StatusEvent::Stop),
            HookKind::TurnComplete => Some(StatusEvent::CodexNotify),
            HookKind::SubagentStart | HookKind::SubagentStop | HookKind::SessionEnd => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::status::StatusEvent;
    use proto::{HookSource, Runtime};
    use serde_json::json;

    #[test]
    fn parses_codex_notify() {
        let hook = parse(
            HookSource::CodexNotify,
            &json!({"type":"agent-turn-complete", "thread-id":"t1", "session_id":"wrong"}),
        )
        .expect("notify parsed");
        assert_eq!(hook.kind, HookKind::TurnComplete);
        assert_eq!(hook.session_id.as_deref(), Some("t1"));
        assert_eq!(hook.status_event(), Some(StatusEvent::CodexNotify));
        assert_eq!(
            parse(
                HookSource::CodexNotify,
                &json!({"type":"other", "thread-id":"t1"})
            ),
            None
        );
    }

    #[test]
    fn parses_codex_hook_payloads_like_claude_ones() {
        let payload = json!({"hook_event_name":"PreToolUse", "session_id":"root", "agent_id":"child", "agent_type":"worker", "tool_name":"shell", "tool_input":{"command":"pwd"}});
        let hook = parse(HookSource::CodexHook, &payload).expect("lifecycle hook parsed");
        assert_eq!(hook.source, HookSource::CodexHook);
        assert_eq!(hook.kind, HookKind::PreToolUse);
        assert_eq!(hook.session_id.as_deref(), Some("root"));
        assert_eq!(hook.agent_id.as_deref(), Some("child"));
        assert_eq!(hook.agent_type.as_deref(), Some("worker"));
        assert_eq!(hook.tool_name.as_deref(), Some("shell"));
        assert_eq!(hook.tool_input, Some(json!({"command":"pwd"})));
    }

    #[test]
    fn parses_a_claude_pre_tool_use_payload() {
        let tool_input = json!({"path": "/tmp/example.rs", "line": 42});
        let tool_response = json!({"ok": true});
        let payload = json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session-1",
            "agent_id": "agent-7",
            "agent_type": "Explore",
            "tool_name": "Read",
            "tool_input": tool_input,
            "notification_type": "ignored-for-this-event",
            "transcript_path": "/home/user/.claude/transcripts/session-1.jsonl",
            "tool_use_id": "tu-99",
            "tool_response": tool_response,
            "tool_result_truncated": true,
            "tool_result_stringified": false,
            "prompt": "refactor the parser"
        });

        assert_eq!(
            parse(HookSource::Claude, &payload),
            Some(ParsedHook {
                source: HookSource::Claude,
                kind: HookKind::PreToolUse,
                session_id: Some("session-1".into()),
                agent_id: Some("agent-7".into()),
                agent_type: Some("Explore".into()),
                tool_name: Some("Read".into()),
                tool_input: Some(tool_input),
                notification_type: Some("ignored-for-this-event".into()),
                transcript_path: Some("/home/user/.claude/transcripts/session-1.jsonl".into()),
                tool_use_id: Some("tu-99".into()),
                tool_response: Some(tool_response),
                tool_result_truncated: Some(true),
                tool_result_stringified: Some(false),
                prompt: Some("refactor the parser".into()),
                session_source: None,
            })
        );
    }

    #[test]
    fn tool_use_id_accepts_both_spellings() {
        let snake = parse(
            HookSource::Claude,
            &json!({"hook_event_name": "PreToolUse", "tool_use_id": "a"}),
        )
        .unwrap();
        assert_eq!(snake.tool_use_id.as_deref(), Some("a"));

        let camel = parse(
            HookSource::Claude,
            &json!({"hook_event_name": "PreToolUse", "toolUseId": "b"}),
        )
        .unwrap();
        assert_eq!(camel.tool_use_id.as_deref(), Some("b"));

        let both = parse(
            HookSource::Claude,
            &json!({"hook_event_name": "PreToolUse", "tool_use_id": "a", "toolUseId": "b"}),
        )
        .unwrap();
        assert_eq!(both.tool_use_id.as_deref(), Some("a"));
    }

    #[test]
    fn parses_the_notification_type() {
        let parsed = parse(
            HookSource::Claude,
            &json!({
                "hook_event_name": "Notification",
                "notification_type": "permission_prompt"
            }),
        )
        .unwrap();

        assert_eq!(parsed.kind, HookKind::Notification);
        assert_eq!(
            parsed.notification_type.as_deref(),
            Some("permission_prompt")
        );
    }

    #[test]
    fn sub_agent_fields_are_read() {
        for (name, kind) in [
            ("SubagentStart", HookKind::SubagentStart),
            ("SubagentStop", HookKind::SubagentStop),
        ] {
            let parsed = parse(
                HookSource::Claude,
                &json!({
                    "hook_event_name": name,
                    "agent_id": "agent-2",
                    "agent_type": "general-purpose"
                }),
            )
            .unwrap();
            assert_eq!(parsed.kind, kind);
            assert_eq!(parsed.agent_id.as_deref(), Some("agent-2"));
            assert_eq!(parsed.agent_type.as_deref(), Some("general-purpose"));
        }
    }

    #[test]
    fn unknown_events_and_non_objects_are_none() {
        assert_eq!(
            parse(
                HookSource::Claude,
                &json!({"hook_event_name": "FutureEvent"})
            ),
            None
        );
        assert_eq!(parse(HookSource::Claude, &json!("Stop")), None);
        assert_eq!(parse(HookSource::Claude, &json!(null)), None);
        assert_eq!(parse(HookSource::CodexNotify, &json!({})), None);
        assert_eq!(parse(HookSource::CodexHook, &json!({})), None);
    }

    #[test]
    fn missing_fields_degrade_to_none() {
        assert_eq!(
            parse(HookSource::Claude, &json!({"hook_event_name": "Stop"})),
            Some(ParsedHook {
                source: HookSource::Claude,
                kind: HookKind::Stop,
                session_id: None,
                agent_id: None,
                agent_type: None,
                tool_name: None,
                tool_input: None,
                notification_type: None,
                transcript_path: None,
                tool_use_id: None,
                tool_response: None,
                tool_result_truncated: None,
                tool_result_stringified: None,
                prompt: None,
                session_source: None,
            })
        );
    }

    #[test]
    fn status_event_mapping() {
        let expected = [
            (HookKind::SessionStart, Some(StatusEvent::SessionStart)),
            (
                HookKind::UserPromptSubmit,
                Some(StatusEvent::UserPromptSubmit),
            ),
            (HookKind::PreToolUse, Some(StatusEvent::PreToolUse)),
            (HookKind::PostToolUse, Some(StatusEvent::PostToolUse)),
            (
                HookKind::PermissionRequest,
                Some(StatusEvent::PermissionRequest),
            ),
            (HookKind::Stop, Some(StatusEvent::Stop)),
            (HookKind::TurnComplete, Some(StatusEvent::CodexNotify)),
            (HookKind::SubagentStart, None),
            (HookKind::SubagentStop, None),
            (HookKind::SessionEnd, None),
        ];
        for (kind, event) in expected {
            assert_eq!(hook(kind, None).status_event(), event, "{kind:?}");
        }
        assert_eq!(
            hook(HookKind::Notification, Some("permission_prompt")).status_event(),
            Some(StatusEvent::PermissionPrompt)
        );
        assert_eq!(
            hook(HookKind::Notification, Some("idle_prompt")).status_event(),
            Some(StatusEvent::IdlePrompt)
        );
        assert_eq!(
            hook(HookKind::Notification, Some("other")).status_event(),
            None
        );
    }

    #[test]
    fn source_acceptance() {
        use HookSource::{Claude as ClaudeSource, CodexHook, CodexNotify};
        use Runtime::{Claude, Codex, Shell};

        let cases = [
            (Claude, ClaudeSource, true),
            (Claude, CodexNotify, false),
            (Claude, CodexHook, false),
            (Codex, ClaudeSource, false),
            (Codex, CodexNotify, true),
            (Codex, CodexHook, true),
            (Shell, ClaudeSource, false),
            (Shell, CodexNotify, false),
            (Shell, CodexHook, false),
        ];
        for (runtime, source, expected) in cases {
            assert_eq!(accepts(runtime, source), expected, "{runtime:?} {source:?}");
        }
    }

    fn hook(kind: HookKind, notification_type: Option<&str>) -> ParsedHook {
        ParsedHook {
            source: HookSource::Claude,
            kind,
            session_id: None,
            agent_id: None,
            agent_type: None,
            tool_name: None,
            tool_input: None,
            notification_type: notification_type.map(str::to_owned),
            transcript_path: None,
            tool_use_id: None,
            tool_response: None,
            tool_result_truncated: None,
            tool_result_stringified: None,
            prompt: None,
            session_source: None,
        }
    }
}
