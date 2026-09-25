//! The end of a worktree and of a run (decision 20): salvaging a dirty worktree's work
//! to `refs/anthrex/salvage/<run>/<task>/<seq>` before it is removed, removing an
//! engine-owned (locked) worktree, deleting a run's branches, and accepting the run
//! branch into the base branch in the user's own checkout ([`super::accept`]).
//!
//! Blocking; call only from `spawn_blocking`. Every function here writes, so each
//! belongs behind the caller's [`super::GitQueue::write`]. The engine-owned writes carry
//! decision 18's flags; [`accept`], the only write into the user's checkout, carries
//! only [`super::NO_HOOKS`] (decision 18, as amended by final fix batch F1).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::read;
use super::worktrees::{forget_missing, listed, repair_git_file};
use super::{DIFF_FLAGS, Git, NO_NESTED, checkout, failure, import, nul_fields, os};

/// Decision 20's salvage. A worktree with nothing to save (`git status --porcelain`,
/// untracked files included, ignored ones not) writes nothing and gives `None`.
/// Otherwise its whole state — staged, unstaged and untracked, and a conflicted
/// merge's markers — is committed on top of `HEAD` with `message`, without moving any
/// branch (`add -A`, `write-tree`, `commit-tree -p HEAD`), and `reference` is created
/// at that commit; the result is `Some(reference)`.
///
/// A salvage ref is never overwritten. If `reference` already exists and holds the
/// same tree, this is the same salvage replayed (an op re-run after a crash) and gives
/// `Some(reference)` again; if it holds other work, this is an error, and the caller
/// should have taken the next `<seq>`.
pub fn salvage(
    git: &OsStr,
    worktree: &Path,
    reference: &str,
    message: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let g = Git::new(git, timeout);
    // Final fix batch F1c (C1): an index the worker replaced with a link (or anything
    // but a plain file) is removed, never written through, and rebuilt from `HEAD`
    // below: `add -A` then saves the worktree's files whatever the index held.
    let rebuild = match import::task_pin(worktree) {
        Ok(pin) => drop_tampered_index(&pin.git_dir)?,
        Err(_) => false,
    };
    // Final fix batch F1b: in a task worktree, the worker's detached `HEAD` is imported
    // and recorded on the task's branch first (its objects are in its private directory
    // until then), and it is the salvage's parent. A `HEAD` left naming a branch (on
    // which the worker's sandbox let it commit nothing) is put back at the branch's tip.
    let parent = if import::task_pin(worktree).is_ok() {
        import::redetach(g, worktree)?;
        import::sync_in(g, worktree)?
    } else {
        "HEAD".to_string()
    };
    if rebuild {
        g.write(worktree, &[os("read-tree"), os(&parent)])?;
    }
    // Final fix batch F1c (3a): a standalone checkout has no refs of its own; the salvage
    // ref is read and written in the user's repository.
    let refs_at = match crate::worktree::pinned::pinned(worktree) {
        Some(pin) if pin.standalone => pin.common_dir,
        _ => worktree.to_path_buf(),
    };
    let refs_at = refs_at.as_path();
    // `--untracked-files=normal` explicitly: a user's `status.showUntrackedFiles=no`
    // must not hide new files from the salvage.
    let args = [
        os("status"),
        os("--porcelain"),
        os("-z"),
        os("--untracked-files=normal"),
        os(NO_NESTED),
    ];
    // Only whether there is any output matters, so none of it is held: a worktree with
    // a vast untracked build tree is still salvaged, not failed over a cap.
    let (output, kept) = g.read_head(worktree, &args, 0)?;
    if !output.success {
        return Err(failure(&args, &output));
    }
    if kept.total == 0 {
        return Ok(None);
    }
    // Whether this salvage is refused is decided before `add -A` touches the index
    // (ruling T9-m4): a refused salvage leaves a hand-back's unmerged entries unmerged.
    if let Some(saved) = read(g, refs_at, reference)? {
        return if matches_existing(g, worktree, &saved)? {
            // The same salvage replayed (an op re-run after its `update-ref`).
            Ok(Some(reference.to_string()))
        } else {
            Err(format!("salvage ref {reference} already holds other work"))
        };
    }
    // Every gitlink is left out of `add -A`: staging a tracked nested repository runs
    // `git status` inside it, under its own (worker-written) config (fix round 1, N1).
    // Its uncommitted contents cannot be salvaged into this repository anyway.
    let gitlinks = gitlinks(g, worktree)?;
    let excludes: Vec<String> = gitlinks
        .iter()
        .map(|path| format!(":(exclude,literal){path}"))
        .collect();
    let mut add = vec![os("add"), os("-A"), os("--"), os(".")];
    add.extend(excludes.iter().map(|e| os(e)));
    g.write(worktree, &add)?;
    if parent != "HEAD" {
        // F1b: blobs the worker staged and left unchanged are still only in its private
        // directory; `write-tree` needs them in the repository.
        import::import_index(g, worktree)?;
    }
    let tree = g.write(worktree, &[os("write-tree")])?.trim().to_string();
    let commit = g
        .write(
            worktree,
            &[
                os("commit-tree"),
                os(&tree),
                os("-p"),
                os(&parent),
                os("-m"),
                os(message),
            ],
        )?
        .trim()
        .to_string();
    // An empty old value: the ref must not exist yet. The read above already refused an
    // existing one, so this guards only against another writer creating it since.
    g.write(
        refs_at,
        &[
            os("update-ref"),
            os("--no-deref"),
            os(reference),
            os(&commit),
            os(""),
        ],
    )?;
    Ok(Some(reference.to_string()))
}

