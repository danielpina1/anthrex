//! The hardened `git status` probe for one worktree.
//!
//! [`probe`] runs the one command design decision 5 names
//! (`git -C <root> --no-optional-locks status --porcelain=v2 --branch
//! --untracked-files=normal -z`), hands its stdout to
//! [`crate::git::parse::parse_porcelain_v2_z`], and fills in the in-progress operation
//! by stat-ing the worktree's own git dir with [`detect_operation`] — never by parsing
//! `status` output for it (design decision 7).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use proto::{GitOperation, GitState};

use crate::git::parse::parse_porcelain_v2_z;
use crate::subprocess::{self, Outcome};

/// Design decision 9: the probe gives up after this long, keeping whatever it parsed
/// and marking the result stale.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Design decision 9: the probe gives up after this much stdout, for the same reason.
/// A worktree with fifty thousand untracked files must degrade, not wedge the daemon.
pub const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// `git rev-parse --absolute-git-dir` only ever prints one path; a generous cap here
/// is just another guard against a runaway or malicious `git`, not a real limit.
const GIT_DIR_MAX_OUTPUT_BYTES: usize = 4096;

/// A fixed, short budget for [`absolute_git_dir`], independent of the caller's own
/// `timeout`. Resolving one absolute path is near-instant for a real `git`; giving it
/// the full probe timeout as well would let a hang there double a probe's worst-case
/// latency instead of merely adding to it.
const GIT_DIR_TIMEOUT: Duration = Duration::from_secs(1);

/// Probes `root` (a worktree root) with `git`, hardened per design decision 8: a
/// scrubbed environment, a non-blocking stdout drain bounded by [`MAX_OUTPUT_BYTES`]
/// and `timeout`, and the child always terminated and reaped.
///
/// Returns `None` when `root` is not a git working tree, when `git` cannot be spawned,
/// or when it exits non-zero — there is no state to show. A timeout or a truncated
/// read is different: whatever was parsed from the partial output is returned with
/// [`proto::GitState::stale`] set, per design decision 9. `stale` is never set anywhere
/// else.
pub fn probe(git: &OsStr, root: &Path, timeout: Duration) -> Option<GitState> {
    let mut command = Command::new(git);
    command.arg("-C").arg(root).args([
        "--no-optional-locks",
        "status",
        "--porcelain=v2",
        "--branch",
        "--untracked-files=normal",
        "-z",
    ]);

    let (output, stale) = match subprocess::run(&mut command, MAX_OUTPUT_BYTES, timeout) {
        Outcome::Complete(output) => (output, false),
        Outcome::TimedOut(output) | Outcome::Truncated(output) => (output, true),
        Outcome::Failed => return None,
    };

    let parsed = parse_porcelain_v2_z(&output)?;

    // Operation detection needs its own `git rev-parse --absolute-git-dir` call
    // (design decision 7's context), which costs time this probe has already run
    // short on when `stale` is set. Skip it then: a degraded probe should not risk
    // compounding its own lateness, and the bar has no rendering for "stale" and an
    // operation at once anyway.
    let operation = if stale {
        None
    } else {
        absolute_git_dir(git, root, GIT_DIR_TIMEOUT).and_then(|git_dir| detect_operation(&git_dir))
    };

    Some(GitState {
        head: parsed.head,
        upstream: parsed.upstream,
        ahead: parsed.ahead,
        behind: parsed.behind,
        dirty: parsed.counts.dirty,
        untracked: parsed.counts.untracked,
        conflicts: parsed.counts.conflicts,
        operation,
        stale,
    })
}

