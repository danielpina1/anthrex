//! What every design-flow scenario shares: the harness, the agents' scripts, the start,
//! the user's gate commands, and readers of what the run stored and committed.

use std::path::Path;
use std::process::Output;
use std::time::Duration;

use proto::{DocGateKind, RunInfo};
use serde_json::Value;

use crate::support::orch_script::passed;
use crate::support::run_design::*;
use crate::support::run_harness::{GIT_TIMEOUT_SECS, RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;

/// The scripted orchestrator: the run's first orchestrator session.
pub const ORCH: &str = "orchestrator-run-1";

/// Both runtimes are `fake-agent` and both are installed, so the brainstormers are
/// labelled by runtime (decision 10).
pub const LABELS: [&str; 2] = ["claude", "codex"];

/// The goal every scenario starts.
pub const GOAL: &str = "add a.txt";

/// The spec's file name, from its `# Add a.txt` title (decision 24).
pub const SLUG: &str = "add-a-txt";

/// The most git calls the documents commit makes (`run/git/docs.rs::commit_docs`,
/// `docs/timing-budgets.md`, "Recorded, from M9.6.20"): the symbolic-link walk, an
/// `ls-tree` per tree on the way to at most three folders (`docs/anthrex/<folder>`;
/// 9, though this harness's repository has no other `docs` tree); the tree, a
/// `read-tree`, a `hash-object` and an `update-index` per document (at most three), an
/// `ls-tree` and a `cat-file` for a round's appended spec, and a `write-tree` (10);
/// the ref moves, at most 9 (a later round's stage: two `rev-parse`s, `commit-tree`,
/// `update-ref`, then on a race `rev-parse`, `own`'s `show`, and `follow`'s `rev-parse`,
/// `update-ref` and `rev-parse`; a branch's take fewer); and the integration checkout's
/// reattach (1): 29, rounded up.
const DOCS_COMMIT_GIT_CALLS: u64 = 30;

/// The read-back of the approved documents before the commit: the driver's `IO_WAIT`
/// (`run/driver/design_io.rs`, private to the daemon, so this restates it).
const DOCS_READ_BACK: Duration = Duration::from_secs(10);

/// The documents commit (decision 23): the read-back, then its git calls at the
/// harness's `git_timeout_secs`, one queued write with nothing else writing (every
/// branch waits while the commit is due).
pub const DOCS_COMMIT_WAIT: Duration = DOCS_READ_BACK.saturating_add(Duration::from_secs(
    DOCS_COMMIT_GIT_CALLS * GIT_TIMEOUT_SECS,
));

/// A design run from its plan's approval to completion: the documents commit, then one
/// task path (`RUN_WAIT`).
pub const DESIGN_RUN_WAIT: Duration = RUN_WAIT.saturating_add(DOCS_COMMIT_WAIT);

/// [`LABELS`] as the orchestrator's steps take them.
pub fn labels() -> [String; 2] {
    LABELS.map(String::from)
}

/// A design harness (`design.default = "full"`) with `lines` more `[orchestrator]`
/// lines.
pub fn harness(lines: &str) -> RunHarness {
    RunHarness::design(lines, &[])
}

/// Both brainstormers submit their fixture drafts in their first session.
pub fn drafts(h: &RunHarness) {
    for label in LABELS {
        h.brainstormer(label, 1, Draft::Fixture);
    }
}

/// Task `id`'s worker commits `file` and calls `task_done`; its reviewer approves.
pub fn green(h: &RunHarness, id: &str, file: &str) {
    crate::support::run_rounds::green(h, id, file);
}

/// Writes the orchestrator's script, starts the goal with `flags`, and waits for its
/// window; when the script asks [`QUESTION`], types [`ANSWER`] once it is asked. The
/// run and the window.
pub fn start(h: &RunHarness, steps: &[Value], flags: &[&str]) -> (String, u32) {
    h.script(ORCH, steps);
    let run = h.start_goal_id(GOAL, flags);
    let window = h.orchestrator_window(&run);
    if steps.iter().any(|s| s["print"] == QUESTION) {
        h.answer_question(ORCH, window);
    }
    (run, window)
}

/// `anthrex <args> --dir <repo>`, which must succeed; its stdout.
pub fn ok(h: &RunHarness, args: &[&str]) -> String {
    let out = cmd(h, args);
    assert!(
        out.status.success(),
        "{args:?}: exit {:?}\nstdout: {}\nstderr: {}\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// `anthrex <args> --dir <repo>`, which the daemon refuses: exit 1 with `message` as
/// stderr's last line.
pub fn refused(h: &RunHarness, args: &[&str], message: &str) {
    let out = cmd(h, args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{args:?}: {stderr}");
    assert_eq!(stderr.lines().last(), Some(message), "{args:?}");
}

fn cmd(h: &RunHarness, args: &[&str]) -> Output {
    let repo = h.repo.display().to_string();
    let mut all = args.to_vec();
    all.extend_from_slice(&["--dir", &repo]);
    h.anthrex_input(&all, "")
}

/// `run approve <run> --gate <kind> --version <n>` (ruling T17-1: the version the user
/// reviewed), once the run waits at that gate at v`n`.
pub fn approve(h: &RunHarness, run: &str, kind: DocGateKind, n: u32) -> RunInfo {
    let info = h.wait_doc_gate(run, kind, n, ORCH_WAIT);
    let n = n.to_string();
    ok(
        h,
        &[
            "run",
            "approve",
            run,
            "--gate",
            kind.label(),
            "--version",
            &n,
        ],
    );
    info
}

/// Waits until the orchestrator's script reached its `n`th marker.
pub fn wait_passed(h: &RunHarness, n: usize) -> Vec<Value> {
    h.wait_log(
        "the orchestrator's script to pass its expectations",
        |log| passed(log, ORCH) >= n,
        RUN_WAIT,
    )
}

/// The orchestrator's refused calls of `tool`: each one's error text.
pub fn refusals(h: &RunHarness, tool: &str) -> Vec<String> {
    (h.mcp_log().iter())
        .filter(|l| l["script"] == ORCH && l["tool"] == tool && l["ok"] == false)
        .map(|l| l["result"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The text of `name` in run `run`'s design folder (`<data>/runs/<id>/design/`), byte
/// for byte (UTF-8, as every stored document is).
pub fn stored(h: &RunHarness, run: &str, name: &str) -> String {
    let path = h.data().join("runs").join(run).join("design").join(name);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    String::from_utf8(bytes).expect("a stored document is UTF-8")
}

/// [`stored`], once the file exists and is UTF-8, within `wait`: a version whose write
/// is its own effect, read as soon as its gate shows.
pub fn stored_within(h: &RunHarness, run: &str, name: &str, wait: Duration) -> String {
    let path = h.data().join("runs").join(run).join("design").join(name);
    crate::support::run_plans::until(name, wait, || {
        let bytes = std::fs::read(&path).ok()?;
        String::from_utf8(bytes)
            .ok()
            .filter(|text| !text.is_empty())
    })
}

/// `git <args>` in `dir`'s repository, isolated as the harness's git is (AGENTS.md hard
/// rule 11's scrub, `--no-optional-locks`): its stdout, byte for byte (never trimmed;
/// UTF-8).
pub fn git_text(dir: &Path, args: &[&str]) -> String {
    crate::support::run_git::git_raw_in(dir, args)
}

/// The repository path of a committed document: `<docs_dir>/<folder>/<date>-<slug>.md`
/// (decision 24), the date the run's start date (UTC), from its `run.json`.
pub fn doc_path(h: &RunHarness, run: &str, folder: &str, suffix: &str) -> String {
    let json = h.run_json(run);
    let created = json["created_at"].as_u64().expect("the run's created_at");
    let date = daemon::run::report::format_utc(created);
    format!("docs/anthrex/{folder}/{}-{SLUG}{suffix}.md", &date[..10])
}
