//! M5.5 — `WindowManager::remove_with_worktree`: removing a window together with the
//! checkout this daemon made for it.
//!
//! A submodule of `tests/manager_worktree.rs` rather than a test binary of its own, so it
//! keeps that file's `manager()`, `worktree_spec()`, `wait_until()` and `drain()` helpers.
//! The split is AGENTS.md rule 8, along the seam the milestone already has: the parent and
//! `admission.rs` are about *making* a worktree window, this file is about *unmaking* one.
//!
//! Two things are under test and only one of them is the outcome, so they are two files.
//!
//! **Here: what ends up on disk.** A clean tree goes, a dirty one is refused, `--force`
//! overrides the refusal, a tree the user already deleted themselves is pruned rather than
//! made to need `--force`, and the branch survives every one of those (design decisions
//! 13, 18 and 19).
//!
//! **In [`ordering`]: when each step happens, and what two callers racing for one window
//! do.** That is where an implementation can reach the right end state by a route that
//! destroys work on the way, and it is why [`FakeRoots`] records more than the fact of a
//! call. A root whose worktree has been deleted must not stay watched, and a root whose
//! worktree survived must not stop being watched, so `unregister` has to sit *after* the
//! dirty check and *before* `worktree::remove`, with a `register` undoing it if the
//! removal fails. Asserting only that the counts balance would pass for an implementation
//! that unregistered first and for one that unregistered last; recording whether the
//! directory still existed at the moment of each call is what pins the sequence instead of
//! the arithmetic.
//!
//! The fixtures both halves share stay in this file.

use super::*;
use daemon::manager::{GitRoots, RemoveError};
use proto::Status;
use std::ffi::OsStr;
use std::sync::Mutex;

#[path = "removal/ordering.rs"]
mod ordering;

// ---------------------------------------------------------------------------
// The registry stand-in
// ---------------------------------------------------------------------------

/// One call the manager made, and whether the worktree directory was still on disk when
/// it made it. The flag is the whole point: see the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Register { root: PathBuf, existed: bool },
    Unregister { root: PathBuf, existed: bool },
}

/// A side effect a test wants to happen *inside* an `unregister` call.
///
/// This is the seam every test in [`ordering`] hangs off, and the only way to reach the
/// instant between the kill and `worktree::remove` without a sleep and a prayer — the
/// window risk 8 lives in, and the one a racing plain `remove` or a still-breathing agent
/// would collide in. The manager calls `unregister` there, and it calls it on test-owned
/// code, so those things can be made to happen exactly then rather than on a race the
/// test would have to win.
type UnregisterHook = Box<dyn Fn(&Path) + Send + Sync>;

/// The two methods `remove_with_worktree` is allowed to know about, and nothing else
/// (design decision 22). That this compiles against `&dyn GitRoots` is half the test: a
/// manager that needed a `GitState`, the registry type or the knowledge that a watcher
/// exists could not be driven by this.
struct FakeRoots {
    log: Mutex<Vec<Call>>,
    /// Run at the end of every `unregister`, after the call has been logged.
    on_unregister: Option<UnregisterHook>,
}

impl FakeRoots {
    fn new() -> Self {
        Self {
            log: Mutex::new(Vec::new()),
            on_unregister: None,
        }
    }

    fn with_hook(hook: impl Fn(&Path) + Send + Sync + 'static) -> Self {
        Self {
            log: Mutex::new(Vec::new()),
            on_unregister: Some(Box::new(hook)),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.log.lock().unwrap().clone()
    }

    /// The log a successful removal must leave: one `unregister`, taken while the
    /// directory was still there — before `worktree::remove`, not after it.
    fn assert_one_unregister_before_the_removal(&self, root: &Path) {
        assert_eq!(
            self.calls(),
            vec![Call::Unregister {
                root: root.to_path_buf(),
                existed: true,
            }],
            "the root must be unregistered exactly once, while its worktree was still \
             on disk"
        );
    }
}

impl GitRoots for FakeRoots {
    fn register(&self, root: PathBuf) {
        let existed = root.exists();
        self.log
            .lock()
            .unwrap()
            .push(Call::Register { root, existed });
    }

