//! Parsing a sub-agent spawn request out of a hook's tool input.
//!
//! `spawn_request` reads the fields a spawning tool call carries (Claude's
//! `Agent`, Codex's `spawn_agent`) into a `PendingSpawn`, which the tracker
//! matches against the `SubagentStart` hook that follows it. `make_label`
//! is the label ordering itself (decision 1 of the graph-overview brief):
//! `description`, then `name`, then the first line of `prompt`, each skipped
//! when absent or empty.

use crate::hooks::{HookKind, ParsedHook};
use proto::Runtime;

pub const LABEL_MAX_CHARS: usize = 60;
pub const CLAUDE_SPAWN_TOOL: &str = "Agent";
pub const CODEX_SPAWN_TOOL: &str = "spawn_agent";
pub const CODEX_SPAWN_TOOL_ALIAS: &str = "collaborationspawn_agent";
pub const CODEX_SPAWN_TYPE_KEY: Option<&str> = None;
pub const CODEX_SPAWN_LABEL_KEYS: (Option<&str>, Option<&str>) = (Some("task_name"), None);
pub const CODEX_SPAWN_MODEL_KEY: Option<&str> = Some("model");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingSpawn {
    pub parent_id: Option<String>,
    pub subagent_type: Option<String>,
    pub label: Option<String>,
    pub model: Option<String>,
}

pub fn spawn_request(runtime: Runtime, hook: &ParsedHook) -> Option<PendingSpawn> {
    if hook.kind != HookKind::PreToolUse {
        return None;
    }
    let (type_key, label_keys, model_key) = match runtime {
        Runtime::Claude if hook.tool_name.as_deref() == Some(CLAUDE_SPAWN_TOOL) => (
            Some("subagent_type"),
            (Some("description"), Some("name"), Some("prompt")),
            Some("model"),
        ),
        Runtime::Codex
            if matches!(
                hook.tool_name.as_deref(),
                Some(CODEX_SPAWN_TOOL | CODEX_SPAWN_TOOL_ALIAS)
            ) =>
        {
            (
                CODEX_SPAWN_TYPE_KEY,
                // Codex has no description-like field; its `name` key slots
                // into make_label's `name` position, unchanged from before.
                (None, CODEX_SPAWN_LABEL_KEYS.0, CODEX_SPAWN_LABEL_KEYS.1),
                CODEX_SPAWN_MODEL_KEY,
            )
        }
        _ => return None,
    };
    let input = hook
        .tool_input
        .as_ref()
        .and_then(serde_json::Value::as_object);
    let string = |field: Option<&str>| {
        field.and_then(|field| {
            input
                .and_then(|object| object.get(field))
                .and_then(serde_json::Value::as_str)
        })
    };
    Some(PendingSpawn {
        parent_id: hook.agent_id.clone(),
        subagent_type: string(type_key).map(str::to_owned),
        label: make_label(
            string(label_keys.0),
            string(label_keys.1),
            string(label_keys.2),
        ),
        model: string(model_key).map(str::to_owned),
    })
}

/// Builds a sub-agent label from candidates tried in order: `description`,
/// then `name`, both used whole, then `prompt`, whose first line only is
/// used since it is the sole candidate that can span multiple lines.
pub fn make_label(
    description: Option<&str>,
    name: Option<&str>,
    prompt: Option<&str>,
) -> Option<String> {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.filter(|value| !value.is_empty())
    }
    let label = non_empty(description)
        .or_else(|| non_empty(name))
        .or_else(|| {
            prompt
                .and_then(|value| value.lines().next())
                .filter(|value| !value.is_empty())
        })?;
    if label.chars().count() <= LABEL_MAX_CHARS {
        return Some(label.to_owned());
    }
    Some(
        label
            .chars()
            .take(LABEL_MAX_CHARS - 1)
            .chain(std::iter::once('…'))
            .collect(),
    )
}
