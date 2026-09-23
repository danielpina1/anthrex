//! Session events to milestone 6.5's two conversation inputs (decision 27, the
//! Interfaces mapping table). Pure.

use super::SessionEvent;
use crate::hooks::{HookKind, ParsedHook};
use crate::transcript::Record;
use proto::{HookSource, Runtime};
use serde_json::json;
use std::collections::HashMap;

/// What `map` carries from one event to the next of the same session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamCursor {
    /// Turns the daemon has sent in this session; the next one's ordinal.
    prompts_sent: u32,
    /// Whether the latest sent turn is still open: prose after its `TurnEnded` belongs
    /// to a turn the daemon did not send.
    turn_open: bool,
    /// Top-level tool names by id, for the synthesised `PostToolUse`.
    tool_names: HashMap<String, String>,
    session_id: Option<String>,
}

/// Milestone 6.5's two inputs: hooks for `WindowManager::conversation_hook`, records
/// for `ConversationSet::enrich`, applied in that order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConversationInput {
    pub hooks: Vec<ParsedHook>,
    pub records: Vec<Record>,
}

/// The conversation inputs for one session event, per the Interfaces table. Hooks are
/// synthesised only when `hooks_fire` is false (every Codex session; Claude only if its
/// turn hooks did not fire in `-p`, which M8a.1 found they do), with `source: Stream`,
/// `session_source: None` and the session id from `Init`.
///
/// Two refinements of the table, both from M8a.1's recordings:
///
/// - Claude repeats `Init` every turn and Codex every process, so `SessionStart` is
///   synthesised only when the session id changes.
/// - Top-level prose is put on the latest sent turn only while that turn is open. A turn
///   Claude Code starts by itself (a background sub-agent finishing) has no sent prompt,
///   and its prose would otherwise be appended to the previous reply.
///
/// `runtime` is not read: the mapping is the same for both, and the caller passes
/// `hooks_fire` for the difference.
pub fn map(
    _runtime: Runtime,
    hooks_fire: bool,
    event: &SessionEvent,
    cursor: &mut StreamCursor,
) -> ConversationInput {
    let mut input = ConversationInput::default();
    match event {
        SessionEvent::Init { session_id, .. } => {
            if cursor.session_id.as_deref() != Some(session_id.as_str()) {
                cursor.session_id = Some(session_id.clone());
                input.hooks.push(cursor.hook(HookKind::SessionStart));
            }
        }
        SessionEvent::AssistantText { text, parent: None } => {
            if cursor.turn_open
                && let Some(ordinal) = cursor.prompts_sent.checked_sub(1)
            {
                input.records.push(Record::AssistantText {
                    session_id: cursor.session_id.clone(),
                    ordinal,
                    text: text.clone(),
                });
            }
        }
        SessionEvent::ToolUse {
            id,
            name,
            input: tool_input,
            parent,
        } => {
            input.records.push(Record::ToolDetail {
                tool_use_id: id.clone(),
                input: Some(tool_input.clone()),
                detail: None,
                ok: None,
            });
            if parent.is_none() {
                if cursor.tool_names.len() < TOOL_NAMES_MAX {
                    cursor.tool_names.insert(id.clone(), name.clone());
                }
                input.hooks.push(ParsedHook {
                    tool_name: Some(name.clone()),
                    tool_input: Some(tool_input.clone()),
                    tool_use_id: Some(id.clone()),
                    ..cursor.hook(HookKind::PreToolUse)
                });
            }
        }
        SessionEvent::ToolResult {
            id,
            text,
            ok,
            parent,
        } => {
            input.records.push(Record::ToolDetail {
                tool_use_id: id.clone(),
                input: None,
                detail: Some(text.clone()),
                ok: Some(*ok),
            });
            if parent.is_none() {
                let key = if *ok { "output" } else { "error" };
                let (response, truncated, stringified) =
                    proto::conversation::bound_tool_response_value(json!({ key: text }));
                input.hooks.push(ParsedHook {
                    tool_name: cursor.tool_names.remove(id),
                    tool_use_id: Some(id.clone()),
                    tool_response: Some(response),
                    tool_result_truncated: Some(truncated),
                    tool_result_stringified: Some(stringified),
                    ..cursor.hook(HookKind::PostToolUse)
                });
            }
        }
        SessionEvent::TurnEnded { .. } => {
            cursor.turn_open = false;
            cursor.tool_names.clear();
            input.hooks.push(cursor.hook(HookKind::Stop));
        }
        _ => {}
    }
    if hooks_fire {
        input.hooks.clear();
    }
    input
}

/// The conversation inputs for a turn the daemon starts with `text`: its prompt, as the
/// `k`-th (0-based) turn sent in this session (M6.5 ruling R1). `human: true` because a
/// hook-built prompt is exactly `text`, so M6.5's exact-match alignment applies.
pub fn sent_turn(
    _runtime: Runtime,
    hooks_fire: bool,
    text: &str,
    cursor: &mut StreamCursor,
) -> ConversationInput {
    let ordinal = cursor.prompts_sent;
    cursor.prompts_sent = cursor.prompts_sent.saturating_add(1);
    cursor.turn_open = true;
    let hooks = if hooks_fire {
        Vec::new()
    } else {
        vec![ParsedHook {
            prompt: Some(text.to_owned()),
            ..cursor.hook(HookKind::UserPromptSubmit)
        }]
    };
    ConversationInput {
        hooks,
        records: vec![Record::UserText {
            session_id: cursor.session_id.clone(),
            ordinal,
            text: text.to_owned(),
            human: true,
        }],
    }
}

/// The most tool names a cursor remembers. A turn's names are dropped when it ends, so
/// this only bounds a turn with calls whose results never arrive.
const TOOL_NAMES_MAX: usize = 1024;

impl StreamCursor {
    /// A synthesised hook of `kind` with every optional field empty.
    fn hook(&self, kind: HookKind) -> ParsedHook {
        ParsedHook {
            source: HookSource::Stream,
            kind,
            session_id: self.session_id.clone(),
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
        }
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
