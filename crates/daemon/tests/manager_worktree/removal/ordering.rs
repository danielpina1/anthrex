//! The half of removal that is about *order and contention* rather than outcome.
//!
//! A submodule of `removal.rs`, which keeps its `FakeRoots`, `worktree_agent`,
//! `group_alive` and the rest; the split is AGENTS.md rule 8 along the seam that file's
//! own docs draw. Its parent asserts what ends up on disk. These assert **when** each step
//! happens and what two callers racing for one window do — the questions where an
//! implementation can reach the right end state by a route that destroys work on the way.
//!
//! Every one of them is timed from `FakeRoots`' `unregister` hook, which the manager calls
//! in the instant between the kill and `worktree::remove`. That is the window every hazard
//! here lives in, and reaching it through test-owned code is what makes these deterministic
//! instead of races the test has to win.

use super::*;

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

/// A window whose screen parser panicked reports Exited while its child is **still
/// alive**: `WindowEvent::ParserPanicked` sets the status, and `Inner::start_cleanup` then
/// walks the process group through HUP, TERM and KILL over three seconds. Those three
/// seconds are a real window — long enough for a user to answer a remove confirm in.
///
/// A removal that took the status for the answer would signal nothing and hand the
/// checkout to `git worktree remove` with an agent still writing into it. With `--force`,
/// where both dirty checks are skipped, everything written in those seconds would go with
/// the tree; without it, the dirty check has already passed by then. `WindowManager::remove`
/// signals on `child_alive` alone, and this path must not be weaker than the one it
/// mirrors.
///
/// The agent ignores HUP and TERM so that the escalation cannot reach it first — which is
/// what makes the Exited-but-alive state last long enough to test rather than a race — and
/// the question is asked from the `unregister` hook, in the instant before the deletion.
/// It is not "does the agent die", which it does either way, but "was it already dead when
/// its checkout was deleted".
#[tokio::test]
async fn a_parser_panicked_window_is_killed_before_its_checkout_goes() {
    let repo = TempRepo::new();
    let (m, _keep, _wt_root) = manager();
    let agent = worktree_agent(&m, &repo, "panicked", "feat/panicked").await;
    let pid = m
        .child_pid(agent.id)
        .unwrap()
        .expect("the agent has a child");

    m.write_input(agent.id, b"trap '' HUP TERM; echo trapped\n")
        .unwrap();
    wait_until("the agent to ignore HUP and TERM", || {
        let (snapshot, _, _) = m.snapshot(agent.id).unwrap();
        String::from_utf8_lossy(&snapshot).contains("trapped")
    })
    .await;

    m.handle_event(agent.id, WindowEvent::ParserPanicked("boom".to_string()));
    assert_eq!(
        status_of(&m, agent.id),
        Some(Status::Exited),
        "a parser panic reports the window as exited"
    );
    assert!(
        group_alive(pid),
        "but its agent is still running, which is the whole hazard"
    );

    let alive_at_deletion = Arc::new(Mutex::new(None));
    let roots = {
        let seen = alive_at_deletion.clone();
        FakeRoots::with_hook(move |_| *seen.lock().unwrap() = Some(group_alive(pid)))
    };

    m.remove_with_worktree(agent.id, false, &roots)
        .await
        .expect("a panicked window's worktree is still removable");

    assert_eq!(
        *alive_at_deletion.lock().unwrap(),
        Some(false),
        "the agent was still running when its checkout was deleted"
    );
    assert!(!listed(&m, agent.id));
    assert!(!agent.path.exists());
    assert!(repo.branch_exists("feat/panicked"));
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
