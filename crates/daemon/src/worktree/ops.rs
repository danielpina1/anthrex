//! What the daemon does to one worktree: create it, ask whether it has changes, remove
//! it, and discard one whose create failed part-way (design decisions 5, 7 to 10, 12,
//! 13 and 16).
//!
//! Everything here is blocking and runs one git command at a time through
//! [`super::run_git`], under the single deadline its caller opened (design decisions 3
//! and 4). The manager calls it only inside `tokio::task::spawn_blocking`, never under
//! its own lock, and no function here knows anything about windows, the registry or
//! tokio.
//!
//! # The one rule this file exists to keep
//!
//! **Nothing here ever deletes a directory itself.** There is no `fs::remove_dir_all`
//! and there must never be one. Every deletion goes through `git worktree remove`, which
//! refuses a path that is not a registered working tree of the repository it is run in
//! (verified against git 2.50.1: `fatal: '<path>' is not a working tree`, exit 128, the
//! directory's contents untouched). That refusal is the backstop under
//! [`discard_new`]: even if a cleanup were somehow handed a path this daemon did not
//! create, git declines rather than deleting it. [`create`] adds the first guard by
//! refusing outright when the target path already exists (design decision 7), so a
//! directory that was already there is never a candidate for cleanup in the first place.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{
    CLEANUP_TIMEOUT, RESERVED_DIR, WorktreeError, branch_dir_name, check_branch_syntax,
    repo_worktrees_dir, run_git,
};
use crate::project;

/// One worktree this daemon created and owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedWorktree {
    /// The project root: the main checkout every linked worktree of this repository
    /// shares, canonical, as `project::detect_roots` resolved it (design decision 5).
    pub repo_root: PathBuf,
    /// The linked worktree, and this window's worktree root (design decision 21).
    pub path: PathBuf,
    pub branch: String,
}

/// A worktree [`create`] just made, and whether the branch under it is new.
///
/// `created_branch` is the only thing that licenses [`discard_new`] to delete a branch:
/// a branch this same create started at `HEAD` moments ago holds nothing that is not
/// already reachable, while a branch that existed before the create may hold work
/// nobody else has. Removal proper (design decision 13) never deletes a branch at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Created {
    pub worktree: ManagedWorktree,
    pub created_branch: bool,
}

