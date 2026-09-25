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

/// `read_message` checking that the message contains `expect`.
pub fn read(expect: &str) -> Value {
    json!({"read_message": {"expect": expect}})
}

pub fn capture(name: &str, cmd: &str) -> Value {
    json!({"capture": {"name": name, "sh": cmd}})
}

/// `task_done` for a tdd task: the test and the red commit (`{{red}}` for a capture).
pub fn done_tdd(test: &str, red: &str) -> Value {
    json!({"mcp_call": {"tool": "task_done", "args": {"summary": "done", "test": test, "red": red}}})
}

/// One review finding at `file:line`.
pub fn finding(severity: &str, file: &str, line: u32, text: &str) -> Value {
    json!({"severity": severity, "file": file, "line": line, "text": text})
}

/// `submit_review` with `verdict` `changes` and `findings`.
pub fn changes(findings: &[Value]) -> Value {
    json!({"mcp_call": {"tool": "submit_review", "args": {"verdict": "changes", "summary": "needs work", "findings": findings}}})
}

/// The run's `run.json`, next to its report.
pub fn run_json(run: &RunInfo) -> Value {
    let path = run.report_path.with_file_name("run.json");
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

/// Waits until the run's report contains `needle` (the report is written at most every
/// 500 ms), and returns it.
pub fn report_with(run: &RunInfo, needle: &str) -> String {
    let wait = Duration::from_secs(10);
    let deadline = Instant::now() + wait;
    loop {
        let text = report(run);
        if text.contains(needle) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "the report has no {needle:?} within {wait:?}:\n{text}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The text of every stream-json user message in `<io>/<name>.stdin`'s lines.
pub fn user_texts(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["type"] == "user")
        .filter_map(|v| {
            v["message"]["content"][0]["text"]
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

/// A read-only `git <args>` in `dir` beside a running engine (M8a.25 fix round 1):
/// `--no-optional-locks`, the user's configuration and every inherited `GIT_*`
/// location variable left out (AGENTS.md rule 11). Its trimmed stdout, or `None` when
/// it failed.
pub fn git_read(dir: &Path, args: &[&str]) -> Option<String> {
    git_read_with(dir, None, args)
}

/// Final fix batch F1b: [`git_read`] in a task worktree, reading the worker's objects
/// too (its private directory, `<data>/runs/<run>/tasks/<task>/objects`, as an
/// alternate), which the engine imports into the repository only at a done check, a
/// turn-end count or a hand-back.
pub fn worker_git_read(
    data: &Path,
    run: &str,
    task: &str,
    dir: &Path,
    args: &[&str],
) -> Option<String> {
    let objects = data
        .join("runs")
        .join(run)
        .join("tasks")
        .join(task)
        .join("objects");
    git_read_with(dir, Some(&objects), args)
}

fn git_read_with(dir: &Path, alternate: Option<&Path>, args: &[&str]) -> Option<String> {
    let mut command = std::process::Command::new("git");
    match alternate {
        Some(objects) => command.env("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects),
        None => command.env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES"),
    };
    let output = command
        .arg("--no-optional-locks")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
