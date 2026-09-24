//! The orchestration engine, milestone 8a (`docs/milestones/M8a-orchestration-engine-core.md`).
//!
//! Design decision 1: the engine lives inside the daemon, not a new crate, because it
//! needs the window manager, the git runner, the registry and the data directory. Design
//! decision 2 draws the pure/impure line module by module; every submodule's own doc
//! comment says which side of that line it is on. This task (M8a.4) adds `globs.rs`
//! (`owns`-glob rules, decision 11 and decision 56's literal-name check) and `roster.rs`
//! (the reviewer and escalation policy, decisions 23, 35 and 39), plus `model.rs`, which
//! for now holds only `ReviewLevel` — `roster::pick_reviewer` needs it, and the rest of
//! the pure model arrives in M8a.5 (refresh note C41). M8a.5 adds `plan.rs` (parsing,
//! profile and limit resolution, `build_run`, the run-id slug), `validate.rs` (task
//! resolution, decisions 8–10) with `validate_graph.rs` (the cross-task rules,
//! decisions 11–13), `env.rs`
//! (the profile environment) and the rest of `model.rs`. M8a.6 adds `edits.rs` (plan
//! edits, decision 13) and starts `contract.rs` with the two message texts edits need.
//! M8a.8 adds `git/` (preflight, worktrees, the done check and the per-repository
//! write queue; blocking I/O) and `contract.rs`'s `REVIEW_DIFF_MAX` diff clamp.
//! M8a.10 adds `exec.rs` (the engine's bounded, scrubbed shells: `setup` and `check`)
//! and `proof.rs` (the fail-to-pass test proof), both blocking I/O. M8a.11 adds the
//! pure reducer `engine/` (start, the plan gate, the scheduler and dispatch),
//! `role_launch.rs` (session specs and ids), `messages.rs` (decision 29's clamp and
//! join), `snapshot.rs` (decision 47's snapshot), and the contracts and prompts in
//! `contract.rs`.

pub mod contract;
pub mod edits;
pub mod engine;
pub mod env;
pub mod exec;
pub mod git;
pub mod globs;
pub mod messages;
pub mod model;
pub mod plan;
pub mod proof;
pub mod report;
mod report_escape;
mod report_task;
pub mod role_launch;
pub mod roster;
pub mod snapshot;
pub mod validate;
mod validate_graph;

#[cfg(test)]
mod test_support;
