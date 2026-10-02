//! The `anthrex run …` helpers of the `pr` end-to-end binaries (`run_e2e_pr.rs`,
//! `run_e2e_pr_preflight.rs`): one command's outcome, a `pr` start, and the refusal
//! shape. Moved from `run_e2e_pr.rs` (AGENTS.md rule 8, move-only).

use super::run_harness::RunHarness;
use super::run_plans::{plan, task};
use super::run_pr::pr_start;

/// One `anthrex …` outcome.
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub fn out(output: std::process::Output) -> Out {
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// `anthrex run <args> --dir <repo>`.
pub fn run(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    out(h.anthrex(&all))
}

pub fn ok(out: &Out) -> &str {
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    &out.stdout
}

/// Exit 1 with `message` as the whole of stderr's last line.
pub fn refused_with(out: &Out, message: &str) {
    assert_eq!(
        out.code, 1,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stderr.lines().last(), Some(message), "{}", out.stderr);
}

/// `run start --plan <toml> --delivery <mode> --yes`, waited for as a `pr` start.
pub fn start_out(h: &RunHarness, toml: &str, mode: &str) -> Out {
    let plan = h.plan(toml).display().to_string();
    let repo = h.repo.display().to_string();
    let args = [
        "run",
        "start",
        "--plan",
        &plan,
        "--dir",
        &repo,
        "--delivery",
        mode,
        "--yes",
    ];
    out(pr_start(h, &args))
}

/// [`start_out`] that must start; the run id.
pub fn start(h: &RunHarness, toml: &str, mode: &str) -> String {
    let started = start_out(h, toml, mode);
    assert_eq!(
        started.code,
        0,
        "start failed: {}{}",
        started.stderr,
        h.log_tail()
    );
    started.stdout.trim().to_string()
}

pub fn one_task() -> String {
    plan("", &[task("t1", &["a.txt"], "")])
}

/// `anthrex run <args> --dir <repo>`, waiting as `run accept` and `run discard` may.
pub fn run_long(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    out(h.anthrex_input(&all, ""))
}
