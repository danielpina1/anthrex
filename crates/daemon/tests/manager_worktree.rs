//! M5.4 — `WindowManager::create` with a git worktree, against real repositories in
//! temporary directories, the real `git` on the machine and real PTYs.
//!
//! Two things are under test here and they are easy to confuse. The first is *what* a
//! worktree window reports: its `cwd` and its `worktree` root are the new linked
//! checkout, never the directory the user pointed at (design decisions 11 and 21). The
//! second is *where the work happened*: `git worktree add` can take seconds, so every
//! assertion about the manager staying responsive while one is in flight is an assertion
//! about the lock, not about the feature.
//!
//! `crates/daemon/tests/manager.rs` is already at the ~600-line guideline, which is why
//! these live in their own binary, and the second of those two concerns — admission and
//! the lock — lives in the `admission` submodule below for the same reason.

mod support;

#[path = "manager_worktree/admission.rs"]
mod admission;

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::window::WindowEvent;
use daemon::worktree::repo_worktrees_dir;
use proto::{Runtime, WindowSpec};
use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::{TempRepo, head_branch};
use tempfile::TempDir;
use tokio::sync::mpsc;

/// A manager whose events are pumped by a background task, like the daemon does, with a
/// `worktrees_root` of its own so nothing here can touch a real data directory.
///
/// The root is canonicalized up front: `worktree::create` canonicalizes the worktree it
/// made, because that path becomes the git registry's key for the window, and a
/// `TempDir` under `/var` would otherwise spell it two ways.
fn manager() -> (Arc<WindowManager>, TempDir, PathBuf) {
    manager_with_socket("/tmp/unused-m54.sock".into())
}

fn manager_with_socket(socket: PathBuf) -> (Arc<WindowManager>, TempDir, PathBuf) {
    let (m, keep, worktrees_root, mut events) = manager_watching_events_with_socket(socket);
    let pump = m.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    (m, keep, worktrees_root)
}

/// As [`manager`], but the window events are handed back instead of being pumped into the
/// manager. A window that never becomes an entry — a create phase C refuses — is
/// invisible to `list()` and to `child_pid`, so its `Exited` event is the only evidence
/// the daemon has that its child is really gone.
fn manager_watching_events() -> (
    Arc<WindowManager>,
    TempDir,
    PathBuf,
    mpsc::UnboundedReceiver<(u32, WindowEvent)>,
) {
    manager_watching_events_with_socket("/tmp/unused-m54.sock".into())
}

fn manager_watching_events_with_socket(
    socket: PathBuf,
) -> (
    Arc<WindowManager>,
    TempDir,
    PathBuf,
    mpsc::UnboundedReceiver<(u32, WindowEvent)>,
) {
    let keep = tempfile::tempdir().unwrap();
    let worktrees_root = keep.path().canonicalize().unwrap();
    let mut config = ManagerConfig::new(socket, "/bin/sh".to_string());
    config.worktrees_root = worktrees_root.clone();
    let (m, events) = WindowManager::new(config);
    (m, keep, worktrees_root, events)
}

