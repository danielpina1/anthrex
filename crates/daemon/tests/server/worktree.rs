//! M5.6 — the two client requests that can run git, over a real socket: `CreateWindow`
//! with a branch, and `Remove` with `remove_worktree`.
//!
//! A submodule of `tests/server.rs` rather than a test binary of its own, so it keeps that
//! file's `Client` helpers and its `support` harness; the split is AGENTS.md rule 8, along
//! the seam the milestone already has — the parent file is milestone 1's protocol surface,
//! this one is the worktree requests bolted onto it.
//!
//! Three separate things are under test here and only the first is the feature.
//!
//! 1. **The answers.** A worktree create reports the new linked checkout, a create failure
//!    comes back under `request::CREATE`, a refused worktree removal comes back under
//!    `request::REMOVE_DIRTY` and not as an opaque error, and `--force` without
//!    `--worktree` is refused before anything is killed (design decisions 20 and 24).
//! 2. **The connection loop is not blocked** while one of these runs (design decision 25).
//!    `list_is_answered_while_a_create_is_running` is the whole reason these requests live
//!    in their own tasks, and it waits for git to be provably *inside* `worktree add`
//!    before it asks, rather than sleeping and hoping.
//! 3. **The requests are never abandoned.** Design decisions 17 and 25: once a create or a
//!    removal has started it runs to completion even if the client that asked for it is
//!    gone. For a removal this is not tidiness — `remove_with_worktree` unregisters the
//!    git root immediately before it deletes the directory and re-registers it if the
//!    deletion fails, so a future dropped between those two steps leaves a live worktree
//!    that nothing watches and a window that cannot be removed again. The two
//!    `..._after_its_client_disconnects` tests take the connection away at a moment they
//!    have *proved* is mid-operation, which is the only way to tell a detached task from
//!    one that merely happened to finish first.

use super::*;
use daemon::worktree::{branch_dir_name, repo_worktrees_dir};
use std::path::PathBuf;
use std::time::Instant;
use support::TempRepo;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn worktree_spec(name: &str, cwd: &std::path::Path, branch: &str) -> WindowSpec {
    WindowSpec {
        cwd: cwd.to_path_buf(),
        worktree_branch: Some(branch.to_string()),
        ..shell_spec(name)
    }
}

/// Where the daemon must put `branch`'s checkout: the layout of design decisions 6 and 7,
/// spelled out by the test rather than read back from the reply, so a wrong path is a
/// failure here and not a tautology.
fn expected_worktree(d: &TestDaemon, repo: &TempRepo, branch: &str) -> PathBuf {
    repo_worktrees_dir(&d.worktrees_root, &repo.root).join(branch_dir_name(branch))
}

async fn wait_until(what: &str, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

impl Client {
    /// Waits for `Created` *and* for the `WindowsChanged` that lists the new window, in
    /// whatever order the two tasks produce them, and hands back that window.
    ///
    /// The order really is undefined: phase C publishes before `create` returns, so the
    /// list change is queued by `changes_task` while the request task is still on its way
    /// to sending `Created`. Waiting for one and then the other would discard whichever
    /// arrived first and then wait forever for a message that had already been read.
    async fn created_window(&mut self) -> proto::WindowInfo {
        let mut id: Option<u32> = None;
        let mut latest: Vec<proto::WindowInfo> = Vec::new();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                match self.recv().await {
                    DaemonMsg::Created { window_id } => id = Some(window_id),
                    DaemonMsg::WindowsChanged { windows } => latest = windows,
                    DaemonMsg::Error { request, message } => {
                        panic!("the create failed: {request}: {message}")
                    }
                    _ => {}
                }
                if let Some(id) = id
                    && let Some(window) = latest.iter().find(|w| w.id == id)
                {
                    return window.clone();
                }
            }
        })
        .await
        .expect("timed out waiting for the created window")
    }
}

// ---------------------------------------------------------------------------
// Creating
// ---------------------------------------------------------------------------

