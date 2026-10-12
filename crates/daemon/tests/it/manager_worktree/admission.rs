//! Phase A and phase C: when a create is admitted, what it claims while it is in git,
//! and what happens to one that finishes after the manager has moved on.
//!
//! A submodule of `tests/manager_worktree.rs` rather than a test binary of its own, so it
//! keeps that file's `manager()`, `spec()`, `worktree_spec()`, `wait_until()` and
//! `drain()` helpers. The split is AGENTS.md rule 8, along the seam that was already
//! there: the parent asserts *what a worktree window reports*, this file asserts *where
//! the blocking happened and what the lock protected while it did*.

use super::*;

/// Two creates must never both enter phase B for one worktree directory.
///
/// Phase B runs git without the lock, so without a claim taken at admission both creates
/// run `git worktree add` concurrently. The loser's pre-flight can pass before the
/// winner's add has registered anything, in which case the loser goes on to add, fails,
/// and cleans up after what it believes is its own half-made worktree — `git worktree
/// remove --force` against the winner's live checkout, and `git branch -D` on its branch.
/// `--force` overrides the dirty refusal, so the other agent's uncommitted work goes with
/// it. Running parallel agents on one repository is the point of this milestone, so this
/// is the normal case.
///
/// The two branches here are deliberately different branches: `branch_dir_name` maps `/`
/// to `-`, so `feat/x` and `feat-x` want one directory while decision 9's "already
/// checked out" check can never flag them. Only the directory claim catches this pair.
#[tokio::test]
async fn two_creates_for_one_worktree_directory_cannot_both_reach_git() {
    let repo = TempRepo::new();
    let (m, _keep, wt_root) = manager();
    let contested = repo_worktrees_dir(&wt_root, &repo.root).join("feat-x");

    let (first, second) = tokio::join!(
        m.create(
            worktree_spec("one", &repo.root, "feat/x"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        ),
        m.create(
            worktree_spec("two", &repo.root, "feat-x"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        ),
    );

    let (winner, refusal) = match (first, second) {
        (Ok(winner), Err(refusal)) => (winner, refusal),
        (Err(refusal), Ok(winner)) => (winner, refusal),
        (first, second) => panic!("exactly one create may win: {first:?} / {second:?}"),
    };
    let refusal = refusal.to_string();

    // Refused at admission, not by git from inside phase B: a git-level "path already
    // exists" would mean the loser had reached the step that cleans up after itself.
    assert!(
        refusal.contains("is already being created"),
        "the loser must be refused at admission: {refusal}"
    );
    assert!(
        refusal.contains(&contested.display().to_string()),
        "the refusal must name the directory: {refusal}"
    );

    // The winner is untouched: still on disk, still a checkout git knows about, and its
    // branch still exists.
    let branch = winner
        .branch
        .clone()
        .expect("the winner asked for a branch");
    assert_eq!(winner.cwd, contested);
    assert!(contested.is_dir(), "the winner's checkout was deleted");
    assert!(
        repo.worktree_paths().contains(&contested),
        "git no longer knows about the winner's checkout"
    );
    assert!(
        repo.branch_exists(&branch),
        "the winner's branch '{branch}' was deleted"
    );
    assert_eq!(m.list().len(), 1, "only the winner may be listed");

    drain(&m);
}

/// Phase C refuses a create that was admitted before `shutdown` and finished phase B
/// after it. Without the guard the window is inserted into a manager that has already
/// killed everything it knew about and is exiting, so its agent outlives the daemon:
/// `anthrex daemon stop` seconds after `anthrex new --worktree` would leave a live
/// process nothing can reach.
#[tokio::test]
async fn a_create_that_races_shutdown_is_refused_and_its_child_killed() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(2);
    let (m, _keep, wt_root, mut events) = manager_watching_events();

    let worker = m.clone();
    let root = repo.root.clone();
    let racing = tokio::spawn(async move {
        worker
            .create(
                worktree_spec("late", &root, "feat/late"),
                root.clone(),
                Some(root.clone()),
                80,
                24,
            )
            .await
    });

    // Shut down while the create is inside `git worktree add`, which is the only window
    // in which phase A has admitted it and phase C has not yet run.
    let marker = repo.hook_marker();
    wait_until("the post-checkout hook to start", || marker.exists()).await;
    m.shutdown().await;

    let error = racing
        .await
        .unwrap()
        .expect_err("a create that finishes after shutdown must be refused")
        .to_string();

    assert!(error.contains("shutting down"), "{error}");
    assert!(
        m.list().is_empty(),
        "no window may be inserted after shutdown"
    );

    // The worktree really was made and is deliberately left on disk, so the error is the
    // only place its path is ever named — no entry exists for a removal to find it by.
    let orphan = repo_worktrees_dir(&wt_root, &repo.root).join("feat-late");
    assert!(
        orphan.is_dir(),
        "the worktree was created before the refusal"
    );
    assert!(
        error.contains(&orphan.display().to_string()),
        "the error must name the worktree left behind: {error}"
    );

    // And the child is gone. It never became an entry, so `list()` and `child_pid` cannot
    // see it; its `Exited` event is the daemon's only evidence that it was reaped.
    let signal = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let (_, event) = events.recv().await.expect("the event channel is open");
            if let WindowEvent::Exited { signal, .. } = event {
                return signal;
            }
        }
    })
    .await
    .expect("the refused window's child was never reaped");
    assert!(
        signal.is_some(),
        "the child must be signalled, not left to exit on its own: {signal:?}"
    );
}

