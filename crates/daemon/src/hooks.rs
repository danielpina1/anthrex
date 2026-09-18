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
    if source != HookSource::Claude {
        return None;
    }

    let object = payload.as_object()?;
    let kind = match object.get("hook_event_name")?.as_str()? {
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
    };
    let string = |key| object.get(key).and_then(Value::as_str).map(str::to_owned);

    Some(ParsedHook {
        source,
        kind,
        session_id: string("session_id"),
        agent_id: string("agent_id"),
        agent_type: string("agent_type"),
        tool_name: string("tool_name"),
        tool_input: object.get("tool_input").cloned(),
        notification_type: string("notification_type"),
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
    fn parses_a_claude_pre_tool_use_payload() {
        let tool_input = json!({"path": "/tmp/example.rs", "line": 42});
        let payload = json!({
            "hook_event_name": "PreToolUse",
            "session_id": "session-1",
            "agent_id": "agent-7",
            "agent_type": "Explore",
            "tool_name": "Read",
            "tool_input": tool_input,
            "notification_type": "ignored-for-this-event"
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
            })
        );
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
        }
    }
}