/// Creates a linked worktree for `branch` from the repository containing `dir`.
///
/// The checks run in the order design decision 10 fixes, stopping at the first failure:
/// branch syntax, the roots, `check-ref-format`, the reserved `runs` directory name, the
/// branch being checked out elsewhere, and the path already existing. Nothing is written
/// to disk until every one of them has passed — `<wt>` itself is only created
/// immediately before `git worktree add`, so a refused create leaves no directory behind
/// at all.
///
/// `deadline` is shared by every git command this runs (design decision 3). Root
/// detection carries its own `DETECT_TIMEOUT` outside it, as decision 5 specifies, so
/// the worst case is one detection timeout plus the operation deadline.
///
/// If `git worktree add` fails or times out, [`discard_new`] runs before returning, on
/// its own fresh [`CLEANUP_TIMEOUT`] deadline, and the *original* error is returned
/// unchanged. (Design decision 16's `; the new worktree was removed` suffix belongs to
/// `WindowManager::create`'s phase B, which is the layer that reports to the client.)
pub fn create(
    git: &OsStr,
    dir: &Path,
    branch: &str,
    worktrees_root: &Path,
    deadline: Instant,
) -> Result<Created, WorktreeError> {
    check_branch_syntax(branch)?;

    // Design decision 5: one implementation of "which repository is this?", milestone
    // 4.5's. `worktree.is_none()` is "not a git working tree", which covers a bare
    // repository too; `project` is the main checkout, already canonical.
    let roots = project::detect_roots(dir);
    if roots.worktree.is_none() {
        return Err(WorktreeError::NotARepo {
            dir: dir.to_path_buf(),
        });
    }
    let project_root = roots.project;

    check_ref_format(git, dir, branch, deadline)?;

    let dir_name = branch_dir_name(branch);
    if dir_name == RESERVED_DIR {
        return Err(WorktreeError::InvalidBranch(format!(
            "branch name '{RESERVED_DIR}' is reserved"
        )));
    }

    // Design decision 9: ask before `worktree add` does, because git's own refusal is
    // worded differently across versions and the daemon's message must not be.
    if let Some(path) = branch_checkout_path(git, dir, branch, deadline)? {
        return Err(WorktreeError::BranchInUse {
            branch: branch.to_string(),
            path,
        });
    }

    let repo_dir = repo_worktrees_dir(worktrees_root, &project_root);
    let path = repo_dir.join(&dir_name);
    // `symlink_metadata`, not `Path::exists`: a symlink pointing nowhere is still
    // something that is already there, and the cautious answer to "is this path taken?"
    // is yes. Design decision 7 also has this catching `feat/x` against an existing
    // `feat-x`, which shares a directory name.
    if fs::symlink_metadata(&path).is_ok() {
        return Err(WorktreeError::PathExists { path });
    }

    let created_branch = !branch_exists(git, dir, branch, deadline)?;

    fs::create_dir_all(&repo_dir).map_err(|error| WorktreeError::Git {
        action: "worktree add".to_string(),
        stderr: format!(
            "could not create the worktree directory {}: {error}",
            repo_dir.display()
        ),
    })?;

    let added = if created_branch {
        // Starts the branch at the `HEAD` of the directory the user chose.
        run_git(
            git,
            dir,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new(branch),
                path.as_os_str(),
            ],
            deadline,
        )
    } else {
        run_git(
            git,
            dir,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                path.as_os_str(),
                OsStr::new(branch),
            ],
            deadline,
        )
    };

    let created = Created {
        worktree: ManagedWorktree {
            repo_root: project_root,
            path,
            branch: branch.to_string(),
        },
        created_branch,
    };

    match added {
        Ok(output) if output.success => {}
        Ok(output) => {
            return Err(clean_up_after(
                git,
                &created,
                WorktreeError::Git {
                    action: "worktree add".to_string(),
                    stderr: output.stderr_tail(),
                },
            ));
        }
        Err(error) => return Err(clean_up_after(git, &created, error)),
    }

    // The created path becomes the git registry's key for this window (design decision
    // 21), and milestone 4.5 keys its state by the canonical root the probe reports, so
    // resolve it here rather than leaving two spellings of one directory in play.
    let mut created = created;
    created.worktree.path = created
        .worktree
        .path
        .canonicalize()
        .unwrap_or(created.worktree.path);
    Ok(created)
}

/// Design decision 12: whether the worktree at `path` holds anything a removal would
/// destroy. Untracked files count; ignored files do not, and a plain `git worktree
/// remove` deletes those itself (both verified against git 2.50.1).
///
/// Any output at all means dirty — not "any non-whitespace output". Git prints nothing
/// for a clean tree, so the two agree in practice, and where they could ever disagree
/// the answer that refuses to delete is the right one.
pub fn is_dirty(git: &OsStr, path: &Path, deadline: Instant) -> Result<bool, WorktreeError> {
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
    Ok(!output.stdout.is_empty())
}

