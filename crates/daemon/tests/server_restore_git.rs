//! Windows restored from `state.json` and the git registry: does a restored window's
//! checkout get watched again after a daemon restart?
//!
//! Git-surface spec 3.3: "A root is registered when at least one window records it, and
//! unregistered when the last such window is removed" — a rule about what a window
//! records, not about how the window came to exist. Spec 3.5: a `Git` message is "sent
//! once for every known root immediately after the client's initial window list, so a
//! fresh client is never blank".
//!
//! Milestone 6 registered roots from the `CreateWindow` handler alone, so neither held
//! after a restart: the bottom bar stayed blank for every restored window, and — since
//! `Restart` never registered either — stayed blank for the rest of that daemon's life.
//!
//! Every test mirrors the real startup order of `lifecycle::run`: build the manager,
//! `restore` the loaded state into it, *then* `serve`. `crates/daemon/tests/server_git.rs`
//! is the created-window half of the same surface.
//!
//! Every wait below is a deadline derived from the budgets the code runs against (see
//! `registration_deadline` and its siblings near the bottom), not a literal. The
//! literals this file used to have (4 s for the first `Git` message, under
//! `PROBE_TIMEOUT`'s 5 s) failed about one run in ten on a freshly built test binary:
//! the first probe waited for the root's watcher to arm, and on macOS the first FSEvents
//! stream a new executable starts was measured taking 1.5 to 7.8 s. The registry no
//! longer makes the first probe wait (`crates/daemon/tests/git_registry_arming.rs`), and
//! these deadlines now cover what the code is actually allowed to take.

mod support;

use daemon::git::probe::PROBE_TIMEOUT;
use daemon::manager::{ManagerConfig, WindowManager};
use daemon::project::DETECT_TIMEOUT;
use daemon::run::driver::RunService;
use daemon::server::{GitWiring, serve};
use daemon::state::{StateFile, WindowRecord, WorktreeRecord};
use proto::{DaemonMsg, PROTO_VERSION, Runtime, Status, read_frame};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Client, TestDaemon, git};
use tokio_util::sync::CancellationToken;

async fn init_repo() -> (tempfile::TempDir, PathBuf) {
    tokio::task::spawn_blocking(|| {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("tracked.txt"), "one\n").unwrap();
        git(dir.path(), &[OsStr::new("init")]);
        git(dir.path(), &[OsStr::new("add"), OsStr::new("tracked.txt")]);
        git(
            dir.path(),
            &[OsStr::new("commit"), OsStr::new("-m"), OsStr::new("init")],
        );
        let root = dir.path().canonicalize().unwrap();
        (dir, root)
    })
    .await
    .unwrap()
}

/// A record for a window in `cwd`. `managed` is the linked worktree anthrex made, when
/// it made one; `worktree` — the watched root — is derived from it here the way a real
/// managed window's is, and set explicitly by the plain-window test below.
fn record(id: u32, name: &str, cwd: &Path, managed: Option<WorktreeRecord>) -> WindowRecord {
    WindowRecord {
        id,
        name: name.to_string(),
        runtime: Runtime::Shell,
        cwd: cwd.to_path_buf(),
        project: Some(cwd.to_path_buf()),
        worktree: managed.as_ref().map(|m| m.path.clone()),
        managed,
        model: None,
        initial_prompt: None,
        session_id: None,
        created_at: 0,
        status: Status::Exited,
        run: None,
        kind: Default::default(),
    }
}

/// Manager + `restore(state)` + `serve`, in the order `lifecycle::run` does it.
async fn start_daemon_restoring(state: StateFile) -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let worktrees_root = dir.path().canonicalize().unwrap().join("worktrees");
    let mut config = ManagerConfig::new(socket.clone(), "/bin/sh".into());
    config.worktrees_root = worktrees_root.clone();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    manager.restore(state);
    let shutdown = CancellationToken::new();
    let git = GitWiring::new(config::Git::default());
    let runs = RunService::for_manager(&manager, dir.path().join("data"), git.registry.clone());
    runs.spawn(shutdown.clone());
    tokio::spawn(serve(
        listener,
        manager.clone(),
        git,
        runs,
        shutdown.clone(),
    ));
    TestDaemon {
        _dir: dir,
        socket,
        worktrees_root,
        shutdown,
        manager,
    }
}

