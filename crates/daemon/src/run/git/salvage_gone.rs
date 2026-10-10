//! Milestone 9.11 decision 13 (A3): the salvage of a task checkout the user deleted by
//! hand. The checkout's repository (`<data>/runs/<run>/tasks/<name>`, [`Repo`]) is
//! still there: its `HEAD` names the worker's last commit and its private object
//! directory holds the objects of every commit the engine has not yet imported. Before
//! that repository is removed, those commits are imported into the user's repository
//! (re-hashed on the way in, as every import is) and kept under the salvage ref.
//!
//! A child of [`super`] (the import), whose staging and `index-pack --strict` path it
//! reuses. Blocking; call only from `spawn_blocking`, behind the caller's
//! [`crate::run::git::GitQueue::write`] (it writes objects and a ref).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::super::checkout::Repo;
use super::super::merge::read;
use super::super::worktrees::common_dir;
use super::super::{Git, os};
use super::{HeadFile, has_commit, head_file, import};
use crate::worktree::pinned::Pin;

/// Decision 13: the commits of the deleted checkout whose repository is `repo`, kept
/// under `reference` in the user's repository at `root`; `Some(reference)` when it
/// made the ref (or found it already there at the same commit: the op replayed after
/// a crash), `None` when there is nothing to keep.
///
/// Nothing to keep: the repository has no `HEAD` (it was never made, or is gone), its
/// `HEAD` names no commit (the worker's sandbox lets it write no ref, so no commit of
/// its can be behind a symbolic `HEAD`), or its `HEAD` is already reachable from a ref
/// of the user's repository (the task branch's tip, recorded by the engine's import, or
/// the base it was made from). Otherwise the objects reachable from `HEAD` and from no
/// branch are imported from the private directory, and `reference` is created at
/// `HEAD` by `update-ref` with an empty old value, `message` its reflog message. A
/// `reference` already holding another commit is an error, as [`super::super::salvage`]
/// gives.
pub fn salvage_gone(
    git: &OsStr,
    root: &Path,
    repo: &Path,
    reference: &str,
    message: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let g = Git::new(git, timeout);
    let repo = Repo::at(repo);
    if std::fs::symlink_metadata(repo.git_dir().join("HEAD")).is_err() {
        return Ok(None);
    }
    let common = common_dir(g, root)?;
    let pin = Pin {
        git_dir: repo.git_dir(),
        common_dir: common,
        head: None,
        own: None,
        objects: Some(repo.objects()),
        engine: Some(repo.engine()),
        standalone: true,
        broken: None,
    };
    let HeadFile::Commit(head) = head_file(&pin)? else {
        return Ok(None);
    };
    match read(g, root, reference)? {
        Some(saved) if saved == head => return Ok(Some(reference.to_string())),
        Some(_) => return Err(format!("salvage ref {reference} already holds other work")),
        None => {}
    }
    if has_commit(g, root, &head)? {
        if reachable(g, root, &head)? {
            return Ok(None);
        }
    } else {
        import_head(g, root, &pin, &head)?;
    }
    let args = [
        os("update-ref"),
        os("-m"),
        os(message),
        os("--no-deref"),
        os(reference),
        os(&head),
        os(""),
    ];
    g.write(root, &args)?;
    Ok(Some(reference.to_string()))
}

/// Whether `head` is reachable from any ref of the repository at `root`.
fn reachable(g: Git<'_>, root: &Path, head: &str) -> Result<bool, String> {
    let args = [
        os("rev-list"),
        os("-n"),
        os("1"),
        os(head),
        os("--not"),
        os("--all"),
    ];
    Ok(g.ok(root, &args)?.trim().is_empty())
}

/// The objects reachable from `head` and from no branch of the repository, imported
/// from the checkout's private directory; then the repository must have `head` and
/// everything it reaches.
fn import_head(g: Git<'_>, root: &Path, pin: &Pin, head: &str) -> Result<(), String> {
    let Some(objects) = pin.objects.as_deref() else {
        return Err(format!(
            "the commit {head} has no object directory to import from"
        ));
    };
    let tips = g.ok(
        root,
        &[
            os("for-each-ref"),
            os("--format=%(objectname)"),
            os("refs/heads/"),
        ],
    )?;
    let mut revs = format!("{head}\n");
    if !tips.trim().is_empty() {
        revs.push_str("--not\n");
        for tip in tips.lines().filter(|tip| !tip.is_empty()) {
            revs.push_str(&format!("{tip}\n"));
        }
    }
    import(g, root, pin, objects, &revs, true)?;
    if !has_commit(g, root, head)? {
        return Err(format!(
            "the deleted checkout's commit {head} did not import: its object in the task's \
             object directory does not hash to its name"
        ));
    }
    // The same walk, bounded by the same branches: every object it reaches must now be
    // in the repository.
    let args = [
        os("rev-list"),
        os("--objects"),
        os("--quiet"),
        os("--stdin"),
    ];
    g.read_input(root, &args, revs.as_bytes())
        .map(|_| ())
        .map_err(|error| {
            format!("the deleted checkout's commit {head} imported incompletely: {error}")
        })
}