/// Design decision 13: removes `wt`, refusing when its tree has changes unless `force`.
/// The branch is never deleted, whatever happens here.
///
/// The refusal is git's own — a plain `git worktree remove` declines a tree with
/// modified or untracked files — and [`is_dirty`] only classifies that refusal into the
/// [`WorktreeError::Dirty`] the client turns into the force prompt. If the classifying
/// `status` cannot be run, the removal still fails, with git's own message: the
/// classification decides *which* refusal the user sees, never *whether* the worktree
/// survives.
///
/// A `wt.path` that is already gone is not an error. The directory is what the user
/// cares about; git just needs to stop listing it, which `worktree prune` does.
pub fn remove(
    git: &OsStr,
    wt: &ManagedWorktree,
    force: bool,
    deadline: Instant,
) -> Result<(), WorktreeError> {
    if fs::symlink_metadata(&wt.path).is_err() {
        return prune(git, &wt.repo_root, deadline);
    }

    let mut args = vec![OsStr::new("worktree"), OsStr::new("remove")];
    if force {
        args.push(OsStr::new("--force"));
    }
    args.push(wt.path.as_os_str());

    let output = run_git(git, &wt.repo_root, &args, deadline)?;
    if output.success {
        return Ok(());
    }

    if !force && matches!(is_dirty(git, &wt.path, deadline), Ok(true)) {
        return Err(WorktreeError::Dirty {
            path: wt.path.clone(),
        });
    }
    Err(WorktreeError::Git {
        action: "worktree remove".to_string(),
        stderr: output.stderr_tail(),
    })
}

