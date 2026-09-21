//! M5.5 — `WindowManager::remove_with_worktree`: removing a window together with the
//! checkout this daemon made for it.
//!
//! A submodule of `tests/manager_worktree.rs` rather than a test binary of its own, so it
//! keeps that file's `manager()`, `worktree_spec()`, `wait_until()` and `drain()` helpers.
//! The split is AGENTS.md rule 8, along the seam the milestone already has: the parent and
//! `admission.rs` are about *making* a worktree window, this file is about *unmaking* one.
//!
//! Two things are under test and only one of them is the outcome.
//!
//! The first is what ends up on disk: a clean tree goes, a dirty one is refused, `--force`
//! overrides the refusal, and the branch survives every one of those (design decisions 13,
//! 18 and 19).
//!
//! The second is the **order** in which it happens, which is design decision 22 and the
//! reason [`FakeRoots`] records more than the fact of a call. A root whose worktree has
//! been deleted must not stay watched, and a root whose worktree survived must not stop
//! being watched, so `unregister` has to sit *after* the dirty check and *before*
//! `worktree::remove`, with a `register` undoing it if the removal fails. Asserting only
//! that the counts balance would pass for an implementation that unregistered first and
//! for one that unregistered last; recording whether the directory still existed at the
//! moment of each call is what pins the sequence instead of the arithmetic.

use super::*;
use daemon::manager::{GitRoots, RemoveError};
use proto::Status;
use std::sync::Mutex;

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
/// This is the seam `a_failed_removal_re_registers_the_root` needs and the only way to
/// reach risk 8 — a file appearing after the dirty check said the tree was clean — without
/// a sleep and a prayer. The manager calls `unregister` in the instant between the kill
/// and `worktree::remove`, which is exactly the window the risk describes, and it calls it
/// on test-owned code, so the file can appear there deterministically rather than on a
/// race the test would have to win.
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

/// Risk 8: the dirty check said clean, the agent was killed, and by the time git was
/// asked to remove the checkout there was a file in it. The removal fails, so the
/// directory is still on disk — and a root whose worktree survived must not be left
/// unwatched. The `register` that undoes the `unregister` is the whole requirement.
///
/// The file is planted from [`FakeRoots`]' `unregister` hook, which the manager calls in
/// the instant between the kill and `worktree::remove`. That is the risk's own window, it
/// needs no sleep to hit, and it fails loudly in the one direction that matters: an
/// implementation that unregistered *after* `worktree::remove` would have deleted the
/// checkout before the file was ever written, and this test would see a removal that
/// succeeded.
#[tokio::test]
async fn a_failed_removal_re_registers_the_root() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let agent = worktree_agent(&m, &repo, "racy", "feat/racy").await;
    let planted = agent.path.join("appeared.txt");
    let roots = FakeRoots::with_hook(|root| {
        std::fs::write(root.join("appeared.txt"), "work that arrived late\n").unwrap();
    });

    let error = m
        .remove_with_worktree(agent.id, false, &roots)
        .await
        .expect_err("a removal that git refuses is not a removal");

    assert!(
        error.to_string().contains("uncommitted or untracked"),
        "{error}"
    );
    assert!(planted.exists(), "the late file is still there");
    assert!(agent.path.exists(), "so is the checkout");
    assert_eq!(repo.worktree_paths().len(), 2);
    assert_eq!(
        roots.calls(),
        vec![
            Call::Unregister {
                root: agent.path.clone(),
                existed: true,
            },
            Call::Register {
                root: agent.path.clone(),
                existed: true,
            },
        ],
        "a failed removal must put the root back, and both calls happen while the \
         directory is still on disk"
    );

    // The window stays listed and Exited, which is what "the user can retry" means.
    assert!(listed(&m, agent.id));
    wait_until("the killed agent to be reaped", || {
        status_of(&m, agent.id) == Some(Status::Exited)
    })
    .await;

    // And the retry really works: `removing` was cleared, not left set by the failure.
    std::fs::remove_file(&planted).unwrap();
    let retry = FakeRoots::new();
    m.remove_with_worktree(agent.id, false, &retry)
        .await
        .expect("the same removal succeeds once the file is gone");
    assert!(!listed(&m, agent.id));
    assert!(!agent.path.exists());
    retry.assert_one_unregister_before_the_removal(&agent.path);
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

