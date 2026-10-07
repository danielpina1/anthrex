//! Shared fixtures for the `run` unit tests (M8a.5 onwards).

use std::path::PathBuf;

use super::model::{Run, Task};
use super::model_roles::RunModels;
use super::plan::{BuildContext, PlanError, Preflight, build_run, parse_plan};

/// Decision 7's example plan, verbatim in every value it sets. Its limits (4, 2, 3)
/// differ from their config defaults (3, 3, 2) and from each other; its route effort
/// (`high`) differs from policy's M default (`medium`); its budget (120/45) differs
/// from the M default (150/60).
pub const EXAMPLE_PLAN: &str = r#"
goal = "Add password reset"
max_writers = 4
max_readers = 2
max_bounces = 3

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "cargo test --workspace"
check_timeout_secs = 1800
single_test = "cargo test --workspace -- --exact {test}"
test_passed = 'test {test} \.\.\. ok'
setup = "cargo fetch"
generated = ["Cargo.lock"]
protected = ["docs/agents/**"]
[profile.env]
CARGO_TARGET_DIR = "{worktree}/target"

[[task]]
id = "t1"
title = "Reset token model"
kind = "code"
size = "M"
interface_change = false
test_mode = "tdd"
test_mode_reason = ""
owns = ["crates/auth/src/token.rs"]
deps = []
priority = 0
brief = "..."
acceptance = ["..."]
test_to_write = "token::expires_after_one_hour"
epic = "auth"
scout_refs = []
[task.route]
runtime = "claude"
model = "claude-sonnet-5"
strength = "standard"
effort = "high"
[task.budget]
tool_calls = 120
minutes = 45
tokens = 3000000
"#;

/// A profile with modules, hub, source, check, single_test and test_passed all set, so a
/// test that wants one of them missing overrides it explicitly.
pub const PROFILE: &str = r#"
goal = "Test goal"

[profile]
modules = ["crates/*"]
hub = ["crates/proto/**"]
source = ["crates/*/src/**"]
check = "cargo test"
single_test = "cargo test -- --exact {test}"
test_passed = 'test {test} \.\.\. ok'
"#;

pub const RUN_ID: &str = "add-password-reset-3f9a";

/// One `[[task]]` table. `owns` is TOML array text; `extra` is appended last, so it may
/// open `[task.route]` or `[task.budget]`.
pub fn task_toml(id: &str, size: &str, owns: &str, extra: &str) -> String {
    format!(
        "\n[[task]]\nid = \"{id}\"\ntitle = \"Title {id}\"\nsize = \"{size}\"\nowns = {owns}\nbrief = \"Brief {id}\"\nacceptance = [\"Accept {id}\"]\n{extra}\n"
    )
}

pub fn plan_with(profile: &str, tasks: &[String]) -> String {
    let mut text = profile.to_string();
    for t in tasks {
        text.push_str(t);
    }
    text
}

pub fn preflight() -> Preflight {
    Preflight {
        root: PathBuf::from("/tmp/x"),
        project: PathBuf::from("/tmp/p"),
        git_common_dir: PathBuf::from("/tmp/p/.git"),
        base_branch: "main".to_string(),
        base_sha: "b".repeat(40),
        protected_files: Vec::new(),
    }
}

pub fn build_full(
    text: &str,
    config: &config::Orchestrator,
    pre: Preflight,
) -> Result<Run, Vec<PlanError>> {
    build_full_tuned(text, config, pre, Default::default())
}

/// [`build_with`], with what a start froze from history (milestone 9.5 decision 12).
pub fn build_tuned(
    text: &str,
    config: &config::Orchestrator,
    tuning: super::refit::Tuned,
) -> Result<Run, Vec<PlanError>> {
    build_full_tuned(text, config, preflight(), tuning)
}

fn build_full_tuned(
    text: &str,
    config: &config::Orchestrator,
    pre: Preflight,
    tuning: super::refit::Tuned,
) -> Result<Run, Vec<PlanError>> {
    // Milestone 9.8: the role table as a start with no repository file freezes it.
    let models = (RunModels::resolve(&config.roles, None), Vec::new());
    build_frozen(text, config, pre, tuning, models)
}

/// [`build_with`], with the role table (and its start lines) a start froze
/// (`driver::build_models::freeze`).
pub fn build_with_models(
    text: &str,
    config: &config::Orchestrator,
    models: RunModels,
    models_log: Vec<String>,
) -> Result<Run, Vec<PlanError>> {
    build_frozen(
        text,
        config,
        preflight(),
        Default::default(),
        (models, models_log),
    )
}

fn build_frozen(
    text: &str,
    config: &config::Orchestrator,
    pre: Preflight,
    tuning: super::refit::Tuned,
    (models, models_log): (RunModels, Vec<String>),
) -> Result<Run, Vec<PlanError>> {
    let plan = parse_plan(text).unwrap_or_else(|e| panic!("fixture plan must parse: {e}"));
    build_run(
        plan,
        pre,
        BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config,
            testing: &config::Testing::default(),
            now: 1_000,
            yes: false,
            delivery: &config::Delivery::default(),
            tuning,
            models,
            models_log,
        },
    )
}