fn state_with(windows: Vec<WindowRecord>) -> StateFile {
    StateFile {
        version: daemon::state::STATE_VERSION,
        next_id: 9,
        windows,
        runs: Vec::new(),
    }
}

/// A window anthrex itself made the worktree for: the record carries the path, so
/// `WindowInfo.worktree` comes back populated. Is the root watched again?
#[tokio::test]
async fn a_restored_managed_worktree_window_is_watched_again() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon_restoring(state_with(vec![record(
        3,
        "restored",
        &root,
        Some(WorktreeRecord {
            repo_root: root.clone(),
            path: root.clone(),
            branch: "main".into(),
        }),
    )]))
    .await;

    let (mut client, welcome) = Client::connect(&d, PROTO_VERSION).await;
    let DaemonMsg::Welcome { windows, .. } = welcome else {
        panic!("expected Welcome");
    };
    assert_eq!(
        windows[0].worktree.as_deref(),
        Some(root.as_path()),
        "the restored window records its worktree root"
    );

    assert!(
        saw_git_within(&mut client, &root, registration_deadline()).await,
        "no Git message for the restored window's root: it was never registered"
    );
}

/// Positive control for the harness above: a window *created* the ordinary way in the
/// same daemon does get its root registered, so a negative result in the restore tests
/// is about the restore path, not about this test file's plumbing.
#[tokio::test]
async fn control_a_created_window_in_this_harness_is_watched() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon_restoring(state_with(Vec::new())).await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(proto::ClientMsg::CreateWindow {
            spec: proto::WindowSpec {
                cwd: root.clone(),
                ..support::shell_spec("created")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    assert!(
        saw_git_within(&mut client, &root, created_deadline()).await,
        "control failed: even a created window produced no Git message"
    );
}

/// `manager/restore.rs` used to claim, in a comment, that an unwatched restored window
/// "comes back unwatched until it is restarted". It did not: `requests::restart` never
/// took the registry at all, so a restart left the root just as unwatched. That comment
/// is gone; this is the property that replaces it — the root is watched from startup and
/// *stays* watched across a restart, which is why the restart path still registers
/// nothing of its own.
///
/// Asserted by changing the worktree after the restart, not by waiting for a message:
/// the registration's own probe has already published by then, and `Publisher` never
/// republishes an unchanged state. Only the root's own task can publish the change, by
/// its watcher, by the probe it runs once that watcher is armed, or by its safety poll,
/// and an unregistered root has no task at all.
#[tokio::test]
async fn a_restart_keeps_the_restored_root_watched() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon_restoring(state_with(vec![record(
        3,
        "restored",
        &root,
        Some(WorktreeRecord {
            repo_root: root.clone(),
            path: root.clone(),
            branch: "main".into(),
        }),
    )]))
    .await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    assert!(
        saw_git_within(&mut client, &root, registration_deadline()).await,
        "the restored window's root must be watched from startup"
    );

    client
        .send(proto::ClientMsg::Restart { window_id: 3 })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "restart"))
        .await;

    touch_tracked_file(&root).await;
    assert!(
        saw_git_within(&mut client, &root, change_deadline()).await,
        "the watcher must survive the restart"
    );
}

