//! Per-session facts, separate from the window lifecycle and status transition table.

use crate::hooks::{HookKind, ParsedHook};
use crate::status::{CodexTitle, StatusContext, StatusEvent};
use crate::subagents::SubagentTracker;
use proto::{HookSource, Runtime};
use std::time::Instant;

#[derive(Debug, Default)]
pub struct AgentState {
    pub session_id: Option<String>,
    pub tool: Option<String>,
    pub signals_seen: bool,
    pub hooks_seen: bool,
    pub last_title: Option<CodexTitle>,
    pub subagents: SubagentTracker,
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

    pub fn on_hook(&mut self, runtime: Runtime, hook: &ParsedHook, now: Instant) -> HookOutcome {
        // A child notify must not mutate even the signal flags or tracker.
        if runtime == Runtime::Codex
            && hook.source == HookSource::CodexNotify
            && self.session_id.is_some()
            && hook.session_id != self.session_id
        {
            return HookOutcome {
                status_event: None,
                changed: false,
            };
        }
        let mut changed = self.subagents.apply(runtime, hook, now);
        if runtime == Runtime::Claude
            && (hook.kind == HookKind::SessionStart || self.session_id.is_none())
            && self.session_id != hook.session_id
        {
            self.session_id.clone_from(&hook.session_id);
            changed = true;
        }
        if runtime == Runtime::Codex
            && self.session_id.is_none()
            && hook.session_id.is_some()
            && (hook.source == HookSource::CodexNotify
                || (hook.source == HookSource::CodexHook
                    && hook.kind == HookKind::SessionStart
                    && hook.agent_id.is_none()))
        {
            self.session_id.clone_from(&hook.session_id);
            changed = true;
        }
        let tool = match hook.kind {
            HookKind::PreToolUse if hook.agent_id.is_none() => Some(hook.tool_name.clone()),
            HookKind::PostToolUse if hook.agent_id.is_none() => Some(None),
            HookKind::Stop if hook.agent_id.is_none() => Some(None),
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
            status_event: if runtime == Runtime::Codex
                && hook.source == HookSource::CodexHook
                && hook.kind == HookKind::Stop
                && hook.agent_id.is_some()
            {
                None
            } else {
                hook.status_event()
            },
            changed,
        }
    }