/// Resolves `root`'s own git dir: for a linked worktree this is
/// `<common-dir>/worktrees/<name>`, distinct from the shared common dir — which is
/// exactly why this cannot be derived from the common dir already known from
/// `daemon::project::detect_roots_with` and needs its own call scoped to `root` (design
/// decision 7). `None` on any failure: operation detection is best-effort and never
/// fails the probe itself.
fn absolute_git_dir(git: &OsStr, root: &Path, timeout: Duration) -> Option<PathBuf> {
    let mut command = Command::new(git);
    command
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--absolute-git-dir"]);

    let output = match subprocess::run(&mut command, GIT_DIR_MAX_OUTPUT_BYTES, timeout) {
        Outcome::Complete(output) => output,
        Outcome::Failed | Outcome::TimedOut(_) | Outcome::Truncated(_) => return None,
    };
    let text = std::str::from_utf8(&output).ok()?.trim_end_matches('\n');
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

/// Finds the git operation in progress in `git_dir`, if any — by `stat`-ing the marker
/// files git itself uses, not by parsing `status` output (design decision 7). First
/// match wins, checked in this order: `MERGE_HEAD`, `rebase-merge/` or `rebase-apply/`,
/// `CHERRY_PICK_HEAD`, `REVERT_HEAD`, `BISECT_LOG`.
///
/// `git_dir` must be the worktree's *own* git dir (`--absolute-git-dir`), not the
/// shared common dir: a linked worktree's rebase or merge state lives under
/// `<common-dir>/worktrees/<name>/`, and reading the common dir instead would show one
/// worktree's operation in every worktree of the repository.
pub fn detect_operation(git_dir: &Path) -> Option<GitOperation> {
    if git_dir.join("MERGE_HEAD").exists() {
        Some(GitOperation::Merge)
    } else if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some(GitOperation::Rebase)
    } else if git_dir.join("CHERRY_PICK_HEAD").exists() {
        Some(GitOperation::CherryPick)
    } else if git_dir.join("REVERT_HEAD").exists() {
        Some(GitOperation::Revert)
    } else if git_dir.join("BISECT_LOG").exists() {
        Some(GitOperation::Bisect)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    //! [`detect_operation`] is pure filesystem logic, so these are plain unit tests
    //! against synthetic marker files — no repository needed. They exist to cover the
    //! priority order and the operations `crates/daemon/tests/git_probe.rs`'s
    //! real-repository tests do not construct (cherry-pick, revert, bisect): a wrong
    //! answer there would not be caught by any of the ten tests the brief names.

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn no_markers_means_no_operation() {
        let dir = tempdir().unwrap();
        assert_eq!(detect_operation(dir.path()), None);
    }

    #[test]
    fn merge_head_is_recognised() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("MERGE_HEAD"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Merge));
    }

    #[test]
    fn rebase_apply_directory_is_recognised() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("rebase-apply")).unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Rebase));
    }

    #[test]
    fn cherry_pick_head_is_recognised() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("CHERRY_PICK_HEAD"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::CherryPick));
    }

    #[test]
    fn revert_head_is_recognised() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("REVERT_HEAD"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Revert));
    }

    #[test]
    fn bisect_log_is_recognised() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("BISECT_LOG"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Bisect));
    }

    #[test]
    fn merge_takes_priority_over_every_other_marker() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("MERGE_HEAD"), "").unwrap();
        std::fs::create_dir(dir.path().join("rebase-merge")).unwrap();
        std::fs::write(dir.path().join("CHERRY_PICK_HEAD"), "").unwrap();
        std::fs::write(dir.path().join("REVERT_HEAD"), "").unwrap();
        std::fs::write(dir.path().join("BISECT_LOG"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Merge));
    }

    #[test]
    fn rebase_takes_priority_over_cherry_pick_revert_and_bisect() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("rebase-merge")).unwrap();
        std::fs::write(dir.path().join("CHERRY_PICK_HEAD"), "").unwrap();
        std::fs::write(dir.path().join("REVERT_HEAD"), "").unwrap();
        std::fs::write(dir.path().join("BISECT_LOG"), "").unwrap();
        assert_eq!(detect_operation(dir.path()), Some(GitOperation::Rebase));
    }
}
