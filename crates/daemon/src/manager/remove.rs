//! `WindowManager::remove_with_worktree`: removing a window *and* the checkout this
//! daemon made for it (design decisions 18 to 20 and 22).
//!
//! [`WindowManager::remove`] is untouched and stays the default everywhere — `C-b x`,
//! `anthrex kill`, and a remove confirm with the box unticked all leave the worktree on
//! disk (design decision 18). This is the other path, the one that deletes a directory the
//! user cannot get back, and it is written around two rules.
//!
//! # The tree is asked before anything is killed — on the common path
//!
//! Step 2's dirty check runs *before* step 3 signals the agent, not after, so on the
//! common path a refusal costs the user nothing: the agent is still running in the
//! checkout it was working in, and they can go and look at what is in there.
//! `app/modal_keys.rs`'s module doc hedges this the same way and for the same reason:
//! **one path is the exception.** Risk 8 — a file appearing between step 2's check and
//! step 3's kill — means `worktree::remove` in step 4 can *itself* answer
//! [`WorktreeError::Dirty`], and [`blocking`]'s doc comment says so; that refusal reaches
//! the user as the identical prompt, but by then the agent has already been SIGKILLed, so
//! "the agent is still running in the checkout" is false on that one path. This is why
//! neither the remove-confirm dialog nor the force-or-keep follow-up may say anything
//! about the agent's process at all, rather than saying "the agent is still running": a
//! rule that reads accurate here and gets qualified without a matching qualifier at the
//! dialog would leave the dialog's wording resting on an unhedged claim.
//!
//! The check itself lives in [`crate::worktree::dirty_reason`], which counts a paused
//! rebase, a paused bisect and an unreachable detached `HEAD` as work precisely because
//! `git status` does not — and which says *which* of them it found, so the refusal the
//! user reads names the state they can actually go and check. Every question it cannot
//! answer is an error here rather than a "no": refusing to remove a clean worktree is an
//! annoyance, removing a dirty one is lost work, and only one of those is recoverable.
//!
//! # The watcher is told before the directory goes, and only when it goes
//!
//! Design decision 22. Milestone 4.5 watches a worktree root recursively and publishes its
//! git state; a root whose directory has been deleted would go on being polled, every
//! probe failing against a missing path, with the last state it published left on screen.
//! So `unregister` runs immediately before [`crate::worktree::remove`]. The converse
//! matters just as much and is easier to get wrong: a root whose worktree *survived* — a
//! refused removal, a removal git declined — must not be left unwatched, so the dirty
//! check sits before the `unregister` and a failed removal `register`s the root again.
//!
//! The manager triggers this rather than the server because only the manager is inside the
//! operation. The server sees one call and its result, so from there the two orderings are
//! indistinguishable, and splitting the operation in two so it could see between them
//! would put a race exactly where the directory is being deleted. What the manager learns
//! about git in exchange stops at [`GitRoots`]: two methods, no `GitState`, no registry
//! type, no knowledge that a watcher exists at all.
//!
//! Nothing here re-derives "is this the last window on this root" from a `list()`
//! snapshot (design decision 23). The registry is reference-counted and `unregister` is
//! called once per removed window; milestone 4.5 removed the snapshot pattern because it
//! raced a concurrent create across two independent locks, and this is the function where
//! it would come back.

