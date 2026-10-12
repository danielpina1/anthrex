//! Milestone 9.11 fix round 1, I1: a task checkout made by anthrex 0.1.x has the
//! config text of that release, without the identity include (decision 15). The pin
//! check accepts that exact text too, so after an upgrade the checkout is still
//! handed back, salvaged, discarded and cleaned without a retry; the next
//! `PrepareWorktree` rewrites it to the current text.

use crate::support;

use daemon::run::git::{hand_back, prepare_worktree, remove_worktree, salvage, sync};
use daemon::worktree::pinned;
use std::path::{Path, PathBuf};
use support::run_git::{T, commit_file, head, out, real_git, repo, write, wt_dir};

const IDENTITY_LINE: &str = "\tpath = ../engine/identity\n";

/// Rewrites the checkout's config, in place (one link, as the engine keeps it), to
/// the 0.1.x text: the current text without the identity include, which is the only
/// line the release added.
fn make_legacy(task: &Path) -> PathBuf {
    let admin = PathBuf::from(out(task, &["rev-parse", "--absolute-git-dir"]));
    let config = admin.join("config");
    let text = std::fs::read_to_string(&config).unwrap();
    assert_eq!(text.matches(IDENTITY_LINE).count(), 1, "{text}");
    std::fs::write(&config, text.replace(IDENTITY_LINE, "")).unwrap();
    config
}

#[test]
fn a_checkout_with_the_0_1_config_still_works_and_is_migrated() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/lg01/t1");
    let branch = "anthrex/lg01/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    let config = make_legacy(&task);
    let pin = pinned::pinned(&task).unwrap();
    pinned::check(&task, &pin).unwrap();

    // Handed back: the worker's commit merged with the run's.
    let claimed = commit_file(&task, "b.txt", "task\n", "task work");
    out(&repo.root, &["branch", "anthrex/lg01/integration", &base]);
    out(&repo.root, &["checkout", "-q", "anthrex/lg01/integration"]);
    let run_head = commit_file(&repo.root, "a.txt", "run\n", "run work");
    out(&repo.root, &["checkout", "-q", "-"]);
    let back = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    assert_eq!(sync(real_git(), &task, T).unwrap(), back.head);
    assert_ne!(back.head, claimed);

    // Salvaged.
    write(&task, "dirty.txt", "unsaved\n");
    let reference = "refs/anthrex/salvage/lg01/t1/1";
    let saved = salvage(real_git(), &task, reference, "m", false, T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));

    // The next prepare rewrites the config to the current text.
    prepare_worktree(real_git(), &repo.root, branch, &base, &task, T).unwrap();
    assert!(
        std::fs::read_to_string(&config)
            .unwrap()
            .contains(IDENTITY_LINE)
    );
    make_legacy(&task);

    // Discarded and cleaned: the checkout and its repository are removed.
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();
    assert!(!task.exists());
}

#[test]
fn a_config_one_line_short_of_either_text_is_still_refused() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let task = wt.join("runs/lg02/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/lg02/t1", &base, &task, T).unwrap();
    let config = make_legacy(&task);
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, text.replace("\tlogAllRefUpdates = false\n", "")).unwrap();
    let pin = pinned::pinned(&task).unwrap();
    let refused = pinned::check(&task, &pin).unwrap_err();
    assert!(
        refused.contains("not the one the engine wrote"),
        "{refused}"
    );
}
