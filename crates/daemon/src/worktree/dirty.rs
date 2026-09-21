//! The one question a removal must never get wrong: does this worktree hold work?
//!
//! Design decision 12, as amended by ruling during task M5.3. It lives beside
//! [`super::ops`] rather than inside it because it is the safety-critical half of
//! removal and answers a question of its own — `ops` decides *what to do*, this decides
//! *whether it is safe to*. Every question here fails in the refusing direction: an
//! answer that cannot be obtained is an error, never a "no".

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use super::{WorktreeError, run_git};

/// The per-worktree files and directories whose existence means git has an operation
/// paused in this worktree. Each is resolved with `rev-parse --git-path`, because a
/// linked worktree's are under `.git/worktrees/<name>/`, not the repository's `.git/`.
const OPERATION_MARKERS: [&str; 4] = [
    "rebase-merge",
    "rebase-apply",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
];

/// Design decision 12 (extended by the M5.3 review's ruling): whether the worktree at
/// `path` holds anything a removal would destroy.
///
/// Three questions, any one of which means dirty:
///
/// 1. **A paused git operation** — `rebase-merge`, `rebase-apply`, `MERGE_HEAD` or
///    `CHERRY_PICK_HEAD`. A rebase stopped at `edit` with a clean tree reports *nothing*
///    in `status --porcelain`, and a plain `git worktree remove` deletes it and every
///    commit it had already replayed (verified against git 2.50.1). An agent told to try
///    an approach on scratch commits leaves exactly this state.
/// 2. **Working-tree changes** — `status --porcelain --ignore-submodules=none`. Untracked
///    files count; ignored files do not, and a plain `git worktree remove` deletes those
///    itself. *Any* output means dirty, not "any non-whitespace output": git prints
///    nothing for a clean tree, so the two agree in practice, and where they could ever
///    disagree the answer that refuses to delete is the right one.
/// 3. **A detached `HEAD` holding commits no ref reaches** — `rev-list --count HEAD --not
///    --branches --remotes --tags`. Also silent in `status`, and also deleted without
///    `--force`, at which point the commits are unreachable. On a branch the count is
///    always zero, because `--branches` covers that branch, so this question answers
///    itself for the ordinary case.
///
/// Questions 1 and 3 are why the caller must ask *before* `git worktree remove` rather
/// than only classifying a refusal afterwards: git does not refuse either state.
pub fn is_dirty(git: &OsStr, path: &Path, deadline: Instant) -> Result<bool, WorktreeError> {
    if operation_in_progress(git, path, deadline)? {
        return Ok(true);
    }

    let output = run_git(
        git,
        path,
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain"),
            OsStr::new("--ignore-submodules=none"),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "status".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    if !output.stdout.is_empty() {
        return Ok(true);
    }

    head_is_unreachable(git, path, deadline)
}

/// Whether any of [`OPERATION_MARKERS`] exists in this worktree's git directory.
fn operation_in_progress(
    git: &OsStr,
    path: &Path,
    deadline: Instant,
) -> Result<bool, WorktreeError> {
    let mut args = vec![OsStr::new("rev-parse")];
    for marker in OPERATION_MARKERS {
        args.push(OsStr::new("--git-path"));
        args.push(OsStr::new(marker));
    }
    let output = run_git(git, path, &args, deadline)?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "rev-parse --git-path".to_string(),
            stderr: output.stderr_tail(),
        });
    }

    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .any(|line| {
            // git 2.50.1 answers with absolute paths, but older versions answer relative
            // to the working directory, which is `path` here.
            let candidate = Path::new(line);
            let candidate = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                path.join(candidate)
            };
            match fs::symlink_metadata(&candidate) {
                Ok(_) => true,
                // Only "it is not there" means it is not there. Anything else is a
                // marker we cannot rule out, and the answer that refuses to delete wins.
                Err(error) => error.kind() != io::ErrorKind::NotFound,
            }
        }))
}

/// Whether `HEAD` reaches commits that no branch, remote-tracking branch or tag does.
fn head_is_unreachable(git: &OsStr, path: &Path, deadline: Instant) -> Result<bool, WorktreeError> {
    let output = run_git(
        git,
        path,
        &[
            OsStr::new("rev-list"),
            OsStr::new("--count"),
            OsStr::new("HEAD"),
            OsStr::new("--not"),
            OsStr::new("--branches"),
            OsStr::new("--remotes"),
            OsStr::new("--tags"),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "rev-list".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    let count: u64 = output
        .stdout
        .trim()
        .parse()
        .map_err(|_| WorktreeError::Git {
            action: "rev-list".to_string(),
            stderr: format!(
                "could not read a commit count from {:?}",
                output.stdout.trim()
            ),
        })?;
    Ok(count > 0)
}
