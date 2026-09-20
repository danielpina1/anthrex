//! The hardened `git status` probe for one worktree.
//!
//! [`probe`] spawns exactly the one command design decision 5 names
//! (`git -C <root> --no-optional-locks status --porcelain=v2 --branch
//! --untracked-files=normal -z`) — no more — and hands its stdout to
//! [`crate::git::parse::parse_porcelain_v2_z`]. It fills in the in-progress operation
//! by resolving the worktree's own git dir straight from the filesystem
//! ([`resolve_git_dir`], no extra process) and stat-ing it with [`detect_operation`] —
//! never by parsing `status` output for it (design decision 7).

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

/// Probes `root` (a worktree root) with `git`, hardened per design decision 8: a
/// scrubbed environment, a non-blocking stdout drain bounded by [`MAX_OUTPUT_BYTES`]
/// and `timeout`, and the child always terminated and reaped.
///
/// Returns `None` in every case where there is no state to show: `root` is not a git
/// working tree, `git` cannot be spawned, `git` exits non-zero, or — this is the
/// narrower case, easy to mis-read as "any timeout" — a timeout or an over-cap read
/// struck before even one `# branch.*` header had been parsed. `GitState` has no way to
/// represent an unknown `Head`, so there is nothing to build a `Some` out of when no
/// header ever arrived; the cause (timeout vs. cap vs. malformed output) does not
/// change that. A timeout or a truncated read that struck *after* a header (and
/// possibly some status records) had already been parsed is different: that partial
/// parse is returned as `Some` with [`proto::GitState::stale`] set, per design
/// decision 9. `stale` is never set anywhere else.
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

    // Detected unconditionally, including on the stale path. Design decision 7 takes
    // the operation from `stat`, not from `status` output, and resolving the git dir
    // costs no extra process (see `resolve_git_dir`) — so the operation does not depend
    // on how much of `status` was read before the deadline or the cap. Skipping it when
    // `stale` is set lost the red `rebase` marker on exactly the repositories big enough
    // to time out, which is when it matters most.
    let operation = resolve_git_dir(root).and_then(|git_dir| detect_operation(&git_dir));

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

/// Resolves `root`'s own git dir straight from the filesystem — the same thing `git
/// rev-parse --absolute-git-dir` would report, without spawning it: design decision 5
/// allows exactly one probe command per refresh, and a probe runs on every debounced
/// change plus every 30s per registered root, so a second spawn here is real,
/// recurring cost.
///
/// `<root>/.git` is either a directory (an ordinary checkout: that directory *is* the
/// git dir) or a file containing `gitdir: <path>` (a linked worktree: that path is the
/// git dir, resolved against `root` when relative — this is exactly what makes a
/// linked worktree's git dir `<common-dir>/worktrees/<name>` instead of the shared
/// common dir). `None` when `<root>/.git` is neither, or can't be read: operation
/// detection is best-effort and never fails the probe itself.
pub(crate) fn resolve_git_dir(root: &Path) -> Option<PathBuf> {
    let dot_git = root.join(".git");
    let metadata = std::fs::metadata(&dot_git).ok()?;
    if metadata.is_dir() {
        return Some(dot_git);
    }
    if !metadata.is_file() {
        return None;
    }
    let contents = std::fs::read_to_string(&dot_git).ok()?;
    let pointer = contents.strip_prefix("gitdir:")?.trim();
    if pointer.is_empty() {
        return None;
    }
    let pointer = Path::new(pointer);
    Some(if pointer.is_absolute() {
        pointer.to_path_buf()
    } else {
        root.join(pointer)
    })
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
