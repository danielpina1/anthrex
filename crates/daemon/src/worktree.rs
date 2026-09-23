//! Layout helpers and the hardened git invocation for milestone 5's linked worktrees.
//!
//! This module owns two things, and only these two: where a worktree lives on disk
//! (`hash8`, `branch_dir_name`, `repo_worktrees_dir`), and the one helper every git
//! command this milestone runs goes through, hardened per design decisions 1 to 3 —
//! `--no-optional-locks` and a scrubbed environment on every invocation (AGENTS.md hard
//! rules 10 and 11), `LC_ALL=C` and `GIT_TERMINAL_PROMPT=0` so messages are stable and
//! git never blocks on a credential prompt, and a caller-supplied deadline rather than a
//! bare timeout, converted to whatever [`subprocess::run_captured`] needs at each call.
//! [`run_git`] is `#[doc(hidden)] pub` rather than private only so that
//! `crates/daemon/tests/worktree_env.rs` can exercise it in a test binary of its own —
//! see that file for why. It is not part of this module's real API; every other caller
//! lives inside this module and its submodules — `create`, `dirty_reason`, `remove` and
//! `discard_new`, plus this file's own tests.
//!
//! `create`, `remove`, `discard_new` and `dirty_reason` — the orchestration that actually
//! creates and removes a worktree — live in the [`ops`] and [`dirty`] submodules and are
//! re-exported here, so the module's public surface is the one the brief's interface
//! block names while no part of it grows past AGENTS.md rule 8's ~600 lines. The split is
//! by responsibility: this file is *where a worktree lives and how git is invoked*,
//! `ops` is *what the daemon does to one*, and `dirty` is *whether doing it would destroy
//! work* — the safety-critical question, kept where it can be read on its own.

mod dirty;
mod ops;

pub use dirty::{DirtyReason, dirty_reason};
pub use ops::{
    Created, ManagedWorktree, create, create_with_cleanup_timeout, discard_and_describe,
    discard_and_describe_with, discard_new, discard_new_with, remove,
};

use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crate::subprocess::{self, Outcome};

/// Design decision 3: the deadline shared by every git command one worktree operation
/// (one create, one removal) runs.
pub const OPERATION_TIMEOUT: Duration = Duration::from_secs(30);
/// Design decision 16: cleanup after a failed create gets its own, shorter deadline.
pub const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);
/// Directory name under `<wt>` that milestone 8 owns (design decision 7).
pub const RESERVED_DIR: &str = "runs";
/// Branch prefix that milestone 8 owns (design decision 8).
pub const RESERVED_BRANCH_PREFIX: &str = "anthrex/";

const MAX_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_STDERR_BYTES: usize = 8 * 1024;
/// Design decision 14.
const STDERR_TAIL_LINES: usize = 20;
/// Design decision 14.
const STDERR_TAIL_CHARS: usize = 1000;

/// One git invocation's result. `success` means git ran and exited zero; a caller that
/// wants the failure reason reads [`GitOutput::stderr_tail`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