fn spec(name: &str, cwd: &Path) -> WindowSpec {
    WindowSpec {
        name: Some(name.to_string()),
        runtime: Runtime::Shell,
        cwd: cwd.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

fn worktree_spec(name: &str, cwd: &Path, branch: &str) -> WindowSpec {
    WindowSpec {
        worktree_branch: Some(branch.to_string()),
        ..spec(name, cwd)
    }
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Kills every window this test made, so no `/bin/sh` outlives the test binary.
fn drain(m: &WindowManager) {
    for window in m.list() {
        m.remove(window.id).unwrap();
    }
}

/// Design decisions 11 and 37: the child runs in the new checkout, and the branch the
/// window asked for is what `WindowInfo.branch` reports.
#[tokio::test]
async fn a_worktree_window_runs_in_its_worktree() {
    let repo = TempRepo::new();
    let (m, _keep, wt_root) = manager();
    let expected = repo_worktrees_dir(&wt_root, &repo.root).join("feat-wt");

    let info = m
        .create(
            worktree_spec("wt", &repo.root, "feat/wt"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();

    assert_eq!(info.branch.as_deref(), Some("feat/wt"));
    assert_eq!(info.cwd, expected, "the window's cwd is the new worktree");
    assert_eq!(head_branch(&expected), "feat/wt");

    // The PTY's own idea of where it is, which is the one that actually matters to the
    // agent: `spec.cwd` must have been replaced before `launch::plan` saw it.
    m.write_input(info.id, b"pwd\n").unwrap();
    wait_until("pwd naming the worktree", || {
        let (snapshot, _, _) = m.snapshot(info.id).unwrap();
        String::from_utf8_lossy(&snapshot).contains("feat-wt")
    })
    .await;

    let plain = m
        .create(
            spec("plain", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();
    assert_eq!(plain.branch, None, "a window without a branch reports none");
    assert_eq!(plain.cwd, repo.root, "and keeps the directory it was given");

    drain(&m);
}

/// Design decision 21: the git registry watches the *new* checkout. The server resolves
/// `worktree` from `spec.cwd` before `create` runs, so it hands in the main checkout;
/// phase C must overwrite it, or every worktree agent would show the parent repository's
/// branch and dirty counts in the bottom bar.
#[tokio::test]
async fn a_worktree_window_reports_its_own_worktree_root() {
    let repo = TempRepo::new();
    let (m, _keep, wt_root) = manager();
    let expected = repo_worktrees_dir(&wt_root, &repo.root).join("feat-wt");

    let info = m
        .create(
            worktree_spec("wt", &repo.root, "feat/wt"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();

    assert_eq!(
        info.worktree,
        Some(expected),
        "the watched root is the new checkout, not the parent"
    );
    assert_eq!(
        info.project, repo.root,
        "the project root is shared with the main checkout"
    );

    let plain = m
        .create(
            spec("plain", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();
    assert_eq!(
        plain.worktree,
        Some(repo.root.clone()),
        "a window with no worktree of its own keeps the root it was handed"
    );

    drain(&m);
}

/// A linked worktree shares its project root, so the sidebar tree still files the agent
/// under its repository rather than under the data directory.
#[tokio::test]
async fn worktree_windows_group_under_their_repository_project() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();

    let plain = m
        .create(
            spec("plain", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();
    let worktree = m
        .create(
            worktree_spec("wt", &repo.root, "feat/group"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();

    assert_eq!(plain.project, repo.root);
    assert_eq!(worktree.project, plain.project);

    drain(&m);
}

/// Every refusal reaches the client as an error and leaves nothing behind: no window, and
/// no checkout git still knows about beyond the main one.
#[tokio::test]
async fn create_errors_come_back_before_any_window_exists() {
    let repo = TempRepo::new();
    let plain_dir = tempfile::tempdir().unwrap();
    let (m, _keep, _wt_root) = manager();

    let cases: [(&str, WindowSpec, &str); 3] = [
        (
            "a directory outside any repository",
            worktree_spec("not-a-repo", plain_dir.path(), "feat/x"),
            "not a git repository",
        ),
        (
            "a directory that is not there at all",
            worktree_spec("missing", Path::new("/definitely/missing/dir"), "feat/x"),
            "directory does not exist",
        ),
        (
            "a branch the main checkout is standing on",
            worktree_spec("in-use", &repo.root, "main"),
            "is already checked out at",
        ),
    ];

    for (what, spec, expected) in cases {
        let error = m
            .create(spec, repo.root.clone(), Some(repo.root.clone()), 80, 24)
            .await
            .expect_err(what)
            .to_string();

        assert!(error.contains(expected), "{what}: {error}");
        assert!(m.list().is_empty(), "{what} left a window behind");
        assert_eq!(
            repo.worktree_paths(),
            vec![repo.root.clone()],
            "{what} left a checkout behind"
        );
    }
}

/// Design decision 16: a phase B that fails *after* the worktree was made must undo it,
/// and say so, or the user is left with a checkout and a branch nobody will ever remove.
///
/// Failing `Window::spawn` on purpose takes some doing, and the brief's suggestion — a
/// manager whose shell does not exist — does not do it. `Window::spawn` always execs
/// `/bin/sh -c 'exec "$0" "$@"'`, so a missing program is a child exiting 127; a working
/// directory that is not a directory is quietly swapped for `$HOME` by portable-pty; and
/// portable-pty's `pre_exec` closes the descriptor `std` reports exec failures on, so
/// even an argv over `ARG_MAX` comes back as a successful spawn. What is left is a
/// failure `std` raises in the parent before it forks at all: an environment value
/// holding a NUL. Every window's environment carries `ANTHREX_SOCKET`, so a socket path
/// with one in it is a spawn that really does fail. *What* the failure is does not
/// matter here — only that phase B fails with the worktree already made.
#[tokio::test]
async fn a_failed_spawn_removes_the_new_worktree() {
    let repo = TempRepo::new();
    let socket = PathBuf::from(OsString::from_vec(b"/tmp/anthrex-m54\0.sock".to_vec()));
    let (m, _keep, wt_root) = manager_with_socket(socket);

    let error = m
        .create(
            worktree_spec("doomed", &repo.root, "doomed"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .expect_err("a window whose environment cannot be built cannot spawn")
        .to_string();

    assert!(
        error.ends_with("the new worktree was removed"),
        "the error must say the worktree is gone: {error}"
    );
    assert!(m.list().is_empty());
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        !repo.branch_exists("doomed"),
        "the branch this create made moments ago at HEAD goes with the worktree"
    );
    assert!(
        !repo_worktrees_dir(&wt_root, &repo.root)
            .join("doomed")
            .exists(),
        "the directory is gone too"
    );

    // What that suffix promises the user is that the create can simply be retried. A
    // retry that reached git again and failed the same way is the proof: the admission
    // claim on the directory was given back, and the directory really is free. A stale
    // reservation would say "already being created" and a surviving checkout would say
    // "worktree path already exists" — both before git ever ran.
    let retry = m
        .create(
            worktree_spec("doomed-again", &repo.root, "doomed"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .expect_err("the same broken socket fails the same way")
        .to_string();
    assert!(
        retry.ends_with("the new worktree was removed"),
        "a retry must reach git again, not a stale claim: {retry}"
    );
}