use super::{Entry, KILL_GRACE, WindowManager, git};
use crate::worktree::{self, ManagedWorktree, WorktreeError};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often the kill wait asks whether the child has been reaped (design decision 19
/// step 3). Never with the manager lock held.
const KILL_POLL: Duration = Duration::from_millis(25);

/// The two calls [`WindowManager::remove_with_worktree`] needs from milestone 4.5's
/// `GitRegistry`, and deliberately nothing else (design decision 22).
///
/// It is this narrow so that the ordering above can live in the manager without the
/// manager learning what a git state is. `GitRegistry` implements it; the manager's tests
/// implement it with a recorder, which is the only way to assert the *sequence* rather
/// than just the end state.
pub trait GitRoots: Send + Sync {
    /// Adds one reference to `root`, starting to watch it if this is the first.
    fn register(&self, root: PathBuf);
    /// Removes one reference from `root`, stopping the watch if it was the last.
    fn unregister(&self, root: &Path);
}

/// Why a removal did not happen.
///
/// The two variants are one distinction: whether the user has a choice to make.
/// [`RemoveError::Dirty`] means the worktree holds work, and the client turns it into the
/// force-or-keep prompt (design decisions 24 and 36). Everything else is a failure the
/// user cannot answer with a keystroke, and is shown as it is.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    /// Always a [`WorktreeError::Dirty`].
    #[error("{0}")]
    Dirty(WorktreeError),
    #[error("{0}")]
    Failed(#[from] anyhow::Error),
}

/// Holds a window's `removing` flag for as long as one removal owns it, and gives it back
/// on every exit path that leaves the window in place — an early return, a panic on the
/// blocking pool, or a caller that drops the future.
///
/// The guard exists rather than a bare pair of assignments for the same reason
/// `create`'s [`super::create`] `Reservation` does: a flag left set after a failure makes
/// the window permanently unremovable, with nothing in `list()` to explain why, until the
/// daemon restarts.
struct Removing<'a> {
    manager: &'a WindowManager,
    id: u32,
    held: bool,
}

impl Removing<'_> {
    /// The window has been removed, so there is no flag left to give back.
    fn forget(mut self) {
        self.held = false;
    }
}

impl Drop for Removing<'_> {
    fn drop(&mut self) {
        if !self.held {
            return;
        }
        let mut inner = crate::lock(&self.manager.inner);
        if let Some(entry) = inner.entries.get_mut(&self.id) {
            entry.removing = false;
        }
    }
}

impl WindowManager {
    /// Removes a window together with the worktree this daemon created for it, in the
    /// order design decision 19 fixes.
    ///
    /// `force` skips the dirty check and hands `--force` to git, which is the user's
    /// answer to a [`RemoveError::Dirty`] and the only thing that discards their changes.
    /// It is meaningless without a worktree to remove; design decision 20's refusal of
    /// `force` on a plain removal is the server's, because [`WindowManager::remove`] has
    /// no `force` argument to refuse it on.
    ///
    /// Every git command this runs shares one deadline (design decision 3), opened here
    /// and spent across the dirty check, the kill wait and the removal, so the whole
    /// operation is bounded by `OPERATION_TIMEOUT + KILL_GRACE` — which is what
    /// `client::WORKTREE_REQUEST_TIMEOUT` is budgeted against.
    ///
    /// Nothing blocking happens on a tokio worker or under the manager lock (AGENTS.md
    /// hard rules 2 and 10): git runs inside `spawn_blocking`, and the kill wait sleeps
    /// between lock acquisitions rather than holding one.
    pub async fn remove_with_worktree(
        &self,
        id: u32,
        force: bool,
        roots: &dyn GitRoots,
    ) -> Result<(), RemoveError> {
        let deadline = Instant::now() + worktree::OPERATION_TIMEOUT;

        // Step 1, under the lock: is this window removable, and is anyone else already
        // removing it? The guard clears `removing` again on every path below that leaves
        // the window listed.
        let (wt, guard) = self.begin_removal(id)?;

        // Step 2, before the kill: does the tree hold work? A refusal here has cost the
        // user nothing — their agent is still running in the checkout.
        //
        // Skipped for a path that is not there, which is not the same as skipping it for
        // convenience. `git -C <missing>` exits 128, so asking a directory that has been
        // deleted turns design decision 13's "path missing → prune → succeed" branch into
        // a raw git error about a directory that does not exist — unreachable except with
        // `--force`, the one flag that sounds destructive. A user who tidied up with their
        // own `git worktree remove` and then closed the window would be told to force it.
        if !force && !worktree_is_gone(&wt.path) {
            let path = wt.path.clone();
            let dirty = blocking(move || worktree::dirty_reason(git(), &path, deadline)).await?;
            if let Some(reason) = dirty {
                return Err(RemoveError::Dirty(WorktreeError::Dirty {
                    path: wt.path,
                    reason,
                }));
            }
        }

        // Step 3: the checkout cannot be removed out from under a live process.
        self.kill_and_await_exit(id).await;

        // Step 4. The order of these two lines is design decision 22 and the reason this
        // function takes `roots` at all: the watcher stops before the directory goes, and
        // starts again if the directory turns out to have stayed.
        roots.unregister(&wt.path);
        let removal = {
            let wt = wt.clone();
            blocking(move || worktree::remove(git(), &wt, force, deadline)).await
        };
        if let Err(error) = removal {
            roots.register(wt.path.clone());
            return Err(error);
        }

        // Step 5, under the lock again.
        self.finish_removal(id, &wt);
        guard.forget();
        Ok(())
    }

