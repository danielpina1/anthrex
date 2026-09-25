//! Decision 27's translation of the manager's feed into the engine's signals (M8a.22):
//! each `WindowSignal` of a headless window becomes at most one `AgentSignal`. Pure.
//!
//! - `Init`, `TurnStarted`, `ToolUse`, `TurnEnded` (with its denials' tool names),
//!   `ApiRetry`, `PermissionDenied`, `ProcessStarted` and `ProcessExited` map one to
//!   one; the pids come from `WindowSignal.pid`. `killed_by_engine` is filled in by the
//!   event loop, which knows which processes it killed, so it is `false` here.
//! - A real `SubagentStart` or `SubagentStop` hook with an agent id maps one to one.
//! - An `Unprompted` event (ruling T7-N1) is never the delivered turn's end: its
//!   `TurnEnded` is `Spend` (its usage still counts), anything else activity.
//! - Text, tool results, compaction and the other recognised lines are `Activity`, at
//!   most one per window per [`ACTIVITY_EVERY`]: activity only resets the stall clock,
//!   and a session can print many lines a second.
//! - Stderr lines, unparsed lines and diagnostics are not activity (the M8a.17 carry's
//!   open question): Claude writes hook-progress lines constantly, and a session that
//!   only complains on stderr is not making progress. Nor is a failed API turn's own
//!   message (`ApiErrorText`, M8a.24): it would end the retry streak that ran into the
//!   failure, and count one rate-limit event twice (decision 32).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::headless::SessionEvent;
use crate::hooks::HookKind;
use crate::manager::{WindowSignal, WindowSignalKind};
use crate::run::engine::AgentSignal;

/// At most one `Activity` per window this often.
pub const ACTIVITY_EVERY: Duration = Duration::from_secs(1);

/// The engine's signal for `signal`, if it has one. `last_activity` is the forwarder's
/// own record of each window's last forwarded `Activity`.
pub fn translate(
    signal: &WindowSignal,
    last_activity: &mut HashMap<u32, Instant>,
    now: Instant,
) -> Option<AgentSignal> {
    let pid = signal.pid.unwrap_or(0);
    let event = match &signal.kind {
        WindowSignalKind::Hook { kind, agent_id } => {
            let agent_id = agent_id.clone()?;
            return match kind {
                HookKind::SubagentStart => Some(AgentSignal::SubagentStart { agent_id }),
                HookKind::SubagentStop => Some(AgentSignal::SubagentStop { agent_id }),
                _ => None,
            };
        }
        WindowSignalKind::Unprompted(SessionEvent::TurnEnded {
            usage: Some(usage), ..
        }) => return Some(AgentSignal::Spend { usage: *usage }),
        WindowSignalKind::Unprompted(_) => return activity(signal.window_id, last_activity, now),
        WindowSignalKind::Session(event) => event,
    };
    Some(match event {
        SessionEvent::Init { session_id, .. } => AgentSignal::Init {
            session_id: session_id.clone(),
        },
        SessionEvent::TurnStarted => AgentSignal::TurnStarted,
        SessionEvent::ToolUse { name, .. } => AgentSignal::ToolUse { name: name.clone() },
        SessionEvent::TurnEnded {
            outcome,
            usage,
            denials,
        } => AgentSignal::TurnEnded {
            outcome: outcome.clone(),
            usage: *usage,
            denials: denials.clone(),
        },
        SessionEvent::ApiRetry {
            error, delay_ms, ..
        } => AgentSignal::ApiRetry {
            error: error.clone(),
            delay_ms: *delay_ms,
        },
        SessionEvent::PermissionDenied { tool, reason } => AgentSignal::PermissionDenied {
            tool: tool.clone(),
            reason: reason.clone(),
        },
        SessionEvent::ProcessStarted { pid } => AgentSignal::ProcessStarted { pid: *pid },
        SessionEvent::ProcessExited { code, .. } => AgentSignal::ProcessExited {
            code: *code,
            killed_by_engine: false,
            pid,
        },
        SessionEvent::StderrLine { .. }
        | SessionEvent::Unknown { .. }
        | SessionEvent::Diagnostic { .. }
        | SessionEvent::ApiErrorText { .. } => return None,
        SessionEvent::UserText { .. }
        | SessionEvent::AssistantText { .. }
        | SessionEvent::ToolResult { .. }
        | SessionEvent::Compacted
        | SessionEvent::Other { .. } => return activity(signal.window_id, last_activity, now),
    })
}