/// The whole create path end to end over the wire: the daemon resolves the roots, makes
/// the checkout, and publishes a window whose `worktree` is the *new* checkout rather
/// than the repository the user pointed at (design decision 21). That last assertion is
/// the one that matters to the client: `worktree` is the key the git registry watches and
/// the bottom bar reads, so a window carrying the parent repository's root here would show
/// the parent's branch and dirty counts for its whole life.
#[tokio::test]
async fn create_with_a_worktree_over_the_socket() {
    let repo = TempRepo::new();
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("api", &repo.root, "feat/api"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window = c.created_window().await;

    let expected = expected_worktree(&d, &repo, "feat/api");
    assert_eq!(
        window.worktree.as_deref(),
        Some(expected.as_path()),
        "the watched root must be the new checkout, not {:?}",
        repo.root
    );
    assert_eq!(window.cwd, expected, "the agent runs in its own checkout");
    assert_eq!(
        window.branch.as_deref(),
        Some("feat/api"),
        "the branch anthrex asked for travels on the window"
    );
    assert_eq!(
        window.project, repo.root,
        "a linked worktree still groups under its project root"
    );
    assert_eq!(support::head_branch(&expected), "feat/api");
    assert!(repo.worktree_paths().contains(&expected));
}

/// A create that git refuses is still a create: the client is waiting on the
/// `request::CREATE` channel and nothing else, and the reason comes back as git gave it.
#[tokio::test]
async fn create_errors_use_the_create_request() {
    let dir = tempfile::tempdir().unwrap();
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("nope", dir.path(), "feat/nope"),
        cols: 80,
        rows: 24,
    })
    .await;

    let DaemonMsg::Error { request, message } =
        c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(request, "create");
    assert!(
        message.contains("not a git repository"),
        "the reason must survive to the user: {message}"
    );
    assert!(
        d.manager.list().is_empty(),
        "a refused create leaves no window behind"
    );
}

/// Design decision 25: the request runs beside the connection loop, so everything else the
/// client asks for is answered while git is still working.
///
/// The hook marker is the synchronisation. It is written by `post-checkout` before the
/// hook sleeps, so when it appears git is provably *inside* `worktree add` — a fixed sleep
/// would pass whether or not the create had got that far, and would still pass against an
/// implementation that answered the list before it ever started the create.
#[tokio::test]
async fn list_is_answered_while_a_create_is_running() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(3);
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("slow", &repo.root, "feat/slow"),
        cols: 80,
        rows: 24,
    })
    .await;
    wait_until("git to reach the checkout hook", || {
        repo.hook_marker().exists()
    })
    .await;

    let asked = Instant::now();
    c.send(ClientMsg::ListWindows).await;
    let reply = tokio::time::timeout(Duration::from_millis(500), c.recv())
        .await
        .expect("ListWindows was queued behind the create");
    // The *first* message back has to be the list, which is how this asserts "before
    // `Created`" as well as "within 500 ms": a `recv_until` would happily skip past a
    // `Created` that had overtaken it.
    match reply {
        DaemonMsg::WindowsChanged { windows } => assert!(
            windows.is_empty(),
            "the create has not finished, so nothing is listed yet: {windows:?}"
        ),
        other => panic!("expected the list answer before Created, got {other:?}"),
    }
    assert!(asked.elapsed() < Duration::from_millis(500));

    let window = c.created_window().await;
    assert_eq!(window.cwd, expected_worktree(&d, &repo, "feat/slow"));
}

// ---------------------------------------------------------------------------
// Removing
// ---------------------------------------------------------------------------