/// The reference counting `register_restored_roots` has to preserve: registering once
/// per *window* rather than once per distinct root. Deduplicating would take one
/// reference for these two windows, and this `Remove` would then stop the watcher while
/// the second window is still looking at the same checkout.
///
/// The mirror of `two_windows_in_one_worktree_register_once` in `server_git.rs`, for
/// restored windows instead of created ones.
#[tokio::test]
async fn two_restored_windows_on_one_root_each_hold_a_reference() {
    let (_repo, root) = init_repo().await;
    let managed = || {
        Some(WorktreeRecord {
            repo_root: root.clone(),
            path: root.clone(),
            branch: "main".into(),
        })
    };
    let d = start_daemon_restoring(state_with(vec![
        record(3, "first", &root, managed()),
        record(4, "second", &root, managed()),
    ]))
    .await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    assert!(saw_git_within(&mut client, &root, registration_deadline()).await);

    client
        .send(proto::ClientMsg::Remove {
            window_id: 3,
            remove_worktree: false,
            force: false,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove"))
        .await;

    touch_tracked_file(&root).await;
    assert!(
        saw_git_within(&mut client, &root, change_deadline()).await,
        "the second window still records this root, so it must still be watched"
    );
}

/// Changes the tracked file so a real `git status` sees a dirty worktree — enough for
/// the watcher to fire and the probe's result to actually differ, since identical states
/// are never published twice.
async fn touch_tracked_file(root: &Path) {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
    })
    .await
    .unwrap();
}

/// Whether a `Git` message for `root` arrives within `within`, draining everything else.
///
/// Reads frames directly rather than through `Client::recv`, whose own 5 s bound would
/// panic inside any deadline longer than that.
async fn saw_git_within(client: &mut Client, root: &Path, within: Duration) -> bool {
    tokio::time::timeout(within, async {
        loop {
            let message = read_frame(&mut client.rd)
                .await
                .unwrap()
                .expect("daemon closed");
            if matches!(message, DaemonMsg::Git { root: r, .. } if r == root) {
                return;
            }
        }
    })
    .await
    .is_ok()
}

/// Scheduling slack on top of the budgets below: `spawn_blocking` pool contention, event
/// delivery and the rest of a real OS. The same margin `git_registry.rs` uses for its
/// real-watcher tests.
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(10);

/// A restored root is registered by `serve` before any client connects, and its first
/// probe starts right away, so its first `Git` message is bounded by that probe alone.
fn registration_deadline() -> Duration {
    PROBE_TIMEOUT + WALL_CLOCK_SLACK
}

/// A created window is registered once `CreateWindow` has resolved its roots, which is
/// a `git` run of its own bounded by [`DETECT_TIMEOUT`]. A shell window has no other
/// timed step before the registration: this harness's launch gate is open from the
/// start.
fn created_deadline() -> Duration {
    DETECT_TIMEOUT + PROBE_TIMEOUT + WALL_CLOCK_SLACK
}

/// A change to an already-published root. With the watcher armed it costs a debounce
/// and a probe, and seldom more than a few hundred milliseconds. But arming the watcher
/// has no bound of its own (see the module docs), and a change made before it arms is
/// only picked up by the probe that follows arming or, failing that, by the safety poll.
/// A probe may already be in flight at that moment, and the new one then waits for it.
/// So the budget is the poll interval `serve` was given plus two probes.
fn change_deadline() -> Duration {
    Duration::from_secs(config::Git::default().poll_secs) + PROBE_TIMEOUT * 2 + WALL_CLOCK_SLACK
}

/// The case the state file could not represent at all until `WindowRecord.worktree`
/// became its own field: a window that merely stood inside a checkout nobody asked
/// anthrex to make. Git-surface spec 3.4 gives it a `WindowInfo.worktree` like any
/// other, so a restart must not lose it — and spec 3.3 then has it watched.
#[tokio::test]
async fn a_restored_plain_window_keeps_its_worktree_and_is_watched() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon_restoring(state_with(vec![WindowRecord {
        worktree: Some(root.clone()),
        kind: Default::default(),
        ..record(4, "plain", &root, None)
    }]))
    .await;

    let (mut client, welcome) = Client::connect(&d, PROTO_VERSION).await;
    let DaemonMsg::Welcome { windows, .. } = welcome else {
        panic!("expected Welcome");
    };
    assert_eq!(
        windows[0].worktree.as_deref(),
        Some(root.as_path()),
        "a restored window inside a checkout must still record its worktree root"
    );
    assert!(
        saw_git_within(&mut client, &root, registration_deadline()).await,
        "and that root must be watched"
    );
}