/// Final fix batch F1c (C1): removes `<git_dir>/index` when it is not a plain file (a
/// link the worker planted: `unlink` removes the link itself). `true` when it did.
fn drop_tampered_index(git_dir: &Path) -> Result<bool, String> {
    if crate::worktree::engine_index::plain_or_missing(git_dir).is_ok() {
        return Ok(false);
    }
    let index = git_dir.join("index");
    let removed = match std::fs::symlink_metadata(&index) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&index),
        _ => std::fs::remove_file(&index),
    };
    removed.map_err(|err| format!("cannot remove the tampered {}: {err}", index.display()))?;
    Ok(true)
}

/// The gitlink (nested repository) paths in `dir`'s index.
fn gitlinks(g: Git<'_>, dir: &Path) -> Result<Vec<String>, String> {
    let listing = g.ok(dir, &[os("ls-files"), os("-s"), os("-z")])?;
    Ok(nul_fields(&listing)
        .filter(|entry| entry.starts_with("160000 "))
        .filter_map(|entry| entry.split_once('\t').map(|(_, path)| path.to_string()))
        .collect())
}

/// Whether the worktree holds exactly the salvage `saved` (the commit of the salvage
/// ref) already saved, read
/// without writing the index: no untracked (unignored) file, and no difference between
/// the working tree and the ref's tree over the paths either has. A first salvage's
/// `add -A` made every untracked file tracked, so a replay of it answers yes.
fn matches_existing(g: Git<'_>, worktree: &Path, saved: &str) -> Result<bool, String> {
    let others = [
        os("ls-files"),
        os("-z"),
        os("--others"),
        os("--exclude-standard"),
    ];
    let (output, kept) = g.read_head(worktree, &others, 0)?;
    if !output.success {
        return Err(failure(&others, &output));
    }
    if kept.total > 0 {
        return Ok(false);
    }
    let mut diff = vec![os("diff")];
    diff.extend(DIFF_FLAGS.map(os));
    diff.extend([os("--quiet"), os(saved), os("--")]);
    let output = g.read(worktree, &diff)?;
    if output.success {
        Ok(true)
    } else if output.stderr.trim().is_empty() {
        Ok(false)
    } else {
        Err(failure(&diff, &output))
    }
}

/// Decision 20's removal. A linked worktree (the integration worktree): `git worktree
/// unlock`, `git worktree remove --force`. A standalone checkout (final fix batch F1c,
/// 3a): its directory and its repository (whose `repo` is found from its pin, else
/// [`checkout::default_repo_dir`]) removed, the worker's private objects with it. It
/// removes whatever is there, dirty or not, so the caller salvages first. A worktree
/// already gone is not an error: the removal is already done, or is finished now.
pub fn remove_worktree(
    git: &OsStr,
    root: &Path,
    path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    remove_checkout(git, root, path, None, timeout)
}

