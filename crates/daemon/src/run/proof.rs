//! The fail-to-pass test proof (decision 33, spec §8.1). Blocking I/O: call only from
//! `spawn_blocking` or a dedicated thread (AGENTS.md rule 2).
//!
//! The proof interleaves git writes with shell runs that may each take
//! `check_timeout_secs`, so it must not be one `GitQueue::write`: that would hold the
//! repository's write lock for the whole proof, and a lock-contention retry would re-run
//! the tests (fix round 1, ruling T10-I2). Instead the caller passes a `git_write` hook,
//! and only the git steps ([`GitStep`]: the worktree, the checkouts) run through it. The executor maps the hook to its `GitQueue` (for example
//! `|step| handle.block_on(queue.write(&root, step))` inside `spawn_blocking`); a
//! caller with no queue passes [`direct`].
//!
//! In the task's scratch proof worktree (`<wt>/runs/<run>/<task>.proof`, created on
//! first use, with `setup` run in it until it once succeeds), the single-test command
//! runs at the `red` commit, where it must fail, then at the task's head, where it
//! must exit 0 and print a line matching `test_passed`. [`proof_command`] and [`proof_pattern`] build
//! `OpKind::Proof`'s `command` and `passed` from the profile and the worker's test
//! name; they are pure, for the engine to call.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use regex::Regex;

use super::confine::ConfineSpec;
use super::exec::{ShellOutcome, run_matching};
use super::git::{absolute_git_dir, materialize, prepare_scratch_in};
use crate::launch::shell_quote;

/// `OpKind::Proof`'s fields (the interface block's `ProofOp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    /// The repository the proof worktree belongs to.
    pub root: PathBuf,
    /// The proof worktree, `<wt>/runs/<run>/<task>.proof`.
    pub path: PathBuf,
    /// Its own repository (final fix batch F1c, 3a): `<data>/runs/<run>/tasks/<task>.proof`.
    pub repo: PathBuf,
    pub red: String,
    pub head: String,
    /// `single_test` with `{test}` already replaced ([`proof_command`]).
    pub command: String,
    /// `test_passed` with `{test}` already replaced ([`proof_pattern`]).
    pub passed: String,
    /// `check_timeout_secs`: each run's timeout, and `setup`'s.
    pub timeout_secs: u64,
    pub setup: Option<String>,
    /// The profile's env, `{worktree}` already the proof worktree.
    pub env: Vec<(String, String)>,
    /// Final fix batch F1c (I2, and round 2 for `setup`): when set, `setup` and both
    /// test runs are confined to the proof worktree, its own object store, its
    /// temporary directory and the profile's `cache_dirs` (`super::confine`).
    pub confine: Option<ConfineSpec>,
}

/// What the two runs showed: `OpResult::Proof`'s fields, which M8a.11 wraps (as
/// M8a.8's `DoneChecked` is wrapped, `OpResult` not existing yet). The proof passed
/// when all three flags are true.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProofRuns {
    /// The run at `red` exited non-zero (a timeout does not count: it is no evidence
    /// that the test fails).
    pub red_failed: bool,
    /// The run at `head` exited 0. Not run, so `false`, when `red` did not fail.
    pub head_passed: bool,
    /// A line of the `head` run's output matched `passed`.
    pub matched: bool,
    pub red_tail: String,
    pub head_tail: String,
}

/// Why no proof could be made at all. Neither is a verdict on the worker's test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofError {
    /// `setup` failed in the proof worktree (`OpResult::SetupFailed`); `output` is its
    /// tail. No [`SETUP_MARKER`] was written, so the next proof re-runs `setup`.
    SetupFailed { output: String },
    /// A git step failed, or `passed` is not a regular expression.
    Failed(String),
}

/// Decision 33: `single_test` with `{test}` replaced by the shell-quoted test name, so
/// a name like `a; touch pwned` is one word to the shell.
pub fn proof_command(single_test: &str, test: &str) -> String {
    single_test.replace("{test}", &shell_quote(test))
}

/// Decision 33: `test_passed` with `{test}` replaced by the regex-escaped test name.
pub fn proof_pattern(test_passed: &str, test: &str) -> String {
    test_passed.replace("{test}", &regex::escape(test))
}

/// The file, in the proof worktree's own git directory, that says `setup` has
/// succeeded in it (ruling T10-M1b). A worktree without it gets `setup` again, whether
/// its setup failed, the daemon died during it, or it was never run.
pub const SETUP_MARKER: &str = "anthrex-setup-ok";

/// One git step of the proof, owning everything it needs, so the caller's hook can
/// move it into `GitQueue::write` (which retries it on lock contention: each step is
/// idempotent).
pub type GitStep = Box<dyn Fn() -> Result<(), String> + Send + Sync + 'static>;