    fn unregister(&self, root: &Path) {
        let existed = root.exists();
        self.log.lock().unwrap().push(Call::Unregister {
            root: root.to_path_buf(),
            existed,
        });
        if let Some(hook) = &self.on_unregister {
            hook(root);
        }
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A worktree Shell window that has reached Working, with the checkout it runs in.
struct Agent {
    id: u32,
    path: PathBuf,
}

async fn worktree_agent(m: &WindowManager, repo: &TempRepo, name: &str, branch: &str) -> Agent {
    let info = m
        .create(
            worktree_spec(name, &repo.root, branch),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .expect("the worktree window was created");
    let id = info.id;
    assert_eq!(
        info.worktree.as_deref(),
        Some(info.cwd.as_path()),
        "the watched root is the window's own checkout"
    );
    wait_until("the agent to reach Working", || {
        status_of(m, id) == Some(Status::Working)
    })
    .await;
    Agent { id, path: info.cwd }
}

fn status_of(m: &WindowManager, id: u32) -> Option<Status> {
    m.list().into_iter().find(|w| w.id == id).map(|w| w.status)
}

fn listed(m: &WindowManager, id: u32) -> bool {
    m.list().iter().any(|w| w.id == id)
}

/// Whether any process in `pid`'s group is still there.
///
/// Signal 0 performs the existence and permission check without delivering anything, so
/// this is a question and never an action. `ESRCH` — the process group is gone — is the
/// only answer that means the agent is dead; a reaped child leaves no zombie behind
/// because the window's own waiter collects it before the `Exited` event is sent.
fn group_alive(pid: u32) -> bool {
    // SAFETY: `pid` came from `child_pid`, and signal 0 delivers nothing.
    unsafe { libc::killpg(pid as libc::pid_t, 0) == 0 }
}

/// Makes the agent's own checkout dirty from inside the agent, the way a real one does:
/// the manager must see work it did not put there.
async fn dirty_the_tree(m: &WindowManager, agent: &Agent, file: &str) {
    m.write_input(agent.id, format!("touch {file}\n").as_bytes())
        .unwrap();
    let marker = agent.path.join(file);
    wait_until("the agent to write an untracked file", || marker.exists()).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The ordinary case, and the one that says what "removed" means: the window is gone, the
/// checkout is gone, the branch is not (design decision 13), and the watcher was told
/// before the directory went rather than after it (design decision 22).
#[tokio::test]
async fn remove_with_worktree_removes_a_clean_tree_and_keeps_the_branch() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();
    let agent = worktree_agent(&m, &repo, "clean", "feat/clean").await;

    m.remove_with_worktree(agent.id, false, &roots)
        .await
        .expect("a clean worktree is removed");

    assert!(!listed(&m, agent.id), "the window is gone");
    assert!(!agent.path.exists(), "the checkout is gone");
    assert_eq!(
        repo.worktree_paths(),
        vec![repo.root.clone()],
        "git no longer lists the checkout"
    );
    assert!(
        repo.branch_exists("feat/clean"),
        "removal never deletes the branch"
    );
    roots.assert_one_unregister_before_the_removal(&agent.path);
}

/// The refusal, which is the half of this feature that protects work. The tree has
/// changes, so nothing is removed, *nothing is killed*, and the root stays watched — the
/// agent is still working in that checkout and a watcher that stopped would leave its
/// bottom bar frozen for as long as it kept running.
#[tokio::test]
async fn a_dirty_tree_is_refused_and_the_agent_keeps_running() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();
    let agent = worktree_agent(&m, &repo, "dirty", "feat/dirty").await;
    dirty_the_tree(&m, &agent, "dirty.txt").await;

    let error = m
        .remove_with_worktree(agent.id, false, &roots)
        .await
        .expect_err("a worktree with changes is not removed");

    assert!(
        matches!(error, RemoveError::Dirty(_)),
        "the refusal must be the one the client turns into a force prompt: {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains(&agent.path.display().to_string()),
        "the refusal must name the worktree: {message}"
    );
    assert!(
        message.contains("uncommitted or untracked"),
        "the refusal must say why: {message}"
    );

    assert!(agent.path.join("dirty.txt").exists(), "the work is intact");
    assert_eq!(
        repo.worktree_paths().len(),
        2,
        "the checkout is still there"
    );
    assert!(
        roots.calls().is_empty(),
        "a refusal must not unwatch a worktree the user is still working in: {:?}",
        roots.calls()
    );

    // The agent was never signalled, which is the difference between a refusal and a
    // half-done removal: it is still listed, still not Exited, and still takes input.
    assert!(listed(&m, agent.id));
    assert_ne!(status_of(&m, agent.id), Some(Status::Exited));
    m.write_input(agent.id, b"echo still-here\n").unwrap();
    wait_until("the agent to answer after the refusal", || {
        let (snapshot, _, _) = m.snapshot(agent.id).unwrap();
        String::from_utf8_lossy(&snapshot).contains("still-here")
    })
    .await;

    drain(&m);
}

/// `--force` is the user's answer to the refusal, and it answers only that: the changes go
/// with the checkout, the branch still does not.
#[tokio::test]
async fn force_removes_a_dirty_tree() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();
    let agent = worktree_agent(&m, &repo, "forced", "feat/forced").await;
    dirty_the_tree(&m, &agent, "dirty.txt").await;

    m.remove_with_worktree(agent.id, true, &roots)
        .await
        .expect("force removes a worktree with changes");

    assert!(!listed(&m, agent.id));
    assert!(!agent.path.exists());
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        repo.branch_exists("feat/forced"),
        "force discards the changes, never the branch"
    );
    roots.assert_one_unregister_before_the_removal(&agent.path);
}

/// Design decision 18: removal is offered, never automatic. The plain path — `C-b x`,
/// `anthrex kill`, and a remove confirm with the box unticked — leaves the checkout alone.
#[tokio::test]
async fn plain_remove_keeps_the_worktree() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let agent = worktree_agent(&m, &repo, "kept", "feat/kept").await;