/// [`remove_worktree`], naming a standalone checkout's repository `repo`
/// (`<data>/runs/<run>/tasks/<name>`), as the daemon does.
pub fn remove_checkout(
    git: &OsStr,
    root: &Path,
    path: &Path,
    repo: Option<&Path>,
    timeout: Duration,
) -> Result<(), String> {
    let g = Git::new(git, timeout);
    match listed(g, root, path)? {
        None => {
            let pin = crate::worktree::pinned::pinned(path).filter(|pin| pin.standalone);
            let repo = match (repo, &pin) {
                (Some(repo), _) => repo.to_path_buf(),
                (None, Some(pin)) => pin
                    .git_dir
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default(),
                (None, None) => checkout::default_repo_dir(path),
            };
            let repo = checkout::Repo::at(&repo);
            if pin.is_none() && repo.git_dir().join("HEAD").is_file() {
                let common = super::worktrees::common_dir(g, root)?;
                crate::worktree::pinned::pin(
                    &common,
                    path,
                    crate::worktree::pinned::PinAs {
                        repo: Some(repo.git_dir()),
                        ..Default::default()
                    },
                );
            }
            checkout::remove(path, &repo)?;
        }
        Some(entry) if !path.exists() => forget_missing(g, root, path, &entry)?,
        Some(entry) => {
            if entry.locked {
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
        }
    }
    // No unconditional `git worktree prune` (final fix batch F1, D-12): `worktree
    // remove` already deregisters, and a prune would also forget the user's own
    // worktrees whose directories are merely missing.
    crate::worktree::pinned::unpin(path);
    Ok(())
}

/// Decision 20's discard and accept: delete every branch under `refs/heads/<prefix>/`
/// (`anthrex/<run>/`, with or without its trailing slash, so `anthrex/r1` never reaches
/// `anthrex/r10/…`). Salvage refs live outside `refs/heads` and are kept. Each delete
/// is `update-ref --no-deref -d <ref> <listed sha>`: a compare-and-swap that removes a
/// symbolic ref itself, never the branch it points at (ruling T9-m2).
///
/// A branch still checked out in any worktree is skipped, and returned: by the time
/// branches are deleted the engine's own worktrees are gone, so a worktree still on
/// one is the user's (they checked the run out to try it), and deleting the branch
/// would leave that checkout on an unborn branch.
pub fn delete_branches(
    git: &OsStr,
    root: &Path,
    prefix: &str,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let prefix = prefix.trim_matches('/');
    if prefix.is_empty() {
        return Err("refusing to delete branches under an empty prefix".to_string());
    }
    let g = Git::new(git, timeout);
    let checked_out = checked_out_branches(g, root)?;
    let pattern = format!("refs/heads/{prefix}/");
    let listing = g.ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(objectname) %(refname)"),
            os(&pattern),
        ],
    )?;
    // The run branch goes last (final fix batch F1, B-I1): until it is gone, a crash
    // part way leaves the run's own record of what accept merged readable.
    let run_branch = format!("refs/heads/{prefix}/integration");
    let mut lines: Vec<&str> = listing.lines().collect();
    lines.sort_by_key(|line| line.ends_with(&format!(" {run_branch}")));
    let mut skipped = Vec::new();
    for line in lines {
        let Some((sha, refname)) = line.split_once(' ') else {
            continue;
        };
        if checked_out.iter().any(|branch| branch == refname) {
            skipped.push(refname.trim_start_matches("refs/heads/").to_string());
            continue;
        }
        g.write(
            root,
            &[
                os("update-ref"),
                os("--no-deref"),
                os("-d"),
                os(refname),
                os(sha),
            ],
        )?;
    }
    Ok(skipped)
}

/// The `refs/heads/…` every worktree of the repository has checked out.
fn checked_out_branches(g: Git<'_>, root: &Path) -> Result<Vec<String>, String> {
    let list = g.ok(
        root,
        &[os("worktree"), os("list"), os("--porcelain"), os("-z")],
    )?;
    Ok(list
        .split('\0')
        .filter_map(|field| field.strip_prefix("branch "))
        .map(str::to_string)
        .collect())
}
