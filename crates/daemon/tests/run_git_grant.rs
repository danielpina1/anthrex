//! M8a final fix batch F1c (re-review 4, I1): a worker's private directory is inside
//! its own sandbox grant, so a leftover worker process can keep swapping it for a
//! symbolic link. `private_dir` once checked the directory with `lstat` and *then*
//! resolved it with `canonicalize`: a swap between the two made the next session's
//! grant whatever the link named (`~/.claude`, another repository). Now only the
//! engine's own parent is resolved, the leaf's name is appended, and the leaf is
//! checked without following it; the answer is always the engine's own path, or a
//! refusal.

mod support;

use daemon::run::git::private_dir;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How long the race runs: long enough for many thousands of calls, which the old
/// check-then-resolve lost within milliseconds on a loaded machine
/// (`docs/timing-budgets.md`).
const RACE_FOR: Duration = Duration::from_secs(3);

/// A thread standing in for the leftover worker process: it keeps moving the real
/// directory aside, putting a link to `target` in its place, and putting it back.
fn swapper(dir: PathBuf, target: PathBuf, stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let aside = dir.with_file_name("objects.aside");
        while !stop.load(Ordering::Relaxed) {
            if std::fs::rename(&dir, &aside).is_ok() {
                let _ = std::os::unix::fs::symlink(&target, &dir);
                std::hint::spin_loop();
                let _ = std::fs::remove_file(&dir);
                let _ = std::fs::rename(&aside, &dir);
            }
        }
    })
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap()
}

#[test]
fn a_swapped_private_dir_never_widens_the_grant() {
    let data = tempfile::tempdir().unwrap();
    let common = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let task = data.path().join("tasks/t1");
    let objects = task.join("objects");
    let expected = canonical(&private_dir(common.path(), &objects).unwrap());
    assert_eq!(expected, canonical(&task).join("objects"));

    let stop = Arc::new(AtomicBool::new(false));
    let racer = swapper(
        objects.clone(),
        elsewhere.path().to_path_buf(),
        stop.clone(),
    );
    let deadline = Instant::now() + RACE_FOR;
    let mut answered = 0;
    let mut widened = Vec::new();
    while Instant::now() < deadline {
        if let Ok(granted) = private_dir(common.path(), &objects) {
            answered += 1;
            if granted != expected {
                widened.push(granted);
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    racer.join().unwrap();
    assert!(
        widened.is_empty(),
        "{} of {answered} grants named another directory, first {}",
        widened.len(),
        widened[0].display()
    );
    assert!(answered > 0, "no call answered while the racer ran");
}

/// A link or a file at the leaf is refused outright; nothing is created through it.
#[test]
fn a_planted_link_at_the_private_dir_is_refused() {
    let data = tempfile::tempdir().unwrap();
    let common = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let task = data.path().join("tasks/t1");
    std::fs::create_dir_all(&task).unwrap();
    let objects = task.join("objects");
    std::os::unix::fs::symlink(elsewhere.path().join("new"), &objects).unwrap();
    let err = private_dir(common.path(), &objects).unwrap_err();
    assert!(err.contains("tampered"), "{err}");
    assert!(!elsewhere.path().join("new").exists());
    std::fs::remove_file(&objects).unwrap();
    std::fs::write(&objects, "file").unwrap();
    assert!(private_dir(common.path(), &objects).is_err());
}

/// I1's sweep: a link the worker left at a granted file of its git directory (its
/// `ORIG_HEAD`, a lock, a rebase directory) is removed before the next session is
/// granted it, so a sandbox that resolves its grants never grants the link's target.
#[test]
fn links_left_at_granted_paths_are_removed_before_the_next_grant() {
    use daemon::run::git::{prepare_worktree, worker_git_dirs};
    use support::run_git::{T, commit_file, out, real_git, repo, wt_dir};
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let task = wt.join("runs/gr1/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/gr1/t1", &base, &task, T).unwrap();
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ));
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]));
    let elsewhere = tempfile::tempdir().unwrap();
    let names = ["ORIG_HEAD", "index.lock", "rebase-merge"];
    for name in names {
        let _ = std::fs::remove_file(admin.join(name));
        std::os::unix::fs::symlink(elsewhere.path(), admin.join(name)).unwrap();
    }
    let granted = worker_git_dirs(&common, &task, &[]).unwrap();
    for name in names {
        assert!(granted.contains(&admin.join(name)), "{name} not granted");
        assert!(
            std::fs::symlink_metadata(admin.join(name)).is_err(),
            "{name} is still a link"
        );
    }
}