impl GitOutput {
    /// Design decision 14: the last 20 lines of stderr, trimmed, cut to at most 1000
    /// characters from the front of that tail so a leading `fatal: ` line survives even
    /// when a single line dwarfs the cap.
    pub fn stderr_tail(&self) -> String {
        let lines: Vec<&str> = self.stderr.lines().collect();
        let start = lines.len().saturating_sub(STDERR_TAIL_LINES);
        let tail = lines[start..].join("\n");
        let tail = tail.trim();
        if tail.chars().count() > STDERR_TAIL_CHARS {
            tail.chars().take(STDERR_TAIL_CHARS).collect()
        } else {
            tail.to_string()
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("not a git repository: {}", dir.display())]
    NotARepo { dir: PathBuf },
    #[error("{0}")]
    InvalidBranch(String), // one of the branch messages below
    #[error("branch '{branch}' is already checked out at {}", path.display())]
    BranchInUse { branch: String, path: PathBuf },
    #[error("worktree path already exists: {}", path.display())]
    PathExists { path: PathBuf },
    /// `reason` is what makes this message true rather than merely alarming: see
    /// [`DirtyReason`], whose variants each describe what a removal would destroy.
    #[error("worktree {} {reason}", path.display())]
    Dirty { path: PathBuf, reason: DirtyReason },
    #[error("git {action} failed: {stderr}")]
    Git { action: String, stderr: String }, // action e.g. "worktree add"
    /// A failure that happened once `git worktree add` had already started, carrying
    /// design decision 16's suffix: the original failure, then whether the worktree it
    /// had made was removed again. Built only by [`ops::create`], at its two `worktree
    /// add` failure sites, from the `String` [`ops::discard_and_describe`] returns —
    /// that function is the one place either suffix is written, but it builds the
    /// message, not this variant.
    #[error("{0}")]
    FailedAfterAdd(String),
    #[error("git is not installed or not on PATH")]
    GitMissing,
    #[error("git {args} timed out after {secs} s")]
    TimedOut { args: String, secs: u64 },
}

/// FNV-1a, 32-bit, over `path`'s raw bytes ([`OsStrExt::as_bytes`]). Design decision 6:
/// deliberately not a truncated SHA-256 — the name has to be short, stable across Rust
/// releases (unlike `DefaultHasher`) and readable in a path, which FNV-1a already gives
/// for free without a dependency the daemon does not otherwise need.
pub fn hash8(path: &Path) -> String {
    const OFFSET_BASIS: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;
    let mut hash = OFFSET_BASIS;
    for &byte in path.as_os_str().as_bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:08x}")
}

/// Design decision 7: every `/` in a branch name becomes `-` in its worktree directory
/// name, so `feat/api` cannot land inside a directory literally named `feat`.
pub fn branch_dir_name(branch: &str) -> String {
    branch.replace('/', "-")
}

/// `<wt>` from design decision 6: `<worktrees_root>/<project_basename>-<hash8>`, where
/// `project_basename` is `project_root`'s last path component with every character
/// outside `[A-Za-z0-9._-]` replaced by `-`, and `hash8` is [`hash8`] of the full
/// `project_root` path (not just its basename).
pub fn repo_worktrees_dir(worktrees_root: &Path, project_root: &Path) -> PathBuf {
    let basename = project_root
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let sanitized: String = basename
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    worktrees_root.join(format!("{sanitized}-{}", hash8(project_root)))
}

/// The directory a worktree for `branch` of `project_root` occupies: `<wt>` joined with
/// the branch's directory name.
///
/// **One derivation, two readers**, and that is the whole reason it is a function rather
/// than two `join`s. `WindowManager::create`'s phase A calls this to claim the directory
/// against other creates, under the manager lock, before any git runs; [`ops::create`]
/// calls it to decide where `git worktree add` writes. If those two ever computed
/// different paths for one create, the claim would guard a directory nobody makes while
/// the real one stayed unclaimed — and the loser of a race would then run
/// `git worktree remove --force` over the winner's live checkout. They were two copies of
/// the arithmetic over two separately resolved roots until the whole-branch review's
/// finding 4; now there is one of each.
///
/// Pure, so phase A can call it under the lock.
pub fn worktree_dir(worktrees_root: &Path, project_root: &Path, branch: &str) -> PathBuf {
    repo_worktrees_dir(worktrees_root, project_root).join(branch_dir_name(branch))
}

/// Design decision 8, rules 1 to 4. Pure. `crates/tui/src/dialog.rs` has its own copy
/// with the same messages (task M5.8).
pub fn check_branch_syntax(branch: &str) -> Result<(), WorktreeError> {
    if branch.trim().is_empty() {
        return Err(WorktreeError::InvalidBranch(
            "branch name is required".to_string(),
        ));
    }
    if branch.chars().any(char::is_whitespace) {
        return Err(WorktreeError::InvalidBranch(
            "branch name cannot contain spaces".to_string(),
        ));
    }
    if branch.starts_with('-') {
        return Err(WorktreeError::InvalidBranch(
            "branch name cannot start with '-'".to_string(),
        ));
    }
    if branch.starts_with(RESERVED_BRANCH_PREFIX) {
        return Err(WorktreeError::InvalidBranch(
            "branches under anthrex/ are reserved for orchestration runs".to_string(),
        ));
    }
    Ok(())
}

