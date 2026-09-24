//! Plans, fake-agent scripts and snapshot accessors shared by the run e2e tests
//! (`run_e2e_basic.rs`, `run_e2e_settings.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use proto::{AgentRole, RunInfo, RunReply, RunState, TaskInfo};
use serde_json::{Value, json};

use super::run_harness::{git_in, script_in};

/// Decision 52's retirement: a retired window stays listed this long.
pub const RETIRE_AFTER: Duration = Duration::from_secs(30);

pub fn commit(file: &str, content: &str) -> Value {
    json!({"git_commit": {"file": file, "content": content, "message": format!("add {file}")}})
}

pub fn done(summary: &str) -> Value {
    json!({"mcp_call": {"tool": "task_done", "args": {"summary": summary}}})
}

pub fn done_expecting_error() -> Value {
    json!({"mcp_call": {"tool": "task_done", "args": {"summary": "first try"}, "expect_error": true}})
}

pub fn sh(cmd: &str) -> Value {
    json!({"sh": {"cmd": cmd}})
}

pub fn approve() -> Value {
    json!({"mcp_call": {"tool": "submit_review", "args": {"verdict": "approve", "summary": "ok", "findings": []}}})
}

/// An S `check`-mode task (reason `smoke`) owning `owns`, with `extra` TOML lines.
pub fn task(id: &str, owns: &[&str], extra: &str) -> String {
    let owns: Vec<String> = owns.iter().map(|o| format!("{o:?}")).collect();
    format!(
        "\n[[task]]\nid = \"{id}\"\ntitle = \"Task {id}\"\nsize = \"S\"\ntest_mode = \"check\"\ntest_mode_reason = \"smoke\"\nowns = [{}]\nbrief = \"Do {id}\"\nacceptance = [\"{id} is done\"]\n{extra}\n",
        owns.join(", ")
    )
}

pub const CODEX: &str = "route = { runtime = \"codex\" }";

/// A plan with `check = "true"` and `profile` extra lines.
pub fn plan(profile: &str, tasks: &[String]) -> String {
    format!(
        "goal = \"Add a\"\n\n[profile]\ncheck = \"true\"\n{profile}\n{}",
        tasks.concat()
    )
}

/// The green scripts: the worker commits `a.txt` and calls `task_done`; the reviewer
/// approves.
pub fn green_scripts(repo: &Path) {
    script_in(
        repo,
        "worker-t1-1",
        &[commit("a.txt", "a\n"), done("added a")],
    );
    script_in(repo, "reviewer-t1-1", &[approve()]);
}

pub fn complete(run: &RunInfo) -> bool {
    run.state == RunState::Complete
}

pub fn t<'a>(run: &'a RunInfo, id: &str) -> &'a TaskInfo {
    run.tasks.iter().find(|t| t.id == id).expect("task listed")
}

pub fn window_of(run: &RunInfo, task: &str, role: AgentRole) -> Option<u32> {
    t(run, task)
        .rounds
        .iter()
        .rev()
        .find(|r| r.role == role)
        .and_then(|r| r.window_id)
}

pub fn integration(task: &TaskInfo) -> PathBuf {
    task.worktree.with_file_name("integration")
}

pub fn no_run_branches(repo: &Path) -> bool {
    git_in(repo, &["for-each-ref", "refs/heads/anthrex/"]).is_empty()
}

pub fn refused(reply: RunReply) -> String {
    match reply {
        RunReply::Refused { request, message } => {
            assert_eq!(request, "run start");
            message
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

pub fn report(run: &RunInfo) -> String {
    std::fs::read_to_string(&run.report_path).unwrap_or_default()
}

/// Polls until `f` returns `Some`, for at most `wait`.
pub fn until<T>(what: &str, wait: Duration, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + wait;
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
