//! Per-session facts, separate from the window lifecycle and status transition table.

use crate::hooks::{HookKind, ParsedHook};
use crate::status::{StatusContext, StatusEvent};
use proto::{HookSource, Runtime};
use std::time::Instant;

#[derive(Debug, Default)]
pub struct AgentState {
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub signals_seen: bool,
    pub hooks_seen: bool,
}

pub struct HookOutcome {
    pub status_event: Option<StatusEvent>,
    pub changed: bool,
}

impl AgentState {
    pub fn context(&self, focused: bool) -> StatusContext {
        StatusContext {
            focused,
            signals_seen: self.signals_seen,
            hooks_seen: self.hooks_seen,
        }
    }

    pub fn on_hook(&mut self, runtime: Runtime, hook: &ParsedHook, _now: Instant) -> HookOutcome {
        let mut changed = false;
        if runtime == Runtime::Claude
            && (hook.kind == HookKind::SessionStart || self.session_id.is_none())
            && self.session_id != hook.session_id
        {
            self.session_id.clone_from(&hook.session_id);
            changed = true;
        }
        let tool = match hook.kind {
            HookKind::PreToolUse if hook.agent_id.is_none() => Some(hook.tool_name.clone()),
            HookKind::PostToolUse if hook.agent_id.is_none() => Some(None),
            HookKind::Stop => Some(None),
            _ => None,
        };
        if let Some(tool) = tool
            && self.tool != tool
        {
            self.tool = tool;
            changed = true;
        }
        self.signals_seen = true;
        self.hooks_seen |= matches!(hook.source, HookSource::Claude | HookSource::CodexHook);
        HookOutcome {
            status_event: hook.status_event(),
            changed,
        }
    }
}