pub fn build_with(text: &str, config: &config::Orchestrator) -> Result<Run, Vec<PlanError>> {
    build_full(text, config, preflight())
}

pub fn build(text: &str) -> Result<Run, Vec<PlanError>> {
    build_with(text, &config::Orchestrator::default())
}

pub fn run_ok(text: &str) -> Run {
    match build(text) {
        Ok(run) => run,
        Err(errors) => panic!("expected a run, got errors: {}", show(&errors)),
    }
}

pub fn errors_of(text: &str) -> Vec<PlanError> {
    match build(text) {
        Ok(_) => panic!("expected errors, the run built"),
        Err(errors) => errors,
    }
}

pub fn show(errors: &[PlanError]) -> String {
    errors
        .iter()
        .map(|e| format!("[{}] {e}", e.rule))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Milestone 9.8: `run`'s role table with a Claude reviewer and test writer (no
/// fallbacks), each task's reviewer picked again from it: with Claude implementer rows
/// (the built-ins), a run whose sessions stay on Claude.
pub fn claude_rows(run: &mut Run) {
    use proto::models::{ModelRef, Role, RoleChoice};
    let mut models = run.limits.models().clone();
    for (role, model) in [
        (Role::Reviewer, "claude:claude-opus-5-5"),
        (Role::TestWriter, "claude:claude-sonnet-5"),
    ] {
        let model = ModelRef::parse(model).expect("a model");
        let row = RoleChoice {
            model,
            effort: None,
            fallback: None,
        };
        models.rows.insert(role, row);
    }
    for t in &mut run.tasks {
        if t.review_route.is_some() {
            t.review_route = Some(models.reviewer_route(&t.route).0);
        }
    }
    run.limits.models = Some(models);
}

/// Milestone 9.8: one row of `run`'s frozen role table, set to `model` (`<runtime>:<id>`)
/// at `effort` with `fallback`.
pub fn set_row(
    run: &mut Run,
    role: proto::models::Role,
    model: &str,
    effort: Option<&str>,
    fallback: Option<&str>,
) {
    use proto::models::{ModelRef, RoleChoice};
    let parse = |m: &str| ModelRef::parse(m).expect("a model");
    let mut models = run.limits.models().clone();
    let row = RoleChoice {
        model: parse(model),
        effort: effort.map(str::to_string),
        fallback: fallback.map(parse),
    };
    models.rows.insert(role, row);
    run.limits.models = Some(models);
}

pub fn task<'a>(run: &'a Run, id: &str) -> &'a Task {
    run.task(id).unwrap_or_else(|| panic!("no task {id}"))
}

pub fn err(task: Option<&str>, field: &str, rule: &str, message: &str) -> PlanError {
    PlanError {
        task: task.map(str::to_string),
        field: field.to_string(),
        rule: rule.to_string(),
        message: message.to_string(),
    }
}

/// Writes a checked-in fixture a test records on request (milestone 9.1 task
/// M9.1.13's `m9_gate_ops.json`), outside the pure engine's own files.
pub fn record_fixture(path: &str, text: &str) {
    std::fs::write(path, text).unwrap_or_else(|e| panic!("could not write {path}: {e}"));
}

/// Milestone 9.5: `task`'s race with lane a on `route`, lane b on Codex, each in
/// `states`' state, with `winner` set when a lane is `Won` or `Adopted` (task M9.5.15's
/// checkout tests; the reducer that fills a race is M9.5.17a's).
pub fn race_of(task: &Task, states: [proto::LaneState; 2]) -> super::model::Race {
    use proto::{LaneState, RaceLane};
    let lane = |lane: RaceLane, state: LaneState| {
        let mut route = task.route.clone();
        if lane == RaceLane::B {
            route.runtime = proto::Runtime::Codex;
        }
        super::model::Lane {
            lane,
            route,
            review_route: None,
            checkout: super::model::lane_checkout(task.id(), lane),
            state,
            session: if lane == RaceLane::A { 1 } else { 2 },
            start_commit: None,
            head: None,
            done: None,
            failures: 0,
            bounces: proto::GateCounts::default(),
            stalls: 0,
            budget_exceeded: 0,
            spent: proto::Spend::default(),
            reason: None,
            salvage_ref: None,
            cleared_locks: Vec::new(),
            kill_sent_at: None,
            removed: false,
            kept: false,
            gates: Default::default(),
        }
    };
    let lanes = vec![lane(RaceLane::A, states[0]), lane(RaceLane::B, states[1])];
    let winner = lanes
        .iter()
        .find(|l| matches!(l.state, LaneState::Won | LaneState::Adopted))
        .map(|l| l.lane);
    let adopted = lanes.iter().any(|l| l.state == LaneState::Adopted);
    super::model::Race {
        lanes,
        winner,
        adopted,
        started_at: 100,
        crowned: winner.is_some(),
        ended: false,
    }
}