/// The two refusals that happen before anything is killed, and the one thing neither of
/// them may do: unregister a root it is not removing.
///
/// The double removal is design decision 19 step 1's `removing` flag. Two callers — two
/// clients, or one client and an impatient retry — must not both reach `worktree::remove`,
/// and the loser must not decrement a reference count it never incremented, because the
/// count belongs to the registry and one removal is one `unregister` (design decision 23).
#[tokio::test]
async fn remove_with_worktree_rejects_windows_without_one_and_double_removal() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();

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
    let error = m
        .remove_with_worktree(plain.id, false, &roots)
        .await
        .expect_err("a window without a worktree has none to remove")
        .to_string();
    assert!(error.contains("has no worktree"), "{error}");
    assert!(
        error.contains("plain"),
        "the refusal names the window: {error}"
    );
    assert!(listed(&m, plain.id), "and the window is left alone");
    assert!(roots.calls().is_empty(), "{:?}", roots.calls());

    let agent = worktree_agent(&m, &repo, "once", "feat/once").await;
    let (first, second) = tokio::join!(
        m.remove_with_worktree(agent.id, false, &roots),
        m.remove_with_worktree(agent.id, false, &roots),
    );

    let refusal = match (first, second) {
        (Ok(()), Err(refusal)) | (Err(refusal), Ok(())) => refusal.to_string(),
        (first, second) => panic!("exactly one removal may win: {first:?} / {second:?}"),
    };
    assert!(
        refusal.contains("already being removed") || refusal.contains("no window with id"),
        "{refusal}"
    );
    assert!(!listed(&m, agent.id));
    assert!(!agent.path.exists());
    roots.assert_one_unregister_before_the_removal(&agent.path);

    drain(&m);
}

/// The other half of the double-removal rule, and the one that costs something when it is
/// missing: a *plain* [`WindowManager::remove`] landing while a worktree removal is in
/// flight.
///
/// The plain path forgets the entry and its caller — the server — then drops one reference
/// to the window's git root. The worktree path drops one of its own, between its kill and
/// its deletion. Design decision 23 is that one removed window is exactly one
/// `unregister`, so both running would decrement a single registration twice: invisible at
/// a count of one, and with two windows on a root it stops the survivor's watch and leaves
/// its git segment stale with nothing on screen to explain why. The refusal is what stops
/// the second decrement, because the server only unregisters when `remove` succeeded.
///
/// Timed from [`FakeRoots`]' `unregister` hook, which the manager calls in the instant
/// between the kill and `worktree::remove` — the exact window the two paths would collide
/// in, reached deterministically rather than on a race the test has to win.
#[tokio::test]
async fn a_plain_remove_is_refused_while_a_worktree_removal_is_in_flight() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let agent = worktree_agent(&m, &repo, "contended", "feat/contended").await;

    let raced: Arc<Mutex<Option<Result<(), String>>>> = Arc::new(Mutex::new(None));
    let roots = {
        let racer = m.clone();
        let raced = raced.clone();
        let id = agent.id;
        FakeRoots::with_hook(move |_| {
            *raced.lock().unwrap() = Some(racer.remove(id).map_err(|e| e.to_string()));
        })
    };

    m.remove_with_worktree(agent.id, false, &roots)
        .await
        .expect("the worktree removal owns the window and finishes");

    let message = raced
        .lock()
        .unwrap()
        .clone()
        .expect("the racing plain remove ran inside the hook")
        .expect_err("a window already being removed is not removed a second time");
    assert!(message.contains("already being removed"), "{message}");
    assert!(
        message.contains("contended"),
        "the refusal names the window: {message}"
    );

    assert!(!listed(&m, agent.id), "the worktree removal still finished");
    assert!(!agent.path.exists());
    // Exactly one `unregister`, from the path that owns it.
    roots.assert_one_unregister_before_the_removal(&agent.path);
}

/// Design decision 19 step 3 waits for the child only when there is one. A window that has
/// already exited must not spend `KILL_GRACE` waiting for an exit that happened minutes
/// ago — in the TUI that is three seconds of a dialog that looks hung.
#[tokio::test]
async fn an_exited_window_is_removed_without_waiting() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let roots = FakeRoots::new();
    let agent = worktree_agent(&m, &repo, "done", "feat/done").await;

    m.write_input(agent.id, b"exit\n").unwrap();
    wait_until("the shell to exit on its own", || {
        status_of(&m, agent.id) == Some(Status::Exited)
    })
    .await;

    let started = Instant::now();
    m.remove_with_worktree(agent.id, false, &roots)
        .await
        .expect("an exited window's worktree is removed");
    let took = started.elapsed();

    assert!(
        took < Duration::from_secs(1),
        "an exited window waited {took:?} for a child that was already gone"
    );
    assert!(!listed(&m, agent.id));
    assert!(!agent.path.exists());
    roots.assert_one_unregister_before_the_removal(&agent.path);
}