/// The lock discipline itself: `git worktree add` runs in phase B, which holds nothing.
/// While one is blocked inside git, `list()` must answer immediately and an unrelated
/// create must run to completion — neither is true if phase B holds the manager lock.
#[tokio::test]
async fn a_slow_worktree_create_does_not_block_the_manager() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(2);
    let (m, _keep, _wt_root) = manager();

    let worker = m.clone();
    let root = repo.root.clone();
    let slow = tokio::spawn(async move {
        worker
            .create(
                worktree_spec("slow", &root, "feat/slow"),
                root.clone(),
                Some(root.clone()),
                80,
                24,
            )
            .await
    });

    let marker = repo.hook_marker();
    wait_until("the post-checkout hook to start", || marker.exists()).await;

    let started = Instant::now();
    let listed = m.list();
    let list_took = started.elapsed();
    assert!(
        list_took < Duration::from_millis(100),
        "list() waited on the manager lock for {list_took:?}"
    );
    assert!(
        listed.is_empty(),
        "the slow create has not been admitted yet"
    );

    let started = Instant::now();
    let other = m
        .create(
            spec("other", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .expect("an unrelated create must not wait on git");
    let create_took = started.elapsed();
    assert!(
        create_took < Duration::from_secs(1),
        "an unrelated create waited {create_took:?} on the slow one"
    );

    let info = slow.await.unwrap().expect("the slow create still succeeds");
    assert_eq!(info.branch.as_deref(), Some("feat/slow"));
    assert_ne!(info.id, other.id, "an id is never handed out twice");

    drain(&m);
}

/// Design decision 15 phase A: the name is taken the moment the create is admitted, not
/// when the window appears. Without the reservation, two creates racing on one name would
/// both pass the duplicate check while the first was inside git.
#[tokio::test]
async fn a_name_is_reserved_while_its_create_is_in_flight() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(2);
    let (m, _keep, _wt_root) = manager();

    let other = m
        .create(
            spec("other", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .unwrap();

    let worker = m.clone();
    let root = repo.root.clone();
    let slow = tokio::spawn(async move {
        worker
            .create(
                worktree_spec("dup", &root, "feat/dup"),
                root.clone(),
                Some(root.clone()),
                80,
                24,
            )
            .await
    });

    let marker = repo.hook_marker();
    wait_until("the post-checkout hook to start", || marker.exists()).await;

    let clash = m
        .create(
            spec("dup", &repo.root),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        )
        .await
        .expect_err("a name still being created is taken")
        .to_string();
    assert!(clash.contains("already exists"), "{clash}");

    let renamed = m
        .rename(other.id, "dup".to_string())
        .expect_err("rename must respect the reservation too")
        .to_string();
    assert!(renamed.contains("already exists"), "{renamed}");

    let info = slow.await.unwrap().expect("the reserved create succeeds");
    assert_eq!(info.name, "dup");
    assert!(
        m.list().iter().any(|w| w.name == "dup"),
        "the reserved name is now a real window"
    );

    drain(&m);
}

/// Whole-branch review finding 4, as its reproduction staged it: two concurrent creates
/// for one worktree directory where the two callers' project roots **disagree**, which is
/// what a `DETECT_TIMEOUT` on one of them produces. The branches are `feat/x` and
/// `feat-x` again — one directory, two genuinely different branches, invisible to
/// decision 9's "already checked out" check.
///
/// Observed before the fix: neither create was refused, **both entered phase B and ran
/// `git worktree add` against one directory**, and only git's own `index.lock` happened
/// to keep that harmless. The claim was computed from the caller's root while
/// `worktree::create` resolved its own, so the moment the two disagreed the claim guarded
/// a directory nobody would ever make.
///
/// Now the roots are resolved once and passed down, so the fallback create cannot reach
/// git at all: it is refused for the reason the fallback actually means — the caller could
/// not say which repository this is — instead of quietly re-deriving an answer the claim
/// knows nothing about.
#[tokio::test]
async fn a_fallback_root_cannot_race_a_resolved_one_into_git() {
    let repo = TempRepo::new();
    let sub = repo.root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let (m, _keep, wt_root) = manager();
    let contested = repo_worktrees_dir(&wt_root, &repo.root).join("feat-x");

    let (resolved, fell_back) = tokio::join!(
        // What the server passes when detection succeeded.
        m.create(
            worktree_spec("resolved", &repo.root, "feat/x"),
            repo.root.clone(),
            Some(repo.root.clone()),
            80,
            24,
        ),
        // What `project::detect_roots_with` returns when detection times out, is
        // truncated, or cannot spawn: the canonical `cwd`, and no worktree at all.
        m.create(
            worktree_spec("fell-back", &sub, "feat-x"),
            sub.clone(),
            None,
            80,
            24,
        ),
    );

    let winner = resolved.expect("the create whose roots resolved must be unaffected");
    let refusal = fell_back
        .expect_err("a create whose root detection fell back must not reach git")
        .to_string();
    assert!(
        refusal.contains("not a git repository"),
        "the refusal must name what actually went wrong: {refusal}"
    );

    assert_eq!(
        winner.cwd, contested,
        "the winner keeps the directory its own admission claimed"
    );
    assert!(contested.is_dir(), "the winner's checkout was deleted");
    assert!(
        repo.worktree_paths().contains(&contested),
        "git no longer knows about the winner's checkout"
    );
    assert_eq!(m.list().len(), 1, "only the winner may be listed");

    drain(&m);
}

/// The invariant underneath that race, without the race: the directory a create's
/// worktree ends up in is the directory its **admission** claimed, derived from the root
/// its caller resolved — not from a root phase B works out for itself.
///
/// The roots here are valid but not the ones `create` would have detected: `project` is a
/// subdirectory of the repository, which is what a fallback produces and what the review
/// passed to reproduce the divergence. Before the fix this create landed under
/// `<wt>/<repo>-<hash>/`, a directory nothing had claimed, while its claim sat unused
/// under `<wt>/sub-<hash>/`.
#[tokio::test]
async fn a_create_lands_in_the_directory_its_admission_claimed() {
    let repo = TempRepo::new();
    let sub = repo.root.join("sub");
    std::fs::create_dir(&sub).unwrap();
    let (m, _keep, wt_root) = manager();

    let info = m
        .create(
            worktree_spec("divergent", &sub, "feat/x"),
            sub.clone(),
            Some(sub.clone()),
            80,
            24,
        )
        .await
        .expect("a subdirectory root is still a usable root");

    assert_eq!(
        info.cwd,
        daemon::worktree::worktree_dir(&wt_root, &sub, "feat/x"),
        "the checkout must be where the claim computed from this caller's root says, \
         not where a second resolution would have put it"
    );
    assert_ne!(
        info.cwd,
        daemon::worktree::worktree_dir(&wt_root, &repo.root, "feat/x"),
        "landing under the re-resolved root is the divergence itself"
    );

    drain(&m);
}
