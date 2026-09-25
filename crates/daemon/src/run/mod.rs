//! The orchestration engine, milestone 8a (`docs/milestones/M8a-orchestration-engine-core.md`).
//!
//! Design decision 1: the engine lives inside the daemon, not a new crate, because it
//! needs the window manager, the git runner, the registry and the data directory. Design
//! decision 2 draws the pure/impure line module by module; every submodule's own doc
//! comment says which side of that line it is on.
//!
//! - **The plan model (pure).** `model.rs` (the run, its tasks and rounds), `plan.rs`
//!   (parsing, profile and limit resolution, `build_run`, the run-id slug), `validate.rs`
//!   with `validate_graph.rs` (task resolution and the cross-task rules, decisions
//!   8–13), `edits.rs` (plan edits, decision 13), `globs.rs` (`owns` globs, decisions 11
//!   and 56), `roster.rs` (reviewer and escalation policy, decisions 23, 35 and 39),
//!   `contract.rs` and `messages.rs` (contracts, prompts and message texts), `env.rs`
//!   (the profile environment), `role_launch.rs` (session specs and ids), `reach.rs`
//!   (which runtimes a run can reach), `snapshot.rs` (decision 47's snapshot) and
//!   `report.rs` with `report_escape.rs` and `report_task.rs` (the run report).
//! - **The reducer (pure).** `engine/`: every event in, effects out.
//! - **The driver and I/O.** `driver/` (`RunService`: executes effects in decision 43's
//!   order), `journal.rs` (`run.json` and the intent journal), `reconcile/` (decision
//!   44's check on start), `git/` (every engine git command and the per-repository write
//!   queue), `exec.rs` (the engine's bounded shells), `proof.rs` (the fail-to-pass
//!   proof), and `confine.rs`, `confine_cache.rs` and `seatbelt.rs` (confinement of
//!   checks and sandbox profiles).

pub mod confine;
mod confine_cache;
pub mod contract;
pub mod driver;
pub mod edits;
pub mod engine;
pub mod env;
pub mod exec;
pub mod git;
pub mod globs;
pub mod journal;
pub mod messages;
pub mod model;
pub mod plan;
pub mod proof;
pub mod reach;
pub mod reconcile;
pub mod report;
mod report_escape;
mod report_task;
pub mod role_launch;
pub mod roster;
pub mod seatbelt;
pub mod snapshot;
pub mod validate;
mod validate_graph;

#[cfg(test)]
mod test_support;
