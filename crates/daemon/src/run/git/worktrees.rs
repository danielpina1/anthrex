//! The run's engine-owned worktrees (decisions 16, 18 and 19): the integration
//! worktree on `anthrex/<run>/integration`, a task's worktree, detached, whose work the
//! engine records on `anthrex/<run>/<task>` (created, reused, re-added or re-pointed;
//! final fix batch F1b), their locks, and a review round's
//! detached worktree with the diff the reviewer is given (ruling Q4). Blocking; every
//! write carries decision 18's flags through [`Git::write`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::import::{move_head, sync_in};
use super::{Git, diff, failure, os};
use crate::worktree::pinned::{self, PinAs};

/// What `git worktree list --porcelain -z` says about one worktree.
pub(crate) struct Listed {
    /// `refs/heads/<branch>`, or `None` when detached.
    pub(crate) branch: Option<String>,
    /// The worktree's `HEAD` commit, `None` when git lists none (M8a.21's reconcile).
    pub(crate) head: Option<String>,
    pub(crate) locked: bool,
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
pub(crate) fn listed(g: Git<'_>, root: &Path, path: &Path) -> Result<Option<Listed>, String> {
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
            head: None,
            locked: false,
        };
        for field in fields {
            if let Some(branch) = field.strip_prefix("branch ") {
                entry.branch = Some(branch.to_string());
            } else if let Some(head) = field.strip_prefix("HEAD ") {
                entry.head = Some(head.to_string());
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
pub(crate) fn forget_missing(
    g: Git<'_>,
    root: &Path,
    path: &Path,
    entry: &Listed,
) -> Result<(), String> {
    if entry.locked {
        g.write(root, &[os("worktree"), os("unlock"), path.as_os_str()])?;
    }
    g.write(root, &[os("worktree"), os("prune")])?;
    Ok(())
}

/// The commit `branch` points at, or `None` when it does not exist. Fix round 4, S1: a
/// branch that is a symbolic ref (a worker can write its own task branch's file) is
/// refused, never resolved to the tip of the branch it names.
fn branch_head(g: Git<'_>, root: &Path, branch: &str) -> Result<Option<String>, String> {
    let refname = format!("refs/heads/{branch}");
    not_symbolic(g, root, &refname)?;
    let output = g.read(
        root,
        &[os("rev-parse"), os("-q"), os("--verify"), os(&refname)],
    )?;
    Ok(output.success.then(|| output.stdout.trim().to_string()))
}

/// Fix round 4, S1: refuses `refname` when it is a symbolic ref (`ref: …`, or a
/// symbolic link git reads as one), which would make the engine read another branch's
/// tip as its own. A run's own branch (`refs/heads/anthrex/…`, which a worker can write)
/// is also refused when its loose file is any symbolic link: git reads through one that
/// is not a ref name.
fn not_symbolic(g: Git<'_>, root: &Path, refname: &str) -> Result<(), String> {
    let output = g.read(root, &[os("symbolic-ref"), os("-q"), os(refname)])?;
    if output.success {
        return Err(format!(
            "{refname} is a symbolic ref to {}; the branch was tampered with",
            output.stdout.trim()
        ));
    }
    if refname.starts_with("refs/heads/anthrex/") {
        let file = common_dir(g, root)?.join(refname);
        if std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(format!(
                "{refname} is a symbolic link; the branch was tampered with"
            ));
        }
    }
    Ok(())
}

/// `git merge-base --is-ancestor`: exit 0 is yes, exit 1 (silent) is no, anything
/// with an error message is an error.
pub(crate) fn is_ancestor(
    g: Git<'_>,
    dir: &Path,
    ancestor: &str,
    of: &str,
) -> Result<bool, String> {
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

/// The run branch `branch` checked out at `path` (the integration worktree, which no
/// worker writes), locked with decision 18's reason: created from `from` when the branch
/// does not exist; reused when both exist; re-added when the branch exists without its
/// worktree. Returns the worktree's `HEAD`.
fn ensure_run_worktree(
    g: Git<'_>,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
) -> Result<String, String> {
    let reason = lock_reason(branch);
    let entry = current_entry(g, root, path)?;
    let add = [
        os("worktree"),
        os("add"),
        os("--lock"),
        os("--reason"),
        os(&reason),
    ];
    match (branch_head(g, root, branch)?, entry) {
        (None, _) => {
            refuse_a_parent_branch(g, root, branch)?;
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
    let head = format!("refs/heads/{branch}");
    pin_in(
        g,
        root,
        path,
        PinAs {
            head: Some(head),
            ..PinAs::default()
        },
    )?;
    // Through the pin's `HEAD` check: a `HEAD` that no longer names the branch fails here.
    Ok(g.ok(path, &[os("rev-parse"), os("HEAD")])?
        .trim()
        .to_string())
}

/// `git worktree list`'s entry for `path`, after forgetting one whose directory is gone.
fn current_entry(g: Git<'_>, root: &Path, path: &Path) -> Result<Option<Listed>, String> {
    let entry = listed(g, root, path)?;
    if let Some(found) = &entry
        && !path.exists()
    {
        forget_missing(g, root, path, found)?;
        return Ok(None);
    }
    Ok(entry)
}

/// Final fix batch F1b: a task's worktree at `path`, **detached**, locked with decision
/// 18's reason, and its branch `branch`, which only the engine writes. The branch is
/// created at `from` when it does not exist; the worktree is added detached at the
/// branch's tip when git does not list it, and reused (it must be detached) when it
/// does. The worker's `HEAD` is then imported onto the branch ([`sync_in`]). With
/// `repoint`, a task that has no commit of its own (its `HEAD` a strict ancestor of
/// `from`) is moved to `from` (decision 19). Returns the worktree's `HEAD`.
fn ensure_task_worktree(
    g: Git<'_>,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
    objects: Option<&Path>,
) -> Result<String, String> {
    let reason = lock_reason(branch);
    let own = format!("refs/heads/{branch}");
    let entry = current_entry(g, root, path)?;
    let tip = match branch_head(g, root, branch)? {
        Some(tip) => tip,
        None => {
            refuse_a_parent_branch(g, root, branch)?;
            let args = [
                os("update-ref"),
                os("--no-deref"),
                os(&own),
                os(from),
                os(""),
            ];
            g.write(root, &args)?;
            from.to_string()
        }
    };
    match entry {
        None => {
            let args = [
                os("worktree"),
                os("add"),
                os("--lock"),
                os("--reason"),
                os(&reason),
                os("--detach"),
                path.as_os_str(),
                os(&tip),
            ];
            g.write(root, &args)?;
        }
        Some(found) => {
            if let Some(on) = found.branch {
                return Err(format!(
                    "worktree {} is on {on}; a task worktree is detached",
                    path.display()
                ));
            }
            if !found.locked {
                lock(g, root, path, &reason)?;
            }
        }
    }
    pin_in(
        g,
        root,
        path,
        PinAs {
            head: None,
            own: Some(own.clone()),
            objects: objects.map(Path::to_path_buf),
        },
    )?;
    let head = sync_in(g, path)?;
    if head != from && is_ancestor(g, path, &head, from)? {
        // Decision 19 as clarified by ruling T8-I4: the task has no commit of its own,
        // so the worktree holds only what the engine's setup made. Its edits to tracked
        // files are dropped (a lockfile setup rewrote must not leak into the task's
        // start state); untracked build output is kept. The caller re-runs setup after
        // a re-point, which it sees as a returned `HEAD` different from the task's
        // earlier start.
        //
        // Final fix batch F1, fix round 3, and F1b: the branch and the worktree's `HEAD`
        // are each moved by name, `--no-deref`, compare-and-swap from the head just
        // read, and the index and files follow with `read-tree`.
        let cas = [
            os("update-ref"),
            os("--no-deref"),
            os(&own),
            os(from),
            os(&head),
        ];
        let output = g.write_raw(path, &cas)?;
        if !output.success {
            return Err(format!(
                "{branch} moved while it was re-pointed: {}",
                failure(&cas, &output)
            ));
        }
        move_head(g, path, from, &head)?;
        g.write(path, &[os("read-tree"), os("-u"), os("--reset"), os(from)])?;
        return Ok(from.to_string());
    }
    Ok(head)
}

/// Git cannot create `a/b/c` while a branch `a/b` or `a` exists (a directory/file ref
/// conflict). Say so plainly rather than in git's words (fix round 1, finding 9). A
/// user branch named `anthrex/<run>` is the case that matters; decision 15's run-id
/// redraw should also treat it as taken (a carry for M8a.11 and M8a.22).
fn refuse_a_parent_branch(g: Git<'_>, root: &Path, branch: &str) -> Result<(), String> {
    let mut parent = branch;
    while let Some((up, _)) = parent.rsplit_once('/') {
        if branch_head(g, root, up)?.is_some() {
            return Err(format!(
                "branch {up} exists, so git cannot create {branch}; rename or delete {up}"
            ));
        }
        parent = up;
    }
    Ok(())
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
    ensure_run_worktree(Git::new(git, timeout), root, branch, base_sha, path)
}

/// Decision 19: the task branch `anthrex/<run>/<task>` from `from` (the run head at
/// dispatch) and its worktree at `path`, detached at the branch's tip and locked (final
/// fix batch F1b). Reuses an existing branch and path, re-adds an existing branch whose
/// worktree is gone, and re-points a task that has no commit of its own to `from`.
/// Returns its `HEAD`. The worker's objects are read only from the repository: see
/// [`prepare_task_worktree`] for a worker with a private object directory.
pub fn prepare_worktree(
    git: &OsStr,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
    timeout: Duration,
) -> Result<String, String> {
    prepare_task_worktree(git, root, branch, from, path, None, timeout)
}

/// [`prepare_worktree`] for a worker whose git writes its objects to `objects` (its
/// private object directory, final fix batch F1b): the engine imports its commits from
/// there, re-hashing each object, before it reads them.
pub fn prepare_task_worktree(
    git: &OsStr,
    root: &Path,
    branch: &str,
    from: &str,
    path: &Path,
    objects: Option<&Path>,
    timeout: Duration,
) -> Result<String, String> {
    ensure_task_worktree(Git::new(git, timeout), root, branch, from, path, objects)
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
    // A run's own branch named in place of a commit (a review of a task that has not
    // claimed one) is read only when it is not a symbolic ref (fix round 4, S1).
    if reference.starts_with("anthrex/") {
        not_symbolic(g, root, &format!("refs/heads/{reference}"))?;
    }
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
            repair_git_file(path)?;
            g.write(
                root,
                &[
                    os("worktree"),
                    os("remove"),
                    os("--force"),
                    path.as_os_str(),
                ],
            )?;
            pinned::unpin(path);
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
    pin_in(g, root, path, PinAs::default())?;
    let patch = diff(g, root, &format!("{base}..{head}"))?;
    Ok((base, head, patch))
}

/// Decision 33's proof worktree: a scratch worktree at `path`, detached at `at`,
/// **reused** when git already lists it with its directory (the caller then checks out
/// what it needs), re-added when its directory is gone. `true` when it was created now,
/// which is when the caller runs the profile's `setup` in it ("created on first use
/// with `setup`"). Never locked and never watched (decision 22): nothing of an agent's
/// lives in it.
pub fn prepare_scratch(
    git: &OsStr,
    root: &Path,
    path: &Path,
    at: &str,
    timeout: Duration,
) -> Result<bool, String> {
    let g = Git::new(git, timeout);
    if let Some(found) = listed(g, root, path)? {
        if path.exists() {
            pin_in(g, root, path, PinAs::default())?;
            return Ok(false);
        }
        forget_missing(g, root, path, &found)?;
    }
    g.write(
        root,
        &[
            os("worktree"),
            os("add"),
            os("--detach"),
            path.as_os_str(),
            os(at),
        ],
    )?;
    pin_in(g, root, path, PinAs::default())?;
    Ok(true)
}

/// `git rev-parse --absolute-git-dir` in `dir`: a linked worktree's own git directory
/// (`<common>/worktrees/<name>`), where per-worktree engine markers live. A read.
pub fn absolute_git_dir(git: &OsStr, dir: &Path, timeout: Duration) -> Result<PathBuf, String> {
    let g = Git::new(git, timeout);
    let out = g.ok(dir, &[os("rev-parse"), os("--absolute-git-dir")])?;
    Ok(PathBuf::from(out.trim_end_matches(['\n', '\r'])))
}

/// The repository's git common directory, read in `root` (the user's checkout, which
/// no worker writes).
pub(crate) fn common_dir(g: Git<'_>, root: &Path) -> Result<PathBuf, String> {
    let out = g.ok(
        root,
        &[
            os("rev-parse"),
            os("--path-format=absolute"),
            os("--git-common-dir"),
        ],
    )?;
    let common = PathBuf::from(out.trim_end_matches(['\n', '\r']));
    Ok(common.canonicalize().unwrap_or(common))
}

/// M8a final fix batch F1, fix round 1 (N2): pins the engine worktree `path` to its git
/// directory as the repository lists it, so no later call in it reads its `.git` file.
/// Its `HEAD` may name only `as_.head` (none: always detached), or be detached (fix round
/// 2, R1); a task worktree also carries its own branch and object directory (F1b).
fn pin_in(g: Git<'_>, root: &Path, path: &Path, as_: PinAs) -> Result<(), String> {
    let common = common_dir(g, root)?;
    match pinned::pin(&common, path, as_).broken {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// Pins each existing worktree of `worktrees` (its path, and what it is pinned as) in
/// the repository whose common directory is `common`: a daemon restart, before any call
/// in them. The git directory is found from the repository's side and must be unique;
/// one that cannot be found is pinned as broken, so every call in it is refused.
/// Blocking.
pub fn pin_worktrees(common: &Path, worktrees: &[(PathBuf, PinAs)]) {
    for (path, as_) in worktrees.iter().filter(|(path, _)| path.exists()) {
        pinned::pin(common, path, as_.clone());
    }
}

/// Puts back the `.git` file of a pinned worktree before git itself (`worktree remove`,
/// run from the user's checkout) reads it: a worker may have rewritten it.
pub(crate) fn repair_git_file(path: &Path) -> Result<(), String> {
    let Some(pin) = pinned::pinned(path) else {
        return Ok(());
    };
    if pin.broken.is_some() || !path.exists() {
        return Ok(());
    }
    let file = path.join(".git");
    let wanted = format!("gitdir: {}\n", pin.git_dir.display());
    if std::fs::read_to_string(&file).ok().as_deref() == Some(wanted.as_str()) {
        return Ok(());
    }
    if file.is_dir() {
        std::fs::remove_dir_all(&file)
    } else {
        std::fs::remove_file(&file).or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })
    }
    .and_then(|()| std::fs::write(&file, wanted))
    .map_err(|err| format!("cannot restore {}: {err}", file.display()))
}