    m.remove(agent.id).unwrap();

    assert!(!listed(&m, agent.id), "the window is gone");
    assert!(agent.path.is_dir(), "the checkout is not");
    assert!(repo.worktree_paths().contains(&agent.path));
    assert!(repo.branch_exists("feat/kept"));
}

/// The user tidied up themselves — `git worktree remove --force` from the main checkout —
/// and only later closed the window with the box ticked.
///
/// Design decision 13 has a branch for exactly this: a path that is gone is pruned and the
/// removal succeeds. Asking `is_dirty` first makes that branch unreachable, because
/// `git -C <missing>` exits 128 and the user gets a raw git error about a directory that
/// does not exist — one they cannot act on, with `--force` as their only route to the
/// behaviour that was designed for them. Worse, the window stays listed and the registry
/// goes on probing a path that is not there, which is the §3.8 staleness design decision
/// 22 exists to prevent: the `unregister` below is the point of the test as much as the
/// success is.
#[tokio::test]
async fn a_worktree_removed_by_hand_is_pruned_without_force() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();
    let agent = worktree_agent(&m, &repo, "tidied", "feat/tidied").await;

    repo.git(&[
        OsStr::new("worktree"),
        OsStr::new("remove"),
        OsStr::new("--force"),
        agent.path.as_os_str(),
    ]);
    assert!(!agent.path.exists(), "the user's own removal took it");

    m.remove_with_worktree(agent.id, false, &roots)
        .await
        .expect("a worktree that is already gone is not a reason to refuse");

    assert!(!listed(&m, agent.id));
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        repo.branch_exists("feat/tidied"),
        "removal never deletes the branch"
    );
    assert_eq!(
        roots.calls(),
        vec![Call::Unregister {
            root: agent.path.clone(),
            existed: false,
        }],
        "a root whose directory is already gone must still stop being watched"
    );
}
