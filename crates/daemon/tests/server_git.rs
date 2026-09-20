//! M4.5.6 — wiring the git registry into the server.

mod support;

use proto::{ClientMsg, DaemonMsg, PROTO_VERSION, WindowSpec};
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Client, git, shell_spec, start_daemon, start_daemon_with_git};

/// A repository with one committed tracked file, whose worktree root is also its
/// project root. Blocking setup runs off the runtime worker, same as
/// `created_windows_carry_their_project_root` in `tests/server.rs`.
async fn init_repo() -> (tempfile::TempDir, PathBuf) {
    use std::ffi::OsStr;
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

/// Changes the tracked file's content, so a real `git status` sees a dirty worktree —
/// enough to make the watcher fire and the probe's result actually change (identical
/// states are never published twice, per `git::schedule::Publisher`).
async fn touch_tracked_file(root: &Path) {
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        std::fs::write(root.join("tracked.txt"), "two\n").unwrap();
    })
    .await
    .unwrap();
}

/// Fails if any `DaemonMsg::Git` arrives on `client` within `within`. Other traffic is
/// drained and ignored so it cannot mask the message under test.
async fn assert_no_git_message(client: &mut Client, within: Duration) {
    let result = tokio::time::timeout(within, async {
        loop {
            if matches!(client.recv().await, DaemonMsg::Git { .. }) {
                return;
            }
        }
    })
    .await;
    assert!(result.is_err(), "unexpected Git message arrived");
}

impl Client {
    /// Waits for both `Created` and a `Git` message for `root`, in whatever order they
    /// arrive, and returns the new window's id. Registration races the probe that
    /// follows it, so the two messages have no guaranteed order.
    async fn wait_for_created_and_git(&mut self, root: &Path) -> u32 {
        tokio::time::timeout(Duration::from_secs(8), async {
            let mut window_id = None;
            let mut saw_git = false;
            while window_id.is_none() || !saw_git {
                match self.recv().await {
                    DaemonMsg::Created { window_id: id } => window_id = Some(id),
                    DaemonMsg::Git { root: r, .. } if r == root => saw_git = true,
                    _ => {}
                }
            }
            window_id.unwrap()
        })
        .await
        .expect("timed out waiting for Created and a Git message")
    }
}

#[tokio::test]
async fn a_new_window_registers_its_worktree() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: root.clone(),
                ..shell_spec("git-window")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    let msg = client
        .recv_until(|m| matches!(m, DaemonMsg::Git { root: r, .. } if *r == root))
        .await;
    let DaemonMsg::Git { state, .. } = msg else {
        unreachable!()
    };
    assert!(
        state.is_some(),
        "a repository with a commit must probe to Some"
    );
}

#[tokio::test]
async fn removing_the_last_window_unregisters_the_root() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: root.clone(),
                ..shell_spec("only-window")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    let window_id = client.wait_for_created_and_git(&root).await;

    client
        .send(ClientMsg::Remove {
            window_id,
            remove_worktree: false,
            force: false,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove"))
        .await;

    touch_tracked_file(&root).await;
    assert_no_git_message(&mut client, Duration::from_millis(2500)).await;
}

#[tokio::test]
async fn two_windows_in_one_worktree_register_once() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;

    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: root.clone(),
                ..shell_spec("a")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    let id_a = client.wait_for_created_and_git(&root).await;

    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: root.clone(),
                ..shell_spec("b")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    let DaemonMsg::Created { window_id: id_b } = client
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await
    else {
        unreachable!()
    };

    // The first window of two goes: the root is still referenced by the second, so it
    // must keep publishing.
    client
        .send(ClientMsg::Remove {
            window_id: id_a,
            remove_worktree: false,
            force: false,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove"))
        .await;
    touch_tracked_file(&root).await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Git { root: r, .. } if *r == root))
        .await;

    // The second and last window goes: nothing references the root any more.
    client
        .send(ClientMsg::Remove {
            window_id: id_b,
            remove_worktree: false,
            force: false,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove"))
        .await;
    touch_tracked_file(&root).await;
    assert_no_git_message(&mut client, Duration::from_millis(2500)).await;
}

#[tokio::test]
async fn a_fresh_client_receives_every_known_root_after_welcome() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon().await;
    let (mut a, _) = Client::connect(&d, PROTO_VERSION).await;
    a.send(ClientMsg::CreateWindow {
        spec: WindowSpec {
            cwd: root.clone(),
            ..shell_spec("a")
        },
        cols: 80,
        rows: 24,
    })
    .await;
    a.wait_for_created_and_git(&root).await;

    let (mut b, welcome) = Client::connect(&d, PROTO_VERSION).await;
    assert!(matches!(welcome, DaemonMsg::Welcome { .. }));
    match b.recv().await {
        DaemonMsg::Git { root: r, state } => {
            assert_eq!(r, root);
            assert!(state.is_some());
        }
        other => panic!("expected Git as the first message after Welcome, got {other:?}"),
    }
}

#[tokio::test]
async fn a_window_outside_a_repository_registers_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: dir.path().into(),
                ..shell_spec("no-repo")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await;
    assert_no_git_message(&mut client, Duration::from_millis(1000)).await;
}

#[tokio::test]
async fn git_off_publishes_nothing() {
    let (_repo, root) = init_repo().await;
    let d = start_daemon_with_git(false).await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: root.clone(),
                ..shell_spec("off")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    // `Created` and this `WindowsChanged` race each other (the manager publishes
    // before the reply is sent); waiting for `Created` first could discard the very
    // `WindowsChanged` this test needs, so it is not awaited separately here.
    let DaemonMsg::WindowsChanged { windows } = client
        .recv_until(|m| {
            matches!(m, DaemonMsg::WindowsChanged { windows }
                if windows.iter().any(|w| w.worktree.as_deref() == Some(root.as_path())))
        })
        .await
    else {
        unreachable!()
    };
    assert_eq!(
        windows[0].worktree.as_deref(),
        Some(root.as_path()),
        "worktree must still be populated with ANTHREX_GIT=off"
    );

    touch_tracked_file(&root).await;
    assert_no_git_message(&mut client, Duration::from_millis(1500)).await;
}
