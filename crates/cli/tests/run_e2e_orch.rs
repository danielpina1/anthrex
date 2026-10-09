//! Milestone 9 task M9.16: end to end, through a real daemon, with `fake-agent` as the
//! orchestrator in its PTY (`orchestrator-run-1`), as the run scouts, workers and
//! reviewers, and as the triage decider; no real agent. The plan path and its gate,
//! steering by typing, the orchestrator's reactions to blocked work, `run promote`,
//! what the orchestrator may not do, wake-ups beside typing, worker messages, refresh
//! and task notes, a restart, the user's messages, the goal form's request and the
//! routing history; and (milestone 9.0.6) a run driven through the TUI's own
//! requests. The scenarios live in `run_e2e_orch/`.

mod support;

#[path = "run_e2e_orch/alerts.rs"]
mod alerts;
#[path = "run_e2e_orch/common.rs"]
mod common;
#[path = "run_e2e_orch/gate.rs"]
mod gate;
#[path = "run_e2e_orch/messages.rs"]
mod messages;
#[path = "run_e2e_orch/promote.rs"]
mod promote;
#[path = "run_e2e_orch/records.rs"]
mod records;
#[path = "run_e2e_orch/steer.rs"]
mod steer;
#[path = "run_e2e_orch/tui_flow.rs"]
mod tui_flow;
