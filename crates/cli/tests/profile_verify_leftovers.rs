//! M8b.10 review: how `profile::verify` ends its checkouts. A leftover of an earlier
//! daemon is salvaged only when it is pinned as ours and is otherwise removed without
//! any git command in it (I1); a directory a command locked cannot stop a removal (I2);
//! salvage names move on when taken and give a clear error when all ten are (I3); a
//! removal that fails after a salvage names the ref (M3), and a leftover's failure says
//! it was the leftover (M4). No agent runs; the commands are plain shell.

mod support;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;

use daemon::profile::verify::{SALVAGE_PREFIX, discard_named};
#[cfg(target_os = "macos")]
use daemon::run::git::task_tmp;

#[cfg(target_os = "macos")]
use support::profile_rig::COMMAND_TIMEOUT;
use support::profile_rig::{GIT_TIMEOUT, Rig, check};
use support::run_harness::{git_in, init_repo};
use support::tempdir;

fn refs(repo: &Path) -> String {
    git_in(repo, &["for-each-ref", "--format=%(refname) %(objectname)"])
}

/// Every file under `dir`, with its length, so a new object or index shows.
fn files(dir: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let entry = entry.unwrap();
            let meta = std::fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), meta.len()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_leftover_without_a_repository_is_removed_without_running_git_in_it() {
    // An enclosing repository stands in for a dotfiles repository at `$HOME`, where the
    // data and worktree directories live by default: git discovery from the leftover
    // would find it.
    let dir = tempdir();
    let root = dir.path().canonicalize().unwrap();
    init_repo(&root, &[("dotfile", "x\n")]);
    std::fs::write(root.join("untracked-dotfile"), "y\n").unwrap();
    let rig = Rig::in_dir(&root);
    // A daemon that died inside `prepare`: the checkout exists, its repository has no
    // `HEAD`, and nothing is pinned.
    std::fs::create_dir_all(rig.checkout()).unwrap();
    std::fs::write(rig.checkout().join("stray.txt"), "stray\n").unwrap();
    std::fs::create_dir_all(rig.checkout_repo().join("git/objects")).unwrap();
    let before = (refs(&root), files(&root.join(".git")));
    let inner_before = refs(&rig.repo);

    let verified = rig.verify(check("test ! -e stray.txt"));

    assert!(
        verified.verification.check.as_ref().is_some_and(|c| c.ok),
        "{verified:?}"
    );
    assert!(verified.salvaged.is_empty(), "{verified:?}");
    assert_eq!(
        (refs(&root), files(&root.join(".git"))),
        before,
        "the enclosing repository changed"
    );
    assert_eq!(refs(&rig.repo), inner_before);
    assert!(!rig.checkout().exists());
    assert!(!rig.checkout_repo().exists());
}

#[test]
fn a_leftover_checkout_is_pinned_salvaged_and_replaced_by_a_fresh_one() {
    let rig = Rig::new();
    let path = rig.prepared();
    std::fs::write(path.join("leftover.txt"), "work\n").unwrap();
    // As after a daemon restart: the checkout and its repository are there, the
    // in-memory pin is not.
    daemon::worktree::pinned::unpin(&path);

    let verified = rig.verify(check("test ! -e leftover.txt"));

    assert!(
        verified.verification.check.as_ref().is_some_and(|c| c.ok),
        "the verification did not start fresh: {verified:?}"
    );
    assert_eq!(verified.salvaged.len(), 1, "{verified:?}");
    let reference = &verified.salvaged[0];
    assert_eq!(
        git_in(&rig.repo, &["show", &format!("{reference}:leftover.txt")]),
        "work"
    );
    assert!(!rig.checkout().exists());
}

#[test]
fn a_command_that_locks_a_directory_cannot_block_removal() {
    let rig = Rig::new();
    // In the checkout and in its repository's object store, both of which a confined
    // command may write.
    let verified = rig.verify(check(
        "mkdir -p d/e && touch d/e/f && chmod 555 d/e && chmod 500 d && \
         G=$(sed -n 's/^gitdir: //p' .git) && mkdir \"$G/objects/x\" && chmod 555 \"$G/objects/x\"",
    ));
    assert!(
        verified.verification.check.as_ref().is_some_and(|c| c.ok),
        "{verified:?}"
    );
    assert_eq!(verified.salvaged.len(), 1, "{verified:?}");
    assert!(!rig.checkout().exists());
    assert!(!rig.checkout_repo().exists());
    // The next verification is not blocked, and salvages nothing again.
    let again = rig.verify(check("true"));
    assert!(again.salvaged.is_empty(), "{again:?}");
    assert_eq!(
        git_in(
            &rig.repo,
            &["for-each-ref", "--format=%(refname)", SALVAGE_PREFIX]
        )
        .lines()
        .count(),
        1
    );
}

