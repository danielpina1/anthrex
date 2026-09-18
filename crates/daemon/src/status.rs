//! Pure status transitions from PTY, hook, notify, and title signals.

use proto::{Runtime, Status};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusEvent {
    /// Bytes arrived from the PTY.
    Output,
    /// No output for `manager::QUIET_AFTER`.
    Quiet,
    /// The program rang the terminal bell.
    Bell,
    /// A client started viewing this window.
    Focused,
    /// A client typed into this window.
    InputSent,
    /// The child process ended.
    Exited,
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PermissionRequest,
    PermissionPrompt,
    IdlePrompt,
    Stop,
    CodexNotify,
    Title(CodexTitle),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexTitle {
    Starting,
    Working,
    Thinking,
    Waiting,
    Ready,
}

impl CodexTitle {
    pub fn parse(title: &str) -> Option<Self> {
        match title.trim() {
            "Starting" => Some(Self::Starting),
            "Working" => Some(Self::Working),
            "Thinking" => Some(Self::Thinking),
            "Waiting" => Some(Self::Waiting),
            "Ready" => Some(Self::Ready),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusContext {
    pub focused: bool,
    pub signals_seen: bool,
    pub hooks_seen: bool,
}

/// Computes one status transition without performing I/O or mutating other state.
///
/// The first matching decision-28 row wins, refining the core spec section 3.4:
///
/// | Signal | Applicable runtime/context | Transition |
/// |---|---|---|
/// | exit | all | `Exited` is terminal |
/// | bell | Claude after hooks / all others | unchanged / `Attention` |
/// | focus, input | all | clear `Done`, or clear `Attention` |
/// | output, quiet | Shell or agent before its first signal | fallback activity transitions |
/// | hook lifecycle | Claude/Codex | start, work, permission, idle, and stop transitions |
/// | notify/title | Codex | unchanged until the M3.10 parser/status integration |
///
/// Session ids, active tools, and sub-agents are recorded by `AgentState`, not here.
pub fn next(current: Status, event: StatusEvent, runtime: Runtime, ctx: StatusContext) -> Status {
    use Status::*;
    use StatusEvent as E;
    match (runtime, current, event) {
        (_, Exited, _) | (_, _, E::Exited) => Exited,
        (Runtime::Claude, status, E::Bell) if ctx.hooks_seen => status,
        (_, _, E::Bell) => Attention,
        (_, Done, E::Focused) => Idle,
        (_, Attention, E::InputSent) => Working,
        (Runtime::Shell, Starting | Idle | Done, E::Output) => Working,
        (Runtime::Shell, Working, E::Quiet) => Idle,
        (Runtime::Claude | Runtime::Codex, Starting | Idle | Done, E::Output)
            if !ctx.signals_seen =>
        {
            Working
        }
        (Runtime::Claude | Runtime::Codex, Working, E::Quiet) if !ctx.signals_seen => Idle,
        (Runtime::Claude | Runtime::Codex, status, E::Output | E::Quiet) if ctx.signals_seen => {
            status
        }
        (Runtime::Claude | Runtime::Codex, status, E::SessionStart) => {
            if !ctx.signals_seen && status != Attention {
                Idle
            } else {
                status
            }
        }
        (Runtime::Claude | Runtime::Codex, _, E::UserPromptSubmit) => Working,
        (Runtime::Claude | Runtime::Codex, Attention, E::PreToolUse) => Attention,
        (Runtime::Claude | Runtime::Codex, _, E::PreToolUse) => Working,
        (Runtime::Claude | Runtime::Codex, status, E::PostToolUse) => status,
        (Runtime::Claude | Runtime::Codex, _, E::PermissionRequest | E::PermissionPrompt) => {
            Attention
        }
        (Runtime::Claude, Starting | Working, E::IdlePrompt) => Idle,
        (Runtime::Claude, status, E::IdlePrompt) => status,
        (Runtime::Claude | Runtime::Codex, _, E::Stop) => completion(ctx),
        (_, status, _) => status,
    }
}

fn completion(ctx: StatusContext) -> Status {
    if ctx.focused {
        Status::Idle
    } else {
        Status::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::Runtime::{Claude, Codex, Shell};
    use proto::Status::*;

    const EMPTY: StatusContext = StatusContext {
        focused: false,
        signals_seen: false,
        hooks_seen: false,
    };

    #[test]
    fn output_makes_a_window_working_and_quiet_makes_it_idle() {
        assert_eq!(next(Starting, StatusEvent::Output, Shell, EMPTY), Working);
        assert_eq!(next(Idle, StatusEvent::Output, Shell, EMPTY), Working);
        assert_eq!(next(Done, StatusEvent::Output, Shell, EMPTY), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Shell, EMPTY), Idle);
        assert_eq!(next(Idle, StatusEvent::Quiet, Shell, EMPTY), Idle);
    }

    #[test]
    fn bell_demands_attention_until_input_is_sent() {
        assert_eq!(next(Working, StatusEvent::Bell, Shell, EMPTY), Attention);
        assert_eq!(
            next(Attention, StatusEvent::Output, Shell, EMPTY),
            Attention
        );
        assert_eq!(
            next(Attention, StatusEvent::InputSent, Shell, EMPTY),
            Working
        );
        assert_eq!(next(Idle, StatusEvent::InputSent, Shell, EMPTY), Idle);
    }

    #[test]
    fn focusing_clears_done() {
        assert_eq!(next(Done, StatusEvent::Focused, Shell, EMPTY), Idle);
        assert_eq!(next(Working, StatusEvent::Focused, Shell, EMPTY), Working);
    }

    #[test]
    fn exited_is_terminal() {
        assert_eq!(next(Working, StatusEvent::Exited, Shell, EMPTY), Exited);
        assert_eq!(next(Exited, StatusEvent::Output, Shell, EMPTY), Exited);
        assert_eq!(next(Exited, StatusEvent::Bell, Shell, EMPTY), Exited);
    }

    #[test]
    fn fallback_rules_apply_to_every_runtime_in_this_milestone() {
        assert_eq!(next(Starting, StatusEvent::Output, Claude, EMPTY), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Claude, EMPTY), Idle);
    }

    #[test]
    fn session_start_makes_a_fresh_window_idle() {
        assert_eq!(
            next(Starting, StatusEvent::SessionStart, Claude, EMPTY),
            Idle
        );
        assert_eq!(
            next(Working, StatusEvent::SessionStart, Claude, EMPTY),
            Idle
        );
        assert_eq!(
            next(Attention, StatusEvent::SessionStart, Claude, EMPTY),
            Attention
        );
    }

    #[test]
    fn session_start_after_signals_changes_nothing() {
        let ctx = StatusContext {
            signals_seen: true,
            ..EMPTY
        };
        assert_eq!(
            next(Starting, StatusEvent::SessionStart, Claude, ctx),
            Starting
        );
        assert_eq!(
            next(Working, StatusEvent::SessionStart, Codex, ctx),
            Working
        );
    }

    #[test]
    fn prompt_submit_and_pre_tool_use_mean_working() {
        for event in [StatusEvent::UserPromptSubmit, StatusEvent::PreToolUse] {
            assert_eq!(next(Idle, event, Claude, EMPTY), Working);
            assert_eq!(next(Done, event, Codex, EMPTY), Working);
        }
    }

    #[test]
    fn pre_tool_use_keeps_attention() {
        assert_eq!(
            next(Attention, StatusEvent::PreToolUse, Claude, EMPTY),
            Attention
        );
    }

    #[test]
    fn post_tool_use_changes_no_status() {
        for status in [Starting, Working, Idle, Done, Attention] {
            assert_eq!(
                next(status, StatusEvent::PostToolUse, Claude, EMPTY),
                status
            );
        }
    }

    #[test]
    fn permission_request_and_prompt_mean_attention() {
        assert_eq!(
            next(Working, StatusEvent::PermissionRequest, Claude, EMPTY),
            Attention
        );
        assert_eq!(
            next(Idle, StatusEvent::PermissionPrompt, Claude, EMPTY),
            Attention
        );
        assert_eq!(
            next(Done, StatusEvent::PermissionRequest, Codex, EMPTY),
            Attention
        );
    }

    #[test]
    fn idle_prompt_keeps_done_and_attention() {
        assert_eq!(next(Starting, StatusEvent::IdlePrompt, Claude, EMPTY), Idle);
        assert_eq!(next(Working, StatusEvent::IdlePrompt, Claude, EMPTY), Idle);
        assert_eq!(next(Done, StatusEvent::IdlePrompt, Claude, EMPTY), Done);
        assert_eq!(
            next(Attention, StatusEvent::IdlePrompt, Claude, EMPTY),
            Attention
        );
        assert_eq!(next(Idle, StatusEvent::IdlePrompt, Claude, EMPTY), Idle);
    }

    #[test]
    fn stop_is_done_unless_viewed() {
        assert_eq!(next(Working, StatusEvent::Stop, Claude, EMPTY), Done);
        let viewed = StatusContext {
            focused: true,
            ..EMPTY
        };
        assert_eq!(next(Working, StatusEvent::Stop, Claude, viewed), Idle);
        assert_eq!(next(Attention, StatusEvent::Stop, Codex, viewed), Idle);
    }

    #[test]
    fn bell_is_ignored_by_a_claude_window_with_hooks() {
        let with_hooks = StatusContext {
            hooks_seen: true,
            ..EMPTY
        };
        assert_eq!(
            next(Working, StatusEvent::Bell, Claude, with_hooks),
            Working
        );
        assert_eq!(next(Working, StatusEvent::Bell, Claude, EMPTY), Attention);
        assert_eq!(
            next(Working, StatusEvent::Bell, Shell, with_hooks),
            Attention
        );
    }

    #[test]
    fn output_and_quiet_drive_agents_only_until_the_first_signal() {
        assert_eq!(next(Idle, StatusEvent::Output, Claude, EMPTY), Working);
        assert_eq!(next(Working, StatusEvent::Quiet, Claude, EMPTY), Idle);

        let after_signal = StatusContext {
            signals_seen: true,
            ..EMPTY
        };
        assert_eq!(next(Idle, StatusEvent::Output, Claude, after_signal), Idle);
        assert_eq!(
            next(Working, StatusEvent::Quiet, Claude, after_signal),
            Working
        );
        assert_eq!(next(Done, StatusEvent::Output, Codex, after_signal), Done);
    }

    #[test]
    fn hook_events_do_not_move_shell_windows() {
        for event in [
            StatusEvent::SessionStart,
            StatusEvent::UserPromptSubmit,
            StatusEvent::PreToolUse,
            StatusEvent::PostToolUse,
            StatusEvent::PermissionRequest,
            StatusEvent::PermissionPrompt,
            StatusEvent::IdlePrompt,
            StatusEvent::Stop,
            StatusEvent::CodexNotify,
            StatusEvent::Title(CodexTitle::Starting),
        ] {
            assert_eq!(next(Idle, event, Shell, EMPTY), Idle, "{event:?}");
        }
    }

    #[test]
    fn codex_specific_events_are_deferred() {
        let cases = [
            (Working, StatusEvent::CodexNotify),
            (Idle, StatusEvent::Title(CodexTitle::Starting)),
            (Idle, StatusEvent::Title(CodexTitle::Working)),
            (Idle, StatusEvent::Title(CodexTitle::Thinking)),
            (Idle, StatusEvent::Title(CodexTitle::Waiting)),
            (Working, StatusEvent::Title(CodexTitle::Ready)),
        ];
        for (status, event) in cases {
            assert_eq!(next(status, event, Codex, EMPTY), status, "{event:?}");
        }
    }

    #[test]
    fn codex_titles_parse_exact_words() {
        assert_eq!(CodexTitle::parse(" Starting "), Some(CodexTitle::Starting));
        assert_eq!(CodexTitle::parse("Working"), Some(CodexTitle::Working));
        assert_eq!(CodexTitle::parse("Thinking"), Some(CodexTitle::Thinking));
        assert_eq!(CodexTitle::parse("Waiting"), Some(CodexTitle::Waiting));
        assert_eq!(CodexTitle::parse("Ready"), Some(CodexTitle::Ready));
        assert_eq!(CodexTitle::parse("working"), None);
        assert_eq!(CodexTitle::parse("Working now"), None);
    }
}
