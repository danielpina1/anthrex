//! The fail-to-pass test proof (decision 33, spec §8.1). Blocking I/O: call only from
//! `spawn_blocking` or a dedicated thread (AGENTS.md rule 2). The proof's git writes
//! (`worktree add`, the checkouts, the clean) are the op executor's to queue through
//! `run::git::GitQueue`, like M8a.9's writes.
//!
//! In the task's scratch proof worktree (`<wt>/runs/<run>/<task>.proof`, created on
//! first use, with `setup` run once in it), the single-test command runs at the `red`
//! commit, where it must fail, then at the task's head, where it must exit 0 and print
//! a line matching `test_passed`. [`proof_command`] and [`proof_pattern`] build
//! `OpKind::Proof`'s `command` and `passed` from the profile and the worker's test
//! name; they are pure, for the engine to call.

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use regex::Regex;

use super::exec::{ShellOutcome, run_matching, run_shell};
use super::git::{materialize, prepare_scratch, remove_worktree};
use crate::launch::shell_quote;

/// `OpKind::Proof`'s fields (the interface block's `ProofOp`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofOp {
    /// The repository the proof worktree belongs to.
    pub root: PathBuf,
    /// The proof worktree, `<wt>/runs/<run>/<task>.proof`.
    pub path: PathBuf,
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
    /// `setup` failed in a new proof worktree (`OpResult::SetupFailed`); `output` is its
    /// tail. The worktree was removed again, so the next proof re-runs `setup`.
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

/// Decision 33's proof. Each git step is bounded by `git_timeout`; `setup` and each run
/// by `op.timeout_secs`. The `head` run is skipped when the `red` run did not fail,
/// since the proof has already failed.
pub fn run_proof(
    git: &OsStr,
    op: &ProofOp,
    git_timeout: Duration,
) -> Result<ProofRuns, ProofError> {
    let pattern = Regex::new(&op.passed).map_err(|error| {
        ProofError::Failed(format!(
            "test_passed is not a valid regular expression: {error}"
        ))
    })?;
    let timeout = Duration::from_secs(op.timeout_secs);
    let created = prepare_scratch(git, &op.root, &op.path, &op.red, git_timeout)
        .map_err(ProofError::Failed)?;
    if created && let Some(setup) = &op.setup {
        let outcome = run_shell(&op.path, setup, &op.env, timeout);
        if !outcome.ok {
            // A scratch checkout of committed work plus setup's own output: nothing to
            // salvage (decision 20 is about agents' work).
            if let Err(error) = remove_worktree(git, &op.root, &op.path, git_timeout) {
                tracing::warn!(%error, path = %op.path.display(), "could not remove a proof worktree whose setup failed");
            }
            return Err(ProofError::SetupFailed {
                output: with_timeout_note(outcome),
            });
        }
    }

    materialize(git, &op.path, &op.red, git_timeout).map_err(ProofError::Failed)?;
    let (red, _) = run_matching(&op.path, &op.command, &op.env, timeout, None);
    let mut runs = ProofRuns {
        red_failed: !red.ok && !red.timed_out,
        ..ProofRuns::default()
    };
    runs.red_tail = with_timeout_note(red);
    if !runs.red_failed {
        return Ok(runs);
    }

    materialize(git, &op.path, &op.head, git_timeout).map_err(ProofError::Failed)?;
    let (head, matched) = run_matching(&op.path, &op.command, &op.env, timeout, Some(&pattern));
    runs.head_passed = head.ok;
    runs.matched = matched;
    runs.head_tail = with_timeout_note(head);
    Ok(runs)
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
