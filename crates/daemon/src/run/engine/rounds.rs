//! A new agent round of a task, moved out of `dispatch.rs` (task M9.9, no behaviour
//! change), whose size limit milestone 9 would otherwise pass. Pure (design decision
//! 2).

use proto::AgentRole;

use crate::run::model::{AgentRound, OpId};

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
    }
}