/// Design decision 24's whole reason for existing: a removal refused for changes is a
/// *question*, not a failure, so it comes back under its own `request` value and the
/// client can offer force-or-keep instead of showing a toast. Then the answer works.
#[tokio::test]
async fn a_dirty_worktree_removal_answers_remove_dirty_then_force_works() {
    let repo = TempRepo::new();
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("dirty", &repo.root, "feat/dirty"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window = c.created_window().await;
    let path = window.cwd.clone();

    // Dirtied from inside the agent, through the socket, the way a real one does it.
    c.send(ClientMsg::Input {
        window_id: window.id,
        bytes: b"touch work-in-progress.txt\n".to_vec(),
    })
    .await;
    wait_until("the agent to write an untracked file", || {
        path.join("work-in-progress.txt").exists()
    })
    .await;

    c.send(ClientMsg::Remove {
        window_id: window.id,
        remove_worktree: true,
        force: false,
    })
    .await;
    let DaemonMsg::Error { request, message } =
        c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(
        request, "remove-dirty",
        "a refusal for changes must be distinguishable from every other removal failure"
    );
    assert!(
        message.contains(&path.display().to_string()),
        "the refusal names the worktree, because the dialog shows it: {message}"
    );
    assert!(
        d.manager.list().iter().any(|w| w.id == window.id),
        "a refusal leaves the window and its agent alone"
    );
    assert!(path.join("work-in-progress.txt").exists());

    c.send(ClientMsg::Remove {
        window_id: window.id,
        remove_worktree: true,
        force: true,
    })
    .await;
    let ack = c
        .recv_until(|m| matches!(m, DaemonMsg::Ack { .. } | DaemonMsg::Error { .. }))
        .await;
    assert_eq!(
        ack,
        DaemonMsg::Ack {
            request: "remove".into()
        },
        "force answers on the ordinary removal channel"
    );
    assert!(
        !d.manager.list().iter().any(|w| w.id == window.id),
        "the window leaves the list"
    );
    assert!(!path.exists(), "and the checkout leaves the disk");
    assert!(
        repo.branch_exists("feat/dirty"),
        "force discards the changes, never the branch"
    );
}

/// Design decision 20. `--force` is the answer to a dirty refusal and means nothing on its
/// own; refusing it here rather than silently ignoring it is what stops `anthrex rm --force`
/// from reading as "remove harder" when the user forgot `--worktree`.
#[tokio::test]
async fn force_without_worktree_is_rejected() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("plain"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window = c.created_window().await;

    c.send(ClientMsg::Remove {
        window_id: window.id,
        remove_worktree: false,
        force: true,
    })
    .await;

    let reply = c
        .recv_until(|m| matches!(m, DaemonMsg::Error { .. } | DaemonMsg::Ack { .. }))
        .await;
    assert_eq!(
        reply,
        DaemonMsg::Error {
            request: "remove".into(),
            message: "--force only applies when removing the worktree".into(),
        }
    );
    assert!(
        d.manager.list().iter().any(|w| w.id == window.id),
        "the window is untouched"
    );
}

// ---------------------------------------------------------------------------
// The requests outlive their connection (design decisions 17 and 25)
// ---------------------------------------------------------------------------

/// A create is never cancelled half-way. Phase B runs on the blocking pool whatever the
/// caller does, so a server that aborted the request task — or awaited it inside anything
/// that can drop a future — would leave a checkout and a branch on disk with no window
/// attached to them and nothing that will ever clean them up.
///
/// The hook marker makes the disconnect provably mid-operation: git is inside
/// `worktree add` when the socket closes, not merely "asked recently".
#[tokio::test]
async fn a_create_runs_to_completion_after_its_client_disconnects() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(2);
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("orphan", &repo.root, "feat/orphan"),
        cols: 80,
        rows: 24,
    })
    .await;
    wait_until("git to reach the checkout hook", || {
        repo.hook_marker().exists()
    })
    .await;
    drop(c);

    wait_until("the create to finish without its client", || {
        d.manager.list().iter().any(|w| w.name == "orphan")
    })
    .await;
    let expected = expected_worktree(&d, &repo, "feat/orphan");
    let window = d
        .manager
        .list()
        .into_iter()
        .find(|w| w.name == "orphan")
        .unwrap();
    assert_eq!(window.cwd, expected);
    assert!(expected.is_dir(), "the checkout was really made");
}

/// The rule this task turns on. `remove_with_worktree` unregisters the git root, deletes
/// the directory, and re-registers the root if the deletion fails; a future dropped
/// between the first two steps skips both the re-`register` and the deletion, leaving a
/// live worktree that nothing watches and a window whose `removing` flag was only cleared
/// because task 5 made it a guard. The server must therefore never abort one of these
/// tasks, and a client going away is the way that happens in practice.
///
/// The disconnect is timed against the daemon's own window table rather than the socket:
/// the agent reaching `Exited` means the removal is past its dirty check and past its
/// kill, so it is inside exactly the window above. The list is read directly because a
/// `watch` channel coalesces — the `Exited` publication can be overtaken by the one that
/// follows it, so the socket is not a reliable place to see that state at all. The
/// `caught_it` assertion is there so the test cannot pass by having finished the whole
/// removal before the first poll, which would exercise nothing.
#[tokio::test]
async fn a_worktree_removal_runs_to_completion_after_its_client_disconnects() {
    let repo = TempRepo::new();
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: worktree_spec("cut-off", &repo.root, "feat/cut-off"),
        cols: 80,
        rows: 24,
    })
    .await;
    let window = c.created_window().await;
    let path = window.cwd.clone();

    c.send(ClientMsg::Remove {
        window_id: window.id,
        remove_worktree: true,
        force: false,
    })
    .await;
    let mut caught_it = false;
    wait_until("the removal to kill the agent", || {
        match d.manager.list().into_iter().find(|w| w.id == window.id) {
            Some(w) => {
                caught_it = w.status == Status::Exited;
                caught_it
            }
            None => true,
        }
    })
    .await;
    assert!(
        caught_it,
        "the whole removal finished before the client was dropped, so this run proved \
         nothing about a removal outliving its connection"
    );
    drop(c);

    wait_until("the removal to finish without its client", || {
        !d.manager.list().iter().any(|w| w.id == window.id)
    })
    .await;
    assert!(!path.exists(), "the checkout was really removed");
    assert_eq!(
        repo.worktree_paths(),
        vec![repo.root.clone()],
        "git no longer lists it either, so `worktree remove` ran rather than a bare rmdir"
    );
    assert!(repo.branch_exists("feat/cut-off"));
}
