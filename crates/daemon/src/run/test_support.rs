//! Shared fixtures for the `run` unit tests (M8a.5 onwards).

use std::path::PathBuf;

use super::model::{Run, Task};
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
    let plan = parse_plan(text).unwrap_or_else(|e| panic!("fixture plan must parse: {e}"));
    build_run(
        plan,
        pre,
        BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config,
            now: 1_000,
            yes: false,
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
