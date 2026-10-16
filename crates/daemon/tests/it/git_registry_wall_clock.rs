//! The real-watcher half of the git registry suite (split from `git_registry.rs`): the
//! tests that wait on wall-clock time, with a deadline, against a real repository.

use std::path::Path;
use std::time::Duration;

use crate::support;

use daemon::git::probe::PROBE_TIMEOUT;
use daemon::git::schedule::DEBOUNCE;
use proto::GitState;
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;

use super::git_registry::{Publication, git_settings};

// ---------------------------------------------------------------------------
// The one test allowed to wait on wall-clock time
// ---------------------------------------------------------------------------

// Slack on top of the real budgets below: `support::WALL_CLOCK_SLACK`, whose history
// (a bare `secs(5)` that equalled `PROBE_TIMEOUT`) is `docs/timing-budgets.md`'s.
use support::WALL_CLOCK_SLACK;

/// The deadline for the registration probe in a real-watcher test: it starts the
/// instant `register` is called, with no debounce ahead of it, so it is bounded only by
/// [`PROBE_TIMEOUT`] itself.
fn registration_probe_deadline() -> Duration {
    PROBE_TIMEOUT + WALL_CLOCK_SLACK
}

/// The deadline for a probe triggered by a real filesystem event: [`DEBOUNCE`] (the
/// wait, from the last accepted event, before a probe is even scheduled) stacked on
/// [`PROBE_TIMEOUT`] (that probe's own hard cap), plus [`WALL_CLOCK_SLACK`] for
/// everything neither constant accounts for. It covers only what follows the write, so
/// a test writes once its watcher has armed (M9.11.37, C10): arming has no bound.
fn watcher_probe_deadline() -> Duration {
    PROBE_TIMEOUT + DEBOUNCE + WALL_CLOCK_SLACK
}

/// A hang guard for the arm's notification: `hold`, then the real arming (1.5–7.8 s
/// measured on a fresh executable, unbounded in principle).
fn arm_wait(hold: Duration) -> Duration {
    hold + Duration::from_secs(60)
}

/// From the arm's notification to the post-arm probe finished: it may wait behind the
/// registration probe still in flight, each under [`PROBE_TIMEOUT`].
fn post_arm_probe_deadline() -> Duration {
    PROBE_TIMEOUT * 2 + WALL_CLOCK_SLACK
}

fn git(dir: &Path, args: &[&str]) {
    let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    support::git(dir, &args);
}

fn repo_with_one_committed_file() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "a.txt"]);
    git(dir.path(), &["commit", "-m", "init"]);
    dir
}

/// Waits on wall-clock time, with a deadline, for a published state that satisfies
/// `wanted`. Publications that do not (the registration probe, an intermediate state)
/// are skipped rather than failing the wait.
async fn wait_for_state(
    rx: &mut mpsc::UnboundedReceiver<Publication>,
    within: Duration,
    what: &str,
    wanted: impl Fn(&GitState) -> bool,
) {
    let deadline = std::time::Instant::now() + within;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let publication = tokio::time::timeout(left, rx.recv())
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
            .expect("the registry dropped the publish channel");
        if publication.1.as_ref().is_some_and(&wanted) {
            return;
        }
    }
}

/// The real `notify` watcher, a real repository and the real probe. This and its
/// linked-worktree sibling below are the only tests in the suite that wait on
/// wall-clock time, and they wait with a deadline.
#[tokio::test]
async fn a_real_write_triggers_a_probe() {
    real_write_triggers_a_probe(Duration::ZERO).await;
}

/// M9.11.37 (C10): the same, the watcher held past [`watcher_probe_deadline`] before
/// it arms; the hold is outside the bound because the write follows the arming.
#[tokio::test]
async fn a_write_after_a_slow_arming_triggers_a_probe() {
    real_write_triggers_a_probe(watcher_probe_deadline() + Duration::from_secs(1)).await;
}

async fn real_write_triggers_a_probe(hold: Duration) {
    if !support::git_is_available() {
        return;
    }
    let repo = repo_with_one_committed_file();
    let root = repo.path().to_path_buf();
    let (tx, mut rx) = mpsc::unbounded_channel::<Publication>();
    let (arm, mut armed) = support::notifying_arm(hold);
    let (registry, finished) = support::registry_with_arm(git_settings(true), tx, arm);
    registry.register(root.clone());

    // The registration probe sees a clean tree.
    let first = tokio::time::timeout(registration_probe_deadline(), rx.recv())
        .await
        .expect("the registration probe must publish")
        .unwrap();
    assert_eq!(first.0, root);
    assert_eq!(first.1.as_ref().map(|s| s.dirty), Some(0));

    support::armed(&mut armed, arm_wait(hold)).await;
    // The probe `run_root` runs once armed must be done too (the registration probe
    // is 1, it is 2): a write it could still read would pass without the watcher.
    support::probes_finished(&finished, 2, post_arm_probe_deadline()).await;
    std::fs::write(root.join("a.txt"), "two\n").unwrap();

    wait_for_state(
        &mut rx,
        watcher_probe_deadline(),
        "a real write to trigger a probe",
        |state| state.dirty == 1,
    )
    .await;
}

/// The same, in a **linked** worktree — which milestone 5 makes the normal case, since
/// every agent gets its own `git worktree add` checkout. A linked worktree's own git
/// dir is `<common>/.git/worktrees/<name>/`, outside the root, which is the only case
/// where `watch::build`'s second, non-recursive `watch()` is added at all; before this
/// test, nothing in the suite built a watcher for one.
///
/// The two phases are not redundant. The write is seen by the recursive watch on the
/// root and would be seen with the git-dir watch missing. The commit is the phase that
/// needs it: `git add` and `git commit` in a linked worktree touch nothing under the
/// worktree root — the index and `COMMIT_EDITMSG` are in its own git dir, the objects
/// and refs are in the common dir — so without that second watch there is no event to
/// debounce, and the tree going clean is not noticed until the 30-second poll.
#[tokio::test]
async fn a_commit_in_a_linked_worktree_triggers_a_probe() {
    if !support::git_is_available() {
        return;
    }
    let repo = repo_with_one_committed_file();
    let parent = tempdir().unwrap();
    let linked = parent.path().join("wt");
    let linked_arg = linked.to_str().expect("a UTF-8 temporary path");
    git(
        repo.path(),
        &["worktree", "add", "-b", "feature", linked_arg],
    );

    let (tx, mut rx) = mpsc::unbounded_channel::<Publication>();
    let (arm, mut armed) = support::notifying_arm(Duration::ZERO);
    let (registry, finished) = support::registry_with_arm(git_settings(true), tx, arm);
    registry.register(linked.clone());

    let first = tokio::time::timeout(registration_probe_deadline(), rx.recv())
        .await
        .expect("the registration probe must publish")
        .unwrap();
    assert_eq!(first.0, linked);
    assert_eq!(first.1.as_ref().map(|s| s.dirty), Some(0));

    support::armed(&mut armed, arm_wait(Duration::ZERO)).await;
    support::probes_finished(&finished, 2, post_arm_probe_deadline()).await;
    std::fs::write(linked.join("a.txt"), "two\n").unwrap();
    wait_for_state(
        &mut rx,
        watcher_probe_deadline(),
        "a write in a linked worktree to trigger a probe",
        |state| state.dirty == 1,
    )
    .await;

    git(&linked, &["add", "a.txt"]);
    git(&linked, &["commit", "-m", "two"]);
    wait_for_state(
        &mut rx,
        watcher_probe_deadline(),
        "a commit in a linked worktree to trigger a probe",
        |state| state.dirty == 0,
    )
    .await;
}
