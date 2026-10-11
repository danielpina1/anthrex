//! Where a linked worktree lives on disk, and why one is not safe to remove: the names
//! the daemon and the client's dialogs both use (milestone 9.12 decision 10). Pure.

use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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

/// Why a worktree is not safe to remove — which of [`dirty_reason`]'s questions fired.
///
/// It exists so the refusal can say what is actually true. A single `bool` left
/// [`WorktreeError::Dirty`] with one sentence for six different states, and that sentence
/// —"uncommitted or untracked changes" — is *false* for the states that hold commits
/// rather than files. That matters more than tidiness: the daemon's message is the sole
/// basis for the user's force-or-keep decision (design decision 24), and a user told
/// about uncommitted changes who runs `git status`, sees a spotlessly clean tree, and
/// concludes the daemon is confused will force — destroying the paused rebase, the
/// unreachable commits or the bisect that `git status` never mentioned.
///
/// Every variant's wording describes what would happen to the worktree's **files and
/// commits**, never to the agent's process: the dirty check runs before the agent is
/// signalled, so on the common path it is still running while this text is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyReason {
    /// `rebase-merge` or `rebase-apply`: a rebase stopped part-way.
    Rebase,
    /// `MERGE_HEAD`: a merge stopped, usually at a conflict.
    Merge,
    /// `CHERRY_PICK_HEAD`: a cherry-pick stopped, usually at a conflict.
    CherryPick,
    /// `REVERT_HEAD`: a revert stopped, usually at a conflict.
    Revert,
    /// `sequencer`: a multi-commit cherry-pick or revert has picks left to replay, whose
    /// current conflict was resolved with plain `git commit` rather than `--continue`.
    Sequence,
    /// `BISECT_LOG`: a bisect in progress.
    Bisect,
    /// `status --porcelain` reported something: modified or untracked files.
    Changes,
    /// `HEAD` reaches commits that no branch, tag or remote does.
    UnreachableHead,
}

impl DirtyReason {
    /// The sentence that follows `worktree <path> ` in the refusal.
    fn describe(self) -> &'static str {
        match self {
            Self::Rebase => {
                "has a rebase in progress; removing it discards the rebase and every \
                 commit it has already replayed"
            }
            Self::Merge => {
                "has a merge in progress; removing it discards the merge and everything \
                 resolved in it so far"
            }
            Self::CherryPick => {
                "has a cherry-pick in progress; removing it discards the cherry-pick and \
                 everything resolved in it so far"
            }
            Self::Revert => {
                "has a revert in progress; removing it discards the revert and everything \
                 resolved in it so far"
            }
            Self::Sequence => {
                "has a cherry-pick or revert in progress; removing it discards the commits \
                 it still has to replay"
            }
            Self::Bisect => {
                "has a bisect in progress; removing it discards every good and bad answer \
                 recorded so far"
            }
            Self::Changes => "has uncommitted or untracked changes",
            Self::UnreachableHead => {
                "is detached at commits no branch, tag or remote reaches; removing it \
                 discards those commits"
            }
        }
    }
}

impl std::fmt::Display for DirtyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}