fn activity(
    window_id: u32,
    last_activity: &mut HashMap<u32, Instant>,
    now: Instant,
) -> Option<AgentSignal> {
    match last_activity.get(&window_id) {
        Some(at) if now.saturating_duration_since(*at) < ACTIVITY_EVERY => None,
        _ => {
            // Ruling T22-minors, m6: only the windows active in the last second are
            // remembered, so a removed window's entry does not outlive it.
            last_activity.retain(|_, at| now.saturating_duration_since(*at) < ACTIVITY_EVERY);
            last_activity.insert(window_id, now);
            Some(AgentSignal::Activity)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::TurnOutcome;
    use proto::TokenUsage;

    fn session(window_id: u32, pid: u32, event: SessionEvent) -> WindowSignal {
        WindowSignal {
            window_id,
            pid: Some(pid),
            kind: WindowSignalKind::Session(event),
        }
    }

    fn one(signal: WindowSignal) -> Option<AgentSignal> {
        translate(&signal, &mut HashMap::new(), Instant::now())
    }

    const USAGE: TokenUsage = TokenUsage {
        input: 11,
        output: 23,
        cache_read: 37,
        cache_write: 41,
    };

    #[test]
    fn session_events_map_one_to_one_with_the_feeds_pid() {
        assert_eq!(
            one(session(3, 7, SessionEvent::ProcessStarted { pid: 7 })),
            Some(AgentSignal::ProcessStarted { pid: 7 })
        );
        assert_eq!(
            one(session(
                3,
                7,
                SessionEvent::ProcessExited {
                    code: Some(1),
                    signal: None
                }
            )),
            Some(AgentSignal::ProcessExited {
                code: Some(1),
                killed_by_engine: false,
                pid: 7
            })
        );
        assert_eq!(
            one(session(
                3,
                7,
                SessionEvent::TurnEnded {
                    outcome: TurnOutcome::Completed,
                    usage: Some(USAGE),
                    denials: vec!["Bash".into()],
                }
            )),
            Some(AgentSignal::TurnEnded {
                outcome: TurnOutcome::Completed,
                usage: Some(USAGE),
                denials: vec!["Bash".into()],
            })
        );
        assert_eq!(
            one(session(
                3,
                7,
                SessionEvent::ApiRetry {
                    error: "rate_limit".into(),
                    attempt: 2,
                    delay_ms: 1500
                }
            )),
            Some(AgentSignal::ApiRetry {
                error: "rate_limit".into(),
                delay_ms: 1500
            })
        );
        assert_eq!(
            one(session(
                3,
                7,
                SessionEvent::Init {
                    session_id: "s".into(),
                    model: None,
                    mcp_ok: None
                }
            )),
            Some(AgentSignal::Init {
                session_id: "s".into()
            })
        );
        assert_eq!(
            one(session(
                3,
                7,
                SessionEvent::ToolUse {
                    id: "t".into(),
                    name: "Bash".into(),
                    input: serde_json::Value::Null,
                    parent: None
                }
            )),
            Some(AgentSignal::ToolUse {
                name: "Bash".into()
            })
        );
    }

    #[test]
    fn an_unprompted_turn_end_is_spend_never_a_turn_end() {
        let ended = SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(USAGE),
            denials: Vec::new(),
        };
        let signal = WindowSignal {
            window_id: 3,
            pid: Some(7),
            kind: WindowSignalKind::Unprompted(ended),
        };
        assert_eq!(one(signal), Some(AgentSignal::Spend { usage: USAGE }));
        let quiet = WindowSignal {
            window_id: 3,
            pid: Some(7),
            kind: WindowSignalKind::Unprompted(SessionEvent::TurnEnded {
                outcome: TurnOutcome::Completed,
                usage: None,
                denials: Vec::new(),
            }),
        };
        assert_eq!(one(quiet), Some(AgentSignal::Activity));
    }

    #[test]
    fn subagent_hooks_map_and_other_hooks_do_not() {
        let hook = |kind, agent_id: Option<&str>| WindowSignal {
            window_id: 3,
            pid: None,
            kind: WindowSignalKind::Hook {
                kind,
                agent_id: agent_id.map(str::to_string),
            },
        };
        assert_eq!(
            one(hook(HookKind::SubagentStart, Some("a1"))),
            Some(AgentSignal::SubagentStart {
                agent_id: "a1".into()
            })
        );
        assert_eq!(
            one(hook(HookKind::SubagentStop, Some("a1"))),
            Some(AgentSignal::SubagentStop {
                agent_id: "a1".into()
            })
        );
        assert_eq!(one(hook(HookKind::SubagentStop, None)), None);
        assert_eq!(one(hook(HookKind::Stop, Some("a1"))), None);
    }

    #[test]
    fn activity_is_at_most_once_a_second_per_window_and_stderr_is_none() {
        let mut last = HashMap::new();
        let t0 = Instant::now();
        let text = |w| {
            session(
                w,
                7,
                SessionEvent::AssistantText {
                    text: "hi".into(),
                    parent: None,
                },
            )
        };
        assert_eq!(
            translate(&text(3), &mut last, t0),
            Some(AgentSignal::Activity)
        );
        assert_eq!(
            translate(&text(3), &mut last, t0 + Duration::from_millis(500)),
            None
        );
        assert_eq!(
            translate(&text(4), &mut last, t0),
            Some(AgentSignal::Activity)
        );
        assert_eq!(
            translate(&text(3), &mut last, t0 + ACTIVITY_EVERY),
            Some(AgentSignal::Activity)
        );
        // Ruling T22-minors, m6: a window quiet for a second is forgotten, so the map
        // never outgrows the windows active in the last second.
        assert_eq!(last.len(), 1, "{last:?}");
        for event in [
            SessionEvent::StderrLine { line: "x".into() },
            SessionEvent::Unknown { line: "x".into() },
            SessionEvent::Diagnostic { text: "x".into() },
            // M8a.24: a failed API turn's own message is part of the failure.
            SessionEvent::ApiErrorText { text: "x".into() },
        ] {
            assert_eq!(translate(&session(5, 7, event), &mut last, t0), None);
        }
    }
}
