//! Milestone 9.6 task M9.6.20: the design flow end to end (DF §12), through the real
//! binary and a real daemon on temporary paths. `fake-agent` is the orchestrator in its
//! PTY (`orchestrator-run-1`), both brainstormers (`brainstormer-<label>-<n>`), the
//! document reviewers (`doc_reviewer-<doc>-r<k>-<n>`), the workers, their reviewers and
//! the triage decider; no real agent and no `gh` (a `pr` run is on `FakeHost`). The
//! harness pins every `*_BIN`, and puts the config, socket and data dir under its temp
//! dir. The scenarios live in `run_e2e_design/`; a round's amendment is
//! `run_e2e_design_rounds.rs`.

mod support;

#[path = "run_e2e_design/agents.rs"]
mod agents;
#[path = "run_e2e_design/common.rs"]
mod common;
#[path = "run_e2e_design/gates.rs"]
mod gates;
#[path = "run_e2e_design/off.rs"]
mod off;
#[path = "run_e2e_design/pr.rs"]
mod pr;
#[path = "run_e2e_design/revisions.rs"]
mod revisions;
