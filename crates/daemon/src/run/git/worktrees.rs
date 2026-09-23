//! The run's engine-owned worktrees (decisions 16, 18 and 19): the integration
//! worktree on `anthrex/<run>/integration`, a task's worktree on `anthrex/<run>/<task>`
//! (created, reused, re-added or re-pointed), their locks, and a review round's
//! detached worktree with the diff the reviewer is given (ruling Q4). Blocking; every
//! write carries decision 18's flags through [`Git::write`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Git, diff, failure, os};

/// What `git worktree list --porcelain -z` says about one worktree.
struct Listed {
    /// `refs/heads/<branch>`, or `None` when detached.
    branch: Option<String>,
    locked: bool,
}

/// Decision 18's lock reason, `anthrex run <run>`, with `<run>` read from the branch
/// (`anthrex/<run>/<task>`); a branch of any other shape is used whole.
fn lock_reason(branch: &str) -> String {
    let run = branch
        .strip_prefix("anthrex/")
        .and_then(|rest| rest.rsplit_once('/'))
        .map(|(run, _)| run)
        .unwrap_or(branch);
    format!("anthrex run {run}")
}

/// `path` spelled the way git records worktree paths: canonical where it exists, else
/// its canonical parent joined with its name.
fn normalize(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// The entry git has for `path`, if any.
fn listed(g: Git<'_>, root: &Path, path: &Path) -> Result<Option<Listed>, String> {
    let list = g.ok(
        root,
        &[os("worktree"), os("list"), os("--porcelain"), os("-z")],
    )?;
    let wanted = normalize(path);
    // Each attribute ends in NUL; an empty field ends a worktree's block.
    for block in list.split("\0\0") {
        let mut fields = block.split('\0').filter(|field| !field.is_empty());
        let Some(first) = fields.next() else { continue };
        let Some(listed_path) = first.strip_prefix("worktree ") else {
            continue;
        };
        if normalize(Path::new(listed_path)) != wanted {
            continue;
        }
        let mut entry = Listed {
            branch: None,
            locked: false,
        };
        for field in fields {
            if let Some(branch) = field.strip_prefix("branch ") {
                entry.branch = Some(branch.to_string());
            } else if field == "locked" || field.starts_with("locked ") {
                entry.locked = true;
            }
        }
        return Ok(Some(entry));
    }
    Ok(None)
}

/// Forgets a registered worktree whose directory is gone: unlock (a locked entry is
/// never pruned), then prune.
fn forget_missing(g: Git<'_>, root: &Path, path: &Path, entry: &Listed) -> Result<(), String> {
    if entry.locked {
        g.write(root, &[os("worktree"), os("unlock"), path.as_os_str()])?;
    }
    g.write(root, &[os("worktree"), os("prune")])?;
    Ok(())
}

fn branch_head(g: Git<'_>, root: &Path, branch: &str) -> Result<Option<String>, String> {
    let refname = format!("refs/heads/{branch}");
    let output = g.read(
        root,
        &[os("rev-parse"), os("-q"), os("--verify"), os(&refname)],
    )?;
    Ok(output.success.then(|| output.stdout.trim().to_string()))
}

/// `git merge-base --is-ancestor`: exit 0 is yes, exit 1 (silent) is no, anything
/// with an error message is an error.
fn is_ancestor(g: Git<'_>, dir: &Path, ancestor: &str, of: &str) -> Result<bool, String> {
    let args = [os("merge-base"), os("--is-ancestor"), os(ancestor), os(of)];
    let output = g.read(dir, &args)?;
    if output.success {
        Ok(true)
    } else if output.stderr.trim().is_empty() {
        Ok(false)
    } else {
        Err(failure(&args, &output))
    }
}

/// The branch `branch` checked out at `path`, locked with decision 18's reason:
/// created from `from` when the branch does not exist; reused when both exist;
/// re-added when the branch exists without its worktree. With `repoint`, an existing
/// branch that has no commit of its own (its head is a strict ancestor of `from`) is
/// moved to `from` with `git checkout -B` in its worktree (decision 19). Returns the
/// worktree's `HEAD`.
fn ensure_worktree(
    g: Git<'_>,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
    repoint: bool,
) -> Result<String, String> {
    let reason = lock_reason(branch);
    let mut entry = listed(g, root, path)?;
    if let Some(found) = &entry
        && !path.exists()
    {
        forget_missing(g, root, path, found)?;
        entry = None;
    }
    let add = [
        os("worktree"),
        os("add"),
        os("--lock"),
        os("--reason"),
        os(&reason),
    ];
    match (branch_head(g, root, branch)?, entry) {
        (None, _) => {
            let mut args = add.to_vec();
            args.extend([os("-b"), os(branch), path.as_os_str(), os(from)]);
            g.write(root, &args)?;
        }
        (Some(_), None) => {
            let mut args = add.to_vec();
            args.extend([path.as_os_str(), os(branch)]);
            g.write(root, &args)?;
        }
        (Some(_), Some(found)) => {
            let wanted = format!("refs/heads/{branch}");
            if found.branch.as_deref() != Some(wanted.as_str()) {
                return Err(format!(
                    "worktree {} is not on {branch} (git lists {})",
                    path.display(),
                    found.branch.as_deref().unwrap_or("a detached HEAD")
                ));
            }
            if !found.locked {
                lock(g, root, path, &reason)?;
            }
        }
    }
    if repoint
        && let Some(head) = branch_head(g, root, branch)?
        && head != from
        && is_ancestor(g, path, &head, from)?
    {
        g.write(
            path,
            &[os("checkout"), os("-q"), os("-B"), os(branch), os(from)],
        )?;
    }
    Ok(g.ok(path, &[os("rev-parse"), os("HEAD")])?
        .trim()
        .to_string())
}

/// Decision 16: the run branch `anthrex/<run>/integration` at `base_sha`, checked out
/// and locked at `path`; an existing one is reused as it is. Returns its `HEAD`.
pub fn create_run_branch(
    git: &OsStr,
    root: &Path,
    branch: &str,
    base_sha: &str,
    path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    ensure_worktree(Git::new(git, timeout), root, branch, base_sha, path, false)
}

/// Decision 19: the task branch `anthrex/<run>/<task>` from `from` (the run head at
/// dispatch), checked out and locked at `path`. Reuses an existing branch and path,
/// re-adds an existing branch whose worktree is gone, and re-points a branch that has
/// no commit of its own to `from`. Returns its `HEAD`.
pub fn prepare_worktree(
    git: &OsStr,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    ensure_worktree(Git::new(git, timeout), root, branch, from, path, true)
}

/// Decision 18: `git worktree lock --reason <reason> <path>`. Already locked is fine.
pub fn lock_worktree(
    git: &OsStr,
    root: &Path,
    path: &Path,
    reason: &str,
    timeout: Duration,
) -> Result<(), String> {
    lock(Git::new(git, timeout), root, path, reason)
}

fn lock(g: Git<'_>, root: &Path, path: &Path, reason: &str) -> Result<(), String> {
    let args = [
        os("worktree"),
        os("lock"),
        os("--reason"),
        os(reason),
        path.as_os_str(),
    ];
    let output = g.write_raw(root, &args)?;
    if output.success || output.stderr.contains("is already locked") {
        Ok(())
    } else {
        Err(failure(&args, &output))
    }
}

fn resolve_commit(g: Git<'_>, root: &Path, reference: &str) -> Result<String, String> {
    let spec = format!("{reference}^{{commit}}");
    let output = g.read(
        root,
        &[os("rev-parse"), os("-q"), os("--verify"), os(&spec)],
    )?;
    if output.success {
        Ok(output.stdout.trim().to_string())
    } else {
        Err(format!("{reference} is not a commit"))
    }
}

/// Decision 35 and ruling Q4: a fresh review worktree at `path`, detached at
/// `head_ref`, replacing any earlier round's (`git worktree remove --force`), and the
/// reviewer's diff `git diff <base>..<head>` clamped to `REVIEW_DIFF_MAX`. Returns
/// `(base, head, patch)` with both refs resolved to full shas.
pub fn prepare_review(
    git: &OsStr,
    root: &Path,
    head_ref: &str,
    base_ref: &str,
    path: &Path,
    timeout: Duration,
) -> Result<(String, String, String), String> {
    let g = Git::new(git, timeout);
    let base = resolve_commit(g, root, base_ref)?;
    let head = resolve_commit(g, root, head_ref)?;
    if let Some(found) = listed(g, root, path)? {
        if !path.exists() {
            forget_missing(g, root, path, &found)?;
        } else {
            if found.locked {
                g.write(root, &[os("worktree"), os("unlock"), path.as_os_str()])?;
            }
            g.write(
                root,
                &[
                    os("worktree"),
                    os("remove"),
                    os("--force"),
                    path.as_os_str(),
                ],
            )?;
        }
    }
    g.write(
        root,
        &[
            os("worktree"),
            os("add"),
            os("--detach"),
            path.as_os_str(),
            os(&head),
        ],
    )?;
    let patch = diff(g, root, &format!("{base}..{head}"))?;
    Ok((base, head, patch))
}