    /// Design decision 19 step 1. Pure bookkeeping under the lock: nothing here touches
    /// the disk, so the lock is held for as long as a `BTreeMap` lookup takes.
    fn begin_removal(&self, id: u32) -> anyhow::Result<(ManagedWorktree, Removing<'_>)> {
        let mut inner = crate::lock(&self.inner);
        let entry: &mut Entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        // `managed`, never `worktree`: the first is "did this daemon make this checkout,
        // and may it delete it", the second is milestone 4.5's watched root, which every
        // window in a repository has and almost none of them owns.
        let Some(wt) = entry.managed.clone() else {
            anyhow::bail!("window '{}' has no worktree", entry.name);
        };
        if entry.removing {
            anyhow::bail!("window '{}' is already being removed", entry.name);
        }
        entry.removing = true;
        Ok((
            wt,
            Removing {
                manager: self,
                id,
                held: true,
            },
        ))
    }

    /// Design decision 19 step 3: SIGKILL to the process group, the way
    /// [`WindowManager::remove`] does it, then wait — without the lock — until the child
    /// has actually been reaped.
    ///
    /// Both halves ask `child_alive`, and **neither asks whether the status is Exited**,
    /// because those are not the same question. `child_alive` goes false only when
    /// [`crate::window::WindowEvent::Exited`] arrives, which is the daemon's evidence that
    /// the process is gone. A status of Exited can precede that by seconds:
    /// `WindowEvent::ParserPanicked` sets the status and leaves the child running while
    /// `Inner::start_cleanup` walks it through HUP, TERM and KILL. A removal that
    /// short-circuited on the status would signal nothing and go straight on to delete the
    /// checkout out from under a live agent that is still writing into it — and with
    /// `force`, with both dirty checks skipped, everything it wrote in those seconds would
    /// go with the tree. `WindowManager::remove` signals on `child_alive` alone; this must
    /// not be weaker than the path it mirrors.
    ///
    /// A window whose child is already reaped returns at once. That is not an
    /// optimisation: removing a long-dead agent would otherwise sit through the whole of
    /// [`KILL_GRACE`] with a dialog on screen that looks hung.
    ///
    /// The grace is a bound, not a requirement. If the exit has not been reported when it
    /// runs out the removal goes ahead anyway: the child has had a SIGKILL, and a worktree
    /// the user asked to remove staying forever because one event never arrived is the
    /// worse failure.
    async fn kill_and_await_exit(&self, id: u32) {
        {
            let inner = crate::lock(&self.inner);
            let Some(entry) = inner.entries.get(&id) else {
                return;
            };
            if !entry.child_alive {
                return;
            }
            let _ = entry.window.signal_group(libc::SIGKILL);
        }

        let deadline = Instant::now() + KILL_GRACE;
        loop {
            if self.child_is_gone(id) {
                return;
            }
            if Instant::now() >= deadline {
                tracing::warn!(
                    id,
                    "no exit reported within the kill grace; removing the worktree anyway"
                );
                return;
            }
            tokio::time::sleep(KILL_POLL).await;
        }
    }