/// The one path every git command this milestone runs takes: `-C <dir>
/// --no-optional-locks <args...>` (design decision 2, written in that order, immediately
/// after `-C <dir>` and before the subcommand), `LC_ALL=C` and `GIT_TERMINAL_PROMPT=0`
/// (design decision 1), through [`subprocess::run_captured`], which scrubs `GIT_DIR`,
/// `GIT_WORK_TREE`, `GIT_COMMON_DIR`, `GIT_INDEX_FILE` and `GIT_PREFIX` unconditionally.
///
/// `deadline` is converted to the timeout `run_captured` wants at the moment this
/// function is called (design decision 3): a deadline already passed fails as
/// [`WorktreeError::TimedOut`] without spawning anything, and a child still running when
/// the deadline strikes is killed by `run_captured` and also reported as `TimedOut`.
///
/// A spawn failure whose [`io::ErrorKind`] is [`io::ErrorKind::NotFound`] is reported as
/// [`WorktreeError::GitMissing`] rather than folded into a non-zero exit, so a caller can
/// tell "git is not installed" apart from "git refused this operation". `success` in the
/// returned [`GitOutput`] is what tells the two remaining cases (a clean exit vs. a
/// non-zero one) apart; this function never fails just because git exited non-zero — the
/// caller decides what that means for the operation it is composing (see design
/// decisions 9, 10, 12 and 13, all implemented in task M5.3).
///
/// `pub` and `#[doc(hidden)]`: see the module doc comment. This is not part of the
/// module's public API.
///
/// `args` are [`OsStr`]s rather than `&str`s because two of them are paths the daemon
/// itself chose — the worktree `git worktree add` creates and `git worktree remove`
/// deletes. A `&str` signature would force a lossy conversion there, and a path that
/// failed it would leave a worktree this daemon made but can never remove. Only the
/// human-readable echo of the arguments in an error message is lossy.
#[doc(hidden)]
pub fn run_git(
    git: &OsStr,
    dir: &Path,
    args: &[&OsStr],
    deadline: Instant,
) -> Result<GitOutput, WorktreeError> {
    run_git_with_cap(git, dir, args, deadline, MAX_OUTPUT_BYTES)
}

