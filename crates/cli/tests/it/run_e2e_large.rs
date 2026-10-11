//! Milestone 9 task M9.17: end to end, through a real daemon, with `fake-agent` as the
//! orchestrator in its PTY (`orchestrator-run-1`), as the sub-planners
//! (`planner-<epic>-1`), the run scouts, workers, reviewers and research sessions, and
//! as the triage decider; no real agent. The large path with sub-planners, a planner's
//! edit outside its area, a new epic held after approval, the per-epic integration
//! review, research and review goals, a daemon restart, OTLP metering of the
//! orchestrator and the role-routing history. The scenarios live in `run_e2e_large/`;
//! `common` is M9.16's, shared with `run_e2e_orch`.

#[allow(
    dead_code,
    reason = "run_e2e_orch's shared helpers; this binary uses only some of them"
)]
// Each binary mounted its own copy before the merge (milestone 9.12, decision 13).
#[allow(clippy::duplicate_mod)]
#[path = "run_e2e_orch/common.rs"]
mod common;
#[path = "run_e2e_large/epics.rs"]
mod epics;
#[path = "run_e2e_large/kinds.rs"]
mod kinds;
#[path = "run_e2e_large/large.rs"]
mod large;
#[path = "run_e2e_large/records.rs"]
mod records;
#[path = "run_e2e_large/restart.rs"]
mod restart;
