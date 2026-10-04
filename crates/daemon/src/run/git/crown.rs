//! Milestone 9.5 decisions 21 and 22 (rulings RR-1, T1-2): a race's git half. The
//! crown of a lane is one compare-and-swap creating the task branch at the lane's head,
//! which its gates already imported; there is no checkout step (a lane checkout is
//! detached and has no refs). And the stale locks a stopped racer can leave in its
//! checkout's own git directory, cleared before the lane is salvaged.
//!
//! Blocking; call only from `spawn_blocking`, the crown behind the caller's
//! [`super::GitQueue::write`]. Every git call goes through [`Git`]: `--no-optional-locks`,
//! a scrubbed environment and a deadline per command; the write carries decision 18's
//! flags (AGENTS.md rules 10 and 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::{read, short};
use super::{Git, failure, is_id, os, refs_tx};
use crate::run::engine::OpResult;

/// The only locks a worker's git leaves in a checkout's own git directory: a commit's
/// index and `HEAD` updates. A worker writes no ref in the user's repository, so there
/// is no ref lock to clear there (ruling RR-1).
const STALE_LOCKS: [&str; 2] = ["index.lock", "HEAD.lock"];

/// The repository's null object id: `0` repeated for its object format
/// (`git rev-parse --show-object-format`), 40 digits for sha1 and 64 for sha256.
pub fn zero_oid(git: &OsStr, root: &Path, timeout: Duration) -> Result<String, String> {
    zero_in(Git::new(git, timeout), root)
}

fn zero_in(g: Git<'_>, root: &Path) -> Result<String, String> {
    let format = g.ok(root, &[os("rev-parse"), os("--show-object-format")])?;
    match format.trim() {
        "sha1" => Ok("0".repeat(40)),
        "sha256" => Ok("0".repeat(64)),
        other => Err(format!(
            "unknown object format {other:?} in {}",
            root.display()
        )),
    }
}

/// Decision 21's crown: `update-ref --no-deref refs/heads/<task_branch> <lane_head>
/// <zero oid>`, a create that fails when the branch exists. `update-ref` fails alike
/// whether the existing branch is at `lane_head` or elsewhere (M9.5.1 item 8), so a
/// failed create reads the branch (ruling T1-2): at `lane_head`, a plain ref, it is
/// `Crowned` (the crown replayed); anywhere else, or symbolic, `RefMoved`; absent, the
/// failure is the error.
pub fn crown(
    git: &OsStr,
    root: &Path,
    task_branch: &str,
    lane_head: &str,
    timeout: Duration,
) -> Result<OpResult, String> {
    if !is_id(lane_head) {
        return Err(format!("{lane_head:?} is not a commit id"));
    }
    let g = Git::new(git, timeout);
    let refname = format!("refs/heads/{task_branch}");
    let zero = zero_in(g, root)?;
    let args = [
        os("update-ref"),
        os("--no-deref"),
        os(&refname),
        os(lane_head),
        os(&zero),
    ];
    let output = g.write_raw(root, &args)?;
    let crowned = OpResult::Crowned {
        head: lane_head.to_string(),
    };
    if output.success {
        return Ok(crowned);
    }
    let moved = |reason: String| Ok(OpResult::RefMoved { reason });
    match read(g, root, &refname)? {
        None => Err(failure(&args, &output)),
        Some(head) if head != lane_head => moved(format!(
            "{refname} exists at {}, not at the crowned lane's head {}",
            short(&head),
            short(lane_head)
        )),
        Some(_) => match refs_tx::symbolic_in(g, root, &refname)? {
            Some(target) => moved(format!("{refname} is a symbolic ref to {target}")),
            None => Ok(crowned),
        },
    }
}

/// Decision 22: removes `index.lock` and `HEAD.lock` from a stopped lane's own git
/// directory (`<data>/runs/<run>/tasks/<task>.<lane>/git`), and nothing else, anywhere
/// else; the names removed, in that order. A lock that is a symbolic link is removed
/// itself, never followed; a directory under a lock's name is left. A git directory that
/// is itself a link is refused: the engine made it a directory.
pub fn clear_stale_locks(checkout_git_dir: &Path) -> std::io::Result<Vec<String>> {
    let meta = std::fs::symlink_metadata(checkout_git_dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::other(format!(
            "{} is not a directory",
            checkout_git_dir.display()
        )));
    }
    let mut removed = Vec::new();
    for name in STALE_LOCKS {
        let lock = checkout_git_dir.join(name);
        match std::fs::symlink_metadata(&lock) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => {
                std::fs::remove_file(&lock)?;
                removed.push(name.to_string());
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(removed)
}