#[test]
fn a_taken_salvage_name_moves_to_the_next_and_ten_give_a_clear_error() {
    let rig = Rig::new();
    let git = OsStr::new("git");
    let secs = 1_700_000_000;
    // `<prefix><secs>` already holds other work (the commit, not this checkout's dirt).
    git_in(
        &rig.repo,
        &["update-ref", &format!("{SALVAGE_PREFIX}{secs}"), "HEAD"],
    );
    let path = rig.prepared();
    std::fs::write(path.join("dirt.txt"), "one\n").unwrap();
    let saved = discard_named(
        git,
        &rig.pre.root,
        &path,
        &rig.checkout_repo(),
        secs,
        GIT_TIMEOUT,
    );
    assert_eq!(saved, Ok(Some(format!("{SALVAGE_PREFIX}{secs}-1"))));
    assert!(!path.exists());

    // All ten names taken: a clear error, and the dirty checkout is kept.
    let secs = secs + 1;
    for name in std::iter::once(format!("{SALVAGE_PREFIX}{secs}"))
        .chain((1..=9).map(|n| format!("{SALVAGE_PREFIX}{secs}-{n}")))
    {
        git_in(&rig.repo, &["update-ref", &name, "HEAD"]);
    }
    let path = rig.prepared();
    std::fs::write(path.join("dirt.txt"), "two\n").unwrap();
    let error = discard_named(
        git,
        &rig.pre.root,
        &path,
        &rig.checkout_repo(),
        secs,
        GIT_TIMEOUT,
    )
    .unwrap_err();
    assert_eq!(
        error,
        format!(
            "every salvage name from {SALVAGE_PREFIX}{secs} to {SALVAGE_PREFIX}{secs}-9 \
             already holds other work; the checkout {} is kept",
            path.display()
        )
    );
    assert_eq!(
        std::fs::read_to_string(path.join("dirt.txt")).unwrap(),
        "two\n"
    );
    let saved = discard_named(
        git,
        &rig.pre.root,
        &path,
        &rig.checkout_repo(),
        secs + 1,
        GIT_TIMEOUT,
    );
    assert_eq!(saved, Ok(Some(format!("{SALVAGE_PREFIX}{}", secs + 1))));
}

/// Clears the user-immutable flag under `path` when dropped, so the test directory can
/// always be removed.
#[cfg(target_os = "macos")]
struct Unlock(PathBuf);

#[cfg(target_os = "macos")]
impl Drop for Unlock {
    fn drop(&mut self) {
        if self.0.exists() {
            let _ = Command::new("chflags")
                .args(["-R", "nouchg"])
                .arg(&self.0)
                .status();
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_removal_that_fails_after_a_salvage_names_the_ref() {
    let rig = Rig::new();
    let git = OsStr::new("git");
    let path = rig.prepared();
    let _unlock = Unlock(path.clone());
    std::fs::write(path.join("stuck.txt"), "work\n").unwrap();
    let status = Command::new("chflags")
        .arg("uchg")
        .arg(path.join("stuck.txt"))
        .status()
        .unwrap();
    assert!(status.success());
    let secs = 1_700_000_100;
    let error = discard_named(
        git,
        &rig.pre.root,
        &path,
        &rig.checkout_repo(),
        secs,
        GIT_TIMEOUT,
    )
    .unwrap_err();
    let reference = format!("{SALVAGE_PREFIX}{secs}");
    assert!(
        error.ends_with(&format!("; its work was salvaged to {reference}")),
        "{error}"
    );
    assert_eq!(
        git_in(&rig.repo, &["show", &format!("{reference}:stuck.txt")]),
        "work"
    );
    // The leftover still cannot go: the next verification says it was the leftover.
    let error = rig
        .try_verify(
            check("true"),
            rig.confine(&config::Orchestrator::default()),
            COMMAND_TIMEOUT,
        )
        .unwrap_err();
    assert!(
        error.starts_with(&format!(
            "could not discard the leftover verification checkout {}: ",
            path.display()
        )),
        "{error}"
    );
    // The test's own clean-up: the flag off, then whatever the partial removal left.
    drop(_unlock);
    daemon::worktree::pinned::unpin(&path);
    for dir in [
        path.clone(),
        rig.checkout_repo(),
        task_tmp(&rig.checkout_repo()),
    ] {
        if dir.exists() {
            std::fs::remove_dir_all(&dir).unwrap();
        }
    }
}