/// The `git_write` hook for a caller with no queue: runs the step at once.
pub fn direct(step: GitStep) -> Result<(), String> {
    step()
}

/// Decision 33's proof. Each git step is bounded by `git_timeout` and runs through
/// `git_write`; `setup` and each test run are bounded by `op.timeout_secs` and run
/// outside it. The `head` run is skipped when the `red` run did not fail, since the
/// proof has already failed.
pub fn run_proof(
    git: &OsStr,
    op: &ProofOp,
    git_timeout: Duration,
    git_write: &dyn Fn(GitStep) -> Result<(), String>,
) -> Result<ProofRuns, ProofError> {
    let pattern = Regex::new(&op.passed).map_err(|error| {
        ProofError::Failed(format!(
            "test_passed is not a valid regular expression: {error}"
        ))
    })?;
    let timeout = Duration::from_secs(op.timeout_secs);
    let (program, root, path) = (git.to_os_string(), op.root.clone(), op.path.clone());
    let (red, repo) = (op.red.clone(), op.repo.clone());
    git_write(Box::new(move || {
        prepare_scratch_in(&program, &root, &path, &red, &repo, git_timeout).map(|_created| ())
    }))
    .map_err(ProofError::Failed)?;

    let confined = op
        .confine
        .as_ref()
        .map(|spec| spec.for_checkout(&op.path))
        .transpose()
        .map_err(|error| ProofError::Failed(format!("the proof cannot be confined: {error}")))?;
    let confined = confined.as_ref();
    let marker = absolute_git_dir(git, &op.path, git_timeout)
        .map_err(ProofError::Failed)?
        .join(SETUP_MARKER);
    if !marker.exists() {
        if let Some(setup) = &op.setup {
            // A reused worktree may be at any commit; setup runs at red, as in a new one.
            git_write(checkout(git, &op.path, &op.red, git_timeout)).map_err(ProofError::Failed)?;
            let (outcome, _) = run_matching(&op.path, setup, &op.env, timeout, None, confined);
            if !outcome.ok {
                return Err(ProofError::SetupFailed {
                    output: with_timeout_note(outcome),
                });
            }
        }
        std::fs::write(&marker, "").map_err(|error| {
            ProofError::Failed(format!("could not write {}: {error}", marker.display()))
        })?;
    }

    git_write(checkout(git, &op.path, &op.red, git_timeout)).map_err(ProofError::Failed)?;
    let (red, _) = run_matching(&op.path, &op.command, &op.env, timeout, None, confined);
    let mut runs = ProofRuns {
        red_failed: !red.ok && !red.timed_out,
        ..ProofRuns::default()
    };
    runs.red_tail = with_timeout_note(red);
    if !runs.red_failed {
        return Ok(runs);
    }

    git_write(checkout(git, &op.path, &op.head, git_timeout)).map_err(ProofError::Failed)?;
    let (head, matched) = run_matching(
        &op.path,
        &op.command,
        &op.env,
        timeout,
        Some(&pattern),
        confined,
    );
    runs.head_passed = head.ok;
    runs.matched = matched;
    runs.head_tail = with_timeout_note(head);
    Ok(runs)
}

/// `materialize` as an owned [`GitStep`]: `checkout --detach --force <commit>`, then
/// `clean -fd`.
fn checkout(git: &OsStr, path: &Path, commit: &str, timeout: Duration) -> GitStep {
    let (program, path, commit): (OsString, PathBuf, String) =
        (git.to_os_string(), path.to_path_buf(), commit.to_string());
    Box::new(move || materialize(&program, &path, &commit, timeout))
}

/// The run's tail, with a last line saying it timed out when it did, so a proof failure
/// message that quotes the tail says why (invented text).
fn with_timeout_note(outcome: ShellOutcome) -> String {
    if !outcome.timed_out {
        return outcome.tail;
    }
    let note = format!("[anthrex: timed out after {} s]", outcome.secs);
    if outcome.tail.is_empty() {
        note
    } else {
        format!("{}\n{note}", outcome.tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_name_is_quoted_and_escaped() {
        assert_eq!(
            proof_command("cargo test {test}", "it's"),
            "cargo test 'it'\\''s'"
        );
        assert_eq!(
            proof_pattern("test {test} ... ok", "a+b"),
            "test a\\+b ... ok"
        );
    }

    #[test]
    fn a_timeout_is_said_in_the_tail() {
        let outcome = ShellOutcome {
            ok: false,
            code: None,
            timed_out: true,
            tail: "compiling".into(),
            secs: 7,
        };
        assert_eq!(
            with_timeout_note(outcome),
            "compiling\n[anthrex: timed out after 7 s]"
        );
    }
}