    pub fn on_title(&mut self, runtime: Runtime, title: &str) -> Option<StatusEvent> {
        if runtime != Runtime::Codex {
            return None;
        }
        let Some(title) = CodexTitle::parse(title) else {
            tracing::debug!(%title, "ignored unrecognised Codex title");
            return None;
        };
        if self.last_title == Some(title) {
            return None;
        }
        self.last_title = Some(title);
        self.signals_seen = true;
        Some(StatusEvent::Title(title))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(source: HookSource, kind: HookKind, session: &str, agent: Option<&str>) -> ParsedHook {
        ParsedHook {
            source,
            kind,
            session_id: Some(session.into()),
            agent_id: agent.map(str::to_owned),
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

    #[test]
    fn codex_session_id_is_written_once() {
        let mut state = AgentState::default();
        let now = Instant::now();
        state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexHook,
                HookKind::SessionStart,
                "child",
                Some("child"),
            ),
            now,
        );
        assert_eq!(state.session_id, None);
        state.on_hook(
            Runtime::Codex,
            &hook(HookSource::CodexHook, HookKind::SessionStart, "root", None),
            now,
        );
        assert_eq!(state.session_id.as_deref(), Some("root"));
        state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexHook,
                HookKind::SessionStart,
                "another",
                None,
            ),
            now,
        );
        assert_eq!(state.session_id.as_deref(), Some("root"));
    }

    #[test]
    fn notify_sets_the_session_when_no_hook_did() {
        let mut state = AgentState::default();
        state.on_hook(
            Runtime::Codex,
            &hook(HookSource::CodexHook, HookKind::PreToolUse, "ignored", None),
            Instant::now(),
        );
        assert_eq!(state.session_id, None);
        state.on_hook(
            Runtime::Codex,
            &hook(HookSource::CodexNotify, HookKind::TurnComplete, "t1", None),
            Instant::now(),
        );
        assert_eq!(state.session_id.as_deref(), Some("t1"));
        assert!(state.signals_seen);
    }

    #[test]
    fn notify_from_another_thread_is_ignored() {
        let mut state = AgentState {
            session_id: Some("root".into()),
            tool: Some("shell".into()),
            ..Default::default()
        };
        let outcome = state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexNotify,
                HookKind::TurnComplete,
                "child",
                None,
            ),
            Instant::now(),
        );
        assert_eq!(outcome.status_event, None);
        assert!(!outcome.changed);
        assert!(!state.signals_seen);
        assert!(!state.hooks_seen);
        assert_eq!(state.session_id.as_deref(), Some("root"));
        assert_eq!(state.tool.as_deref(), Some("shell"));
    }

    #[test]
    fn child_stop_does_not_end_the_root_turn() {
        let mut state = AgentState::default();
        let now = Instant::now();
        state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexHook,
                HookKind::SubagentStart,
                "root",
                Some("child"),
            ),
            now,
        );
        let outcome = state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexHook,
                HookKind::Stop,
                "child",
                Some("child"),
            ),
            now,
        );
        assert_eq!(outcome.status_event, None);
        assert!(state.signals_seen && state.hooks_seen);
        assert_eq!(state.subagents.infos(now)[0].id, "child");
        let stopped = state.on_hook(
            Runtime::Codex,
            &hook(
                HookSource::CodexHook,
                HookKind::SubagentStop,
                "child",
                Some("child"),
            ),
            now,
        );
        assert!(stopped.changed);
        assert_eq!(
            state.subagents.infos(now)[0].state,
            proto::SubagentState::Done
        );
    }

    #[test]
    fn claude_session_id_follows_the_latest_session_start() {
        let mut state = AgentState::default();
        for session in ["first", "second"] {
            state.on_hook(
                Runtime::Claude,
                &hook(HookSource::Claude, HookKind::SessionStart, session, None),
                Instant::now(),
            );
            assert_eq!(state.session_id.as_deref(), Some(session));
        }
    }

    #[test]
    fn repeated_titles_are_dropped() {
        let mut state = AgentState::default();
        assert_eq!(
            state.on_title(Runtime::Codex, "Ready"),
            Some(StatusEvent::Title(CodexTitle::Ready))
        );
        assert_eq!(state.on_title(Runtime::Codex, "Ready"), None);
        assert_eq!(state.on_title(Runtime::Codex, " Ready "), None);
        assert_eq!(
            state.on_title(Runtime::Codex, "Working"),
            Some(StatusEvent::Title(CodexTitle::Working))
        );
        assert_eq!(
            state.on_title(Runtime::Codex, "Ready"),
            Some(StatusEvent::Title(CodexTitle::Ready))
        );
    }

    #[test]
    fn a_recognised_title_disables_the_fallback() {
        let mut state = AgentState::default();
        assert_eq!(state.on_title(Runtime::Codex, "unknown"), None);
        assert!(!state.signals_seen);
        state.on_title(Runtime::Codex, "Working");
        assert!(state.signals_seen);
        assert!(!state.hooks_seen);
        assert_eq!(
            crate::status::next(
                proto::Status::Working,
                StatusEvent::Quiet,
                Runtime::Codex,
                state.context(false)
            ),
            proto::Status::Working
        );
    }

    #[test]
    fn titles_from_other_runtimes_are_ignored() {
        for runtime in [Runtime::Claude, Runtime::Shell] {
            let mut state = AgentState::default();
            assert_eq!(state.on_title(runtime, "Ready"), None);
            assert!(!state.signals_seen);
            assert_eq!(state.last_title, None);
        }
    }
}