/// Design decision 16: undoes a [`create`] whose window never came up — `git worktree
/// remove --force`, then `git worktree prune`, then `git branch -D` only for a branch
/// this same create made.
///
/// Runs on its own fresh [`CLEANUP_TIMEOUT`] deadline, because the deadline that failed
/// is by definition no longer usable.
///
/// `--force` is right here and nowhere else: the tree was checked out seconds ago by
/// this daemon and no agent ever ran in it, so there is no user work in it to lose. What
/// bounds the damage is not the absence of `--force` but *which path* it is pointed at,
/// and that is the module-level rule above: only a path `create` established did not
/// exist, removed only through git, which refuses anything that is not a working tree.
pub fn discard_new(git: &OsStr, created: &Created) -> Result<(), WorktreeError> {
    let deadline = Instant::now() + CLEANUP_TIMEOUT;
    let wt = &created.worktree;

    if fs::symlink_metadata(&wt.path).is_ok() {
        let output = run_git(
            git,
            &wt.repo_root,
            &[
                OsStr::new("worktree"),
                OsStr::new("remove"),
                OsStr::new("--force"),
                wt.path.as_os_str(),
            ],
            deadline,
        )?;
        if !output.success {
            return Err(WorktreeError::Git {
                action: "worktree remove --force".to_string(),
                stderr: output.stderr_tail(),
            });
        }
    }

    prune(git, &wt.repo_root, deadline)?;

    if !created.created_branch {
        return Ok(());
    }
    // A `worktree add` that died early may never have got as far as creating the branch.
    // Asking first keeps that case a successful cleanup instead of a spurious "cleanup
    // failed", and asking can delete nothing.
    if !branch_exists(git, &wt.repo_root, &wt.branch, deadline)? {
        return Ok(());
    }
    let output = run_git(
        git,
        &wt.repo_root,
        &[
            OsStr::new("branch"),
            OsStr::new("-D"),
            OsStr::new(&wt.branch),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "branch -D".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    Ok(())
}

/// Runs [`discard_new`] for a create that has just failed and returns `original`
/// unchanged. A cleanup that fails is logged at `warn` and swallowed: the caller must
/// hear about the failure that actually stopped the create, not about the tidying.
fn clean_up_after(git: &OsStr, created: &Created, original: WorktreeError) -> WorktreeError {
    if let Err(error) = discard_new(git, created) {
        tracing::warn!(
            path = ?created.worktree.path,
            branch = %created.worktree.branch,
            %error,
            "could not clean up the worktree of a failed create"
        );
    }
    original
}

fn prune(git: &OsStr, repo_root: &Path, deadline: Instant) -> Result<(), WorktreeError> {
    let output = run_git(
        git,
        repo_root,
        &[OsStr::new("worktree"), OsStr::new("prune")],
        deadline,
    )?;
    if output.success {
        return Ok(());
    }
    Err(WorktreeError::Git {
        action: "worktree prune".to_string(),
        stderr: output.stderr_tail(),
    })
}

/// Design decision 8, rule 5: `check-ref-format --branch` must exit zero *and* echo the
/// branch unchanged. The second half is what rejects the `@{-1}`-style names git would
/// happily expand into some other branch entirely.
fn check_ref_format(
    git: &OsStr,
    dir: &Path,
    branch: &str,
    deadline: Instant,
) -> Result<(), WorktreeError> {
    let output = run_git(
        git,
        dir,
        &[
            OsStr::new("check-ref-format"),
            OsStr::new("--branch"),
            OsStr::new(branch),
        ],
        deadline,
    )?;
    if output.success && output.stdout.trim() == branch {
        return Ok(());
    }
    Err(WorktreeError::InvalidBranch(format!(
        "invalid branch name '{branch}'"
    )))
}

fn branch_exists(
    git: &OsStr,
    dir: &Path,
    branch: &str,
    deadline: Instant,
) -> Result<bool, WorktreeError> {
    let refname = OsString::from(format!("refs/heads/{branch}"));
    let output = run_git(
        git,
        dir,
        &[
            OsStr::new("show-ref"),
            OsStr::new("--verify"),
            OsStr::new("--quiet"),
            &refname,
        ],
        deadline,
    )?;
    Ok(output.success)
}

/// Design decision 9: the checkout holding `branch`, if any, from `git worktree list
/// --porcelain`. This covers the main checkout as well as every linked worktree.
fn branch_checkout_path(
    git: &OsStr,
    dir: &Path,
    branch: &str,
    deadline: Instant,
) -> Result<Option<PathBuf>, WorktreeError> {
    let output = run_git(
        git,
        dir,
        &[
            OsStr::new("worktree"),
            OsStr::new("list"),
            OsStr::new("--porcelain"),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "worktree list".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    Ok(parse_branch_checkout(&output.stdout, branch))
}

/// The `worktree ` line of the porcelain record whose `branch ` line is `branch`.
///
/// The porcelain format is one record per blank-line-separated block, `worktree <path>`
/// first; a detached checkout has `detached` where `branch refs/heads/...` would be, and
/// a bare main repository has `bare`. Both simply never match.
fn parse_branch_checkout(porcelain: &str, branch: &str) -> Option<PathBuf> {
    let wanted = format!("refs/heads/{branch}");
    let mut current: Option<&str> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(path);
        } else if line.strip_prefix("branch ") == Some(wanted.as_str()) {
            return current.map(PathBuf::from);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORCELAIN: &str = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\n\
         worktree /wt/detached\nHEAD def\ndetached\n\n\
         worktree /wt/feat-api\nHEAD abc\nbranch refs/heads/feat/api\n";

    #[test]
    fn parse_branch_checkout_finds_the_holder_of_a_branch() {
        assert_eq!(
            parse_branch_checkout(PORCELAIN, "main"),
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            parse_branch_checkout(PORCELAIN, "feat/api"),
            Some(PathBuf::from("/wt/feat-api"))
        );
    }

    #[test]
    fn parse_branch_checkout_ignores_detached_and_unrelated_records() {
        assert_eq!(parse_branch_checkout(PORCELAIN, "feat"), None);
        assert_eq!(parse_branch_checkout(PORCELAIN, "refs/heads/main"), None);
        assert_eq!(parse_branch_checkout(PORCELAIN, "detached"), None);
        assert_eq!(parse_branch_checkout("", "main"), None);
    }

    /// A prefix must not count as a match: `feat` and `feat/api` are different branches
    /// and only one of them is in use.
    #[test]
    fn parse_branch_checkout_matches_the_whole_ref() {
        let porcelain = "worktree /wt/feature\nbranch refs/heads/feature\n";
        assert_eq!(parse_branch_checkout(porcelain, "feat"), None);
        assert_eq!(
            parse_branch_checkout(porcelain, "feature"),
            Some(PathBuf::from("/wt/feature"))
        );
    }
}