/// As [`run_git`], with a caller-chosen cap on stdout instead of 256 KiB. Milestone 8a's
/// run git operations (`crate::run::git`) read whole-tree listings and diffs, which a
/// real repository can push past the default cap; everything else about the invocation
/// — `-C <dir> --no-optional-locks`, the scrubbed environment, the deadline — is
/// identical, because it is the same code.
pub fn run_git_with_cap(
    git: &OsStr,
    dir: &Path,
    args: &[&OsStr],
    deadline: Instant,
    max_output_bytes: usize,
) -> Result<GitOutput, WorktreeError> {
    let now = Instant::now();
    let joined_args = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    if deadline <= now {
        return Err(WorktreeError::TimedOut {
            args: joined_args,
            secs: 0,
        });
    }
    let timeout = deadline - now;

    let mut command = Command::new(git);
    command
        .arg("-C")
        .arg(dir)
        .arg("--no-optional-locks")
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0");

    let captured =
        subprocess::run_captured(&mut command, max_output_bytes, MAX_STDERR_BYTES, timeout);

    if let Some(kind) = captured.spawn_error {
        return if kind == io::ErrorKind::NotFound {
            Err(WorktreeError::GitMissing)
        } else {
            Err(WorktreeError::Git {
                action: joined_args,
                stderr: format!("could not start git: {kind}"),
            })
        };
    }

    match captured.outcome {
        Outcome::Complete(stdout) => Ok(GitOutput {
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: captured.stderr,
            success: true,
        }),
        Outcome::Failed => Ok(GitOutput {
            stdout: String::new(),
            stderr: captured.stderr,
            success: false,
        }),
        Outcome::TimedOut(_) => Err(WorktreeError::TimedOut {
            args: joined_args,
            secs: timeout.as_secs(),
        }),
        Outcome::Truncated(_) => Err(WorktreeError::Git {
            action: joined_args,
            stderr: "git produced more output than expected".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::tempdir;

    fn write_script(dir: &Path, name: &str, body: &str) -> PathBuf {
        let script = dir.join(name);
        fs::write(&script, body).unwrap();
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
        script
    }

    #[test]
    fn hash8_matches_fnv1a_vectors() {
        assert_eq!(hash8(Path::new("")), "811c9dc5");
        assert_eq!(hash8(Path::new("a")), "e40c292c");
        assert_eq!(hash8(Path::new("/tmp/repo")), "a3cbe2c8");
    }

    #[test]
    fn branch_dir_name_replaces_slashes() {
        assert_eq!(branch_dir_name("feat/api/v2"), "feat-api-v2");
    }

    #[test]
    fn repo_worktrees_dir_uses_sanitized_basename_and_hash() {
        let worktrees_root = Path::new("/data/worktrees");
        let project_root = Path::new("/tmp/my repo");

        let dir = repo_worktrees_dir(worktrees_root, project_root);

        let expected = worktrees_root.join(format!("my-repo-{}", hash8(project_root)));
        assert_eq!(dir, expected);
    }

    #[test]
    fn branch_syntax_rules_and_messages() {
        let cases = [
            ("", "branch name is required"),
            ("   ", "branch name is required"),
            ("feat api", "branch name cannot contain spaces"),
            (" feat", "branch name cannot contain spaces"),
            ("-feat", "branch name cannot start with '-'"),
            (
                "anthrex/run",
                "branches under anthrex/ are reserved for orchestration runs",
            ),
        ];
        for (branch, expected) in cases {
            let error = check_branch_syntax(branch).unwrap_err();
            assert_eq!(error.to_string(), expected, "branch {branch:?}");
        }

        assert!(check_branch_syntax("feat/api").is_ok());
    }

    // `the_git_helper_passes_no_optional_locks_and_scrubs_the_environment` lives in
    // `crates/daemon/tests/worktree_env.rs`, its own test binary, not here: it mutates
    // the real process environment, and `a_missing_git_is_reported_as_such` below
    // spawns a process on another libtest thread of *this* binary — a concurrent
    // `environ` reader racing that mutation, which is undefined behaviour regardless of
    // which variable either side touches. See that file's module doc comment.

    #[test]
    fn a_passed_deadline_fails_without_spawning() {
        let scripts = tempdir().unwrap();
        let argv_log = scripts.path().join("argv.log");
        let script = write_script(
            scripts.path(),
            "should-not-run",
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n",
                argv_log.display()
            ),
        );
        let dir = tempdir().unwrap();
        let deadline = Instant::now() - Duration::from_secs(1);

        let result = run_git(
            script.as_os_str(),
            dir.path(),
            &[OsStr::new("status")],
            deadline,
        );

        assert!(
            matches!(result, Err(WorktreeError::TimedOut { .. })),
            "{result:?}"
        );
        assert!(
            !argv_log.exists(),
            "the script must never have run for a deadline already in the past"
        );
    }

    #[test]
    fn a_missing_git_is_reported_as_such() {
        let dir = tempdir().unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);

        let result = run_git(
            OsStr::new("/definitely/missing/git-xyz"),
            dir.path(),
            &[OsStr::new("status")],
            deadline,
        );

        assert!(
            matches!(result, Err(WorktreeError::GitMissing)),
            "{result:?}"
        );
    }

    #[test]
    fn stderr_tail_keeps_the_last_lines_within_the_cap() {
        let lines: Vec<String> = (1..=50).map(|n| format!("line {n}")).collect();
        let output = GitOutput {
            stdout: String::new(),
            stderr: lines.join("\n"),
            success: false,
        };

        let tail = output.stderr_tail();

        assert!(tail.starts_with("line 31"), "{tail:?}");
        assert_eq!(tail.lines().count(), 20);
        assert!(tail.ends_with("line 50"), "{tail:?}");

        let huge = GitOutput {
            stdout: String::new(),
            stderr: "x".repeat(5000),
            success: false,
        };

        let tail = huge.stderr_tail();

        assert_eq!(tail.chars().count(), STDERR_TAIL_CHARS);
    }
}
