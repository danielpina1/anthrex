//! A new agent round of a task, moved out of `dispatch.rs` (task M9.9, no behaviour
//! change), whose size limit milestone 9 would otherwise pass, and milestone 9.0.5's
//! record of a round's latest action and last message (`note_tool`, `note_said`). Pure
//! (design decision 2).

use proto::safe_text::{multi_line, one_line};
use proto::{ACTIVITY_MAX, AgentRole, WORKER_SUMMARY_MAX};

use crate::run::model::{AgentRound, OpId};
use crate::run::snapshot_detail::cut;

/// The anthrex MCP server's tool prefix, dropped from an activity line.
const ANTHREX_TOOL_PREFIX: &str = "mcp__anthrex__";

/// A round whose first turn is open from its launch (decision 27: the reducer marks the
/// turn open when it delivers one).
pub(super) fn new_round(
    role: AgentRole,
    session: u32,
    route: proto::Route,
    op: OpId,
    session_id: Option<String>,
    now: u64,
) -> AgentRound {
    AgentRound {
        role,
        session,
        round: session,
        window_id: None,
        route,
        launch_op: op,
        session_id,
        pid: None,
        ended: false,
        started_at: now,
        ended_at: None,
        turn_open: true,
        turns: 1,
        turn_had_task_done: false,
        last_event: now,
        tool_calls: 0,
        rate_limited_until: None,
        rate_limited_since: None,
        sent_back_at: Vec::new(),
        in_retry_streak: false,
        open_subagents: Default::default(),
        denials: 0,
        usage: Default::default(),
        deaths: 0,
        fallback: Default::default(),
        stall: Default::default(),
        failed_turn: Default::default(),
        review_nudged: false,
        wrap_up_sent: false,
        retiring: false,
        delivery_failures: 0,
        delivery_retry_at: None,
        turn_denied: Vec::new(),
        excused_secs: 0,
        last_denial: None,
        fallback_waiting: false,
        carried: Vec::new(),
        failed_error: None,
        resume_op: None,
        count_op: None,
        count_failures: 0,
        count_retry_at: None,
        count_turn: 0,
        interrupted: false,
        relaunch: None,
        closed_pid: None,
        exited_pid: None,
        activity: None,
        last_text: None,
    }
}

/// Milestone 9.0.5 decision 3: a top-level tool call is the round's latest action,
/// `<tool> <target>` (the bare name when its summary is empty), the anthrex server's
/// prefix dropped. A sub-agent's call (`target: None`) changes nothing, as the
/// window's `tool` ignores it.
pub(super) fn note_tool(round: &mut AgentRound, name: &str, target: Option<&str>) {
    let Some(target) = target else { return };
    let name = name.strip_prefix(ANTHREX_TOOL_PREFIX).unwrap_or(name);
    set_activity(round, &format!("{name} {target}"));
}

/// Decisions 3 and 4: top-level text is the round's latest action (`says: ` and its
/// first non-blank line) and, for a worker, its last message, line breaks kept. A text
/// that is blank once sanitised (a Codex `agent_message` with no text) changes neither.
pub(super) fn note_said(round: &mut AgentRound, text: &str) {
    let first = text.lines().map(one_line).find(|l| !l.trim().is_empty());
    if let Some(first) = first {
        set_activity(round, &format!("says: {}", first.trim()));
    }
    let text = multi_line(text);
    if round.role == AgentRole::Worker && !text.trim().is_empty() {
        round.last_text = Some(cut(&text, WORKER_SUMMARY_MAX));
    }
}

/// One sanitised line of at most `ACTIVITY_MAX` characters, `…` included.
fn set_activity(round: &mut AgentRound, line: &str) {
    let line = one_line(line);
    round.activity = Some(cut(line.trim(), ACTIVITY_MAX - 1));
}