    /// Whether this window's child has been reaped, asked once under the lock rather than
    /// by building a `WindowInfo` for every other window 40 times a second — and asked of
    /// `child_alive`, which `list()` does not publish, for the reason above.
    ///
    /// A missing entry counts as gone too. Not because a concurrent plain `remove` can
    /// still take it out from under this loop — [`WindowManager::remove`]'s own
    /// `entry.removing` check (added by the same change that gave `child_is_gone` its
    /// current name) refuses exactly that for as long as this removal holds the flag, so
    /// the id this loop polls cannot be unlisted by another caller while it runs. It is
    /// belt and braces instead: if this id were ever unlisted anyway — a future bug in
    /// that guard, not anything reachable today — there would be no evidence about it
    /// left to wait for, and treating "not there" as "gone" is what lets the loop return
    /// rather than sit out the whole of [`KILL_GRACE`] waiting for an entry that no
    /// longer exists to change.
    fn child_is_gone(&self, id: u32) -> bool {
        crate::lock(&self.inner)
            .entries
            .get(&id)
            .is_none_or(|entry| !entry.child_alive)
    }

    /// Design decision 19 step 5.
    ///
    /// The SIGKILL mirrors [`WindowManager::remove`] and is the last thing that can reach
    /// this process group: once the entry is forgotten, nothing holds its pid. It is not
    /// redundant with step 3 — a child that outlived [`KILL_GRACE`] without reporting an
    /// exit reaches here alive — but it is also not a substitute for it, because by this
    /// point `worktree::remove` has already deleted the checkout. Keeping a live agent out
    /// of a tree that is about to be deleted is step 3's job and only step 3's.
    fn finish_removal(&self, id: u32, wt: &ManagedWorktree) {
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.remove(&id) else {
            return;
        };
        if entry.child_alive {
            let _ = entry.window.signal_group(libc::SIGKILL);
        }
        drop(entry);
        tracing::info!(id, path = ?wt.path, branch = %wt.branch, "window and worktree removed");
        self.publish(&inner);
    }
}

/// Whether there is nothing at `path` to ask questions about, so that step 2's dirty
/// check can be skipped and [`worktree::remove`] can go straight to its prune.
///
/// **Only [`io::ErrorKind::NotFound`] answers yes**, exactly as `worktree::remove` decides
/// the same question: a `PermissionDenied`, or an `EIO` from a dead network mount, is not
/// a worktree that is gone. Those fall through to the dirty check, where git fails and the
/// removal fails with it, rather than being quietly treated as work that is no longer
/// there. The safety-critical direction is the one this function must never guess in.
fn worktree_is_gone(path: &Path) -> bool {
    matches!(
        std::fs::symlink_metadata(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound
    )
}

/// Runs one blocking worktree call on the blocking pool and gives its failure the shape
/// the client needs.
///
/// A [`WorktreeError::Dirty`] from `worktree::remove` is the same answer as the one the
/// check in step 2 produces — a file that appeared between the check and the kill (risk 8)
/// — so it reaches the user as the same prompt rather than as an opaque git failure.
async fn blocking<T: Send + 'static>(
    task: impl FnOnce() -> Result<T, WorktreeError> + Send + 'static,
) -> Result<T, RemoveError> {
    match tokio::task::spawn_blocking(task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error @ WorktreeError::Dirty { .. })) => Err(RemoveError::Dirty(error)),
        Ok(Err(error)) => Err(RemoveError::Failed(anyhow::anyhow!(error))),
        Err(error) => Err(RemoveError::Failed(anyhow::anyhow!(
            "worktree removal failed: {error}"
        ))),
    }
}
