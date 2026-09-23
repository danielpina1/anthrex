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
//! (the profile environment) and the rest of `model.rs`.

pub mod env;
pub mod globs;
pub mod model;
pub mod plan;
pub mod roster;
pub mod validate;
mod validate_graph;

#[cfg(test)]
mod test_support;
