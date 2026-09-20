//! The filesystem watcher for one worktree, and the pure path filter in front of it.
//!
//! Design decision 11: `notify`'s recommended watcher, recursive over the worktree
//! root plus non-recursive over that worktree's own git dir. Design decision 12: the
//! events it produces are filtered **by path only** — no gitignore parsing, because
//! parsing `.gitignore` correctly is a project of its own and getting it subtly wrong
//! would drop real edits.

use std::any::Any;
use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc::UnboundedSender;

/// Design decision 12: a path with any component equal to one of these is a build or
/// dependency directory, and a probe would only ever report what git already ignores.
pub const DENY_COMPONENTS: [&str; 6] =
    ["target", "node_modules", ".venv", "dist", "build", ".next"];

/// Whether an event on `path` should wake the scheduler.
///
/// Pure, and the only thing that decides it. `root` is the watched worktree root and
/// `git_dir` is that worktree's own git dir when it could be resolved; pass both
/// canonicalised, because the paths a watcher reports are canonical on macOS (a
/// `/var/folders/...` root is reported under `/private/var/folders/...`) and a prefix
/// test against an uncanonicalised one would silently never match.
pub fn accepts(path: &Path, root: &Path, git_dir: Option<&Path>) -> bool {
    // `*.lock` catches `index.lock`, `config.lock`, `packed-refs.lock` and the rest of
    // git's own transient files. It is a name test, not a component test: a directory
    // called `x.lock` is not a thing, and a file is what the churn is.
    if path
        .file_name()
        .is_some_and(|name| name.as_encoded_bytes().ends_with(b".lock"))
    {
        return false;
    }
    // Only the components *below* the root are the worktree's own. Scanning the whole
    // absolute path instead would reject every event in a checkout that happens to
    // live under a directory called `build` or `dist` — which does not announce
    // itself: the root would simply go quiet and fall back to the 30-second poll.
    let inside = path
        .strip_prefix(root)
        .or_else(|_| path.strip_prefix(git_dir.unwrap_or(root)))
        .unwrap_or(path);
    if inside.components().any(|component| {
        matches!(component, Component::Normal(name)
            if DENY_COMPONENTS.iter().any(|deny| name == OsStr::new(deny)))
    }) {
        return false;
    }
    if let Some(git_dir) = git_dir
        && (path.starts_with(git_dir.join("objects")) || path.starts_with(git_dir.join("lfs")))
    {
        return false;
    }
    true
}

/// A live watcher, kept alive by being held and disarmed by being dropped — with the
/// one guarantee the naked watcher cannot give: **its destructor never runs on a tokio
/// runtime thread.**
///
/// `RecommendedWatcher`'s `Drop` is blocking work. On macOS it is `FsEventWatcher`,
/// whose `Drop` spins on `CFRunLoopIsWaiting` and then `join()`s the run-loop thread;
/// the inotify backend likewise joins its event thread. The registry's watcher lives
/// inside the per-root future, so without this guard it is destroyed on a runtime
/// worker — on the worker that drops the aborted future for `unregister`, and on
/// whichever worker drops it at teardown. Removing the last window on a repository
/// that is churning (the `git checkout` of a large tree the circuit breaker exists for)
/// would then burn a worker at 100% and block it on `join()`; on a two-worker runtime
/// that is half the executor, with every other window's PTY forwarding queued behind
/// it. That is AGENTS.md hard rule 2, and hard rule 10 for git specifically.
///
/// So the watcher is type-erased into a box that is handed to [`tokio::task::spawn_blocking`]
/// for disposal. Nothing ever reads the box back; holding it is what keeps the watch
/// armed. Disposal is detached on purpose: it has to outlive this drop, which is itself
/// often the tail of a future the runtime is throwing away.
pub struct WatchGuard(Option<Box<dyn Any + Send>>);

impl WatchGuard {
    fn new<W: Send + 'static>(watcher: W) -> Self {
        Self(Some(Box::new(watcher)))
    }
}

impl Drop for WatchGuard {
    fn drop(&mut self) {
        let Some(watcher) = self.0.take() else {
            return;
        };
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                // The `JoinHandle` is dropped immediately; a `spawn_blocking` task runs
                // to completion regardless. The one case where the closure is dropped
                // rather than run is a runtime already shutting down, and then the drop
                // lands on the thread driving that shutdown — never on a worker with
                // other windows' work queued behind it.
                drop(handle.spawn_blocking(move || drop(watcher)));
            }
            // No runtime on this thread means no runtime thread to protect, and there
            // is nothing waiting behind this drop.
            Err(_) => drop(watcher),
        }
    }
}

/// Builds and arms the watcher for one root, sending one `()` per accepted event.
///
/// **Blocking**: on the inotify backend `watch(.., Recursive)` walks the tree, which
/// on a large checkout is real work, so the caller runs this on `spawn_blocking`. The
/// returned [`WatchGuard`] must be kept alive — dropping it unwatches everything, which
/// is exactly how the registry's per-root task tears its watcher down when it is
/// aborted, and why the guard exists rather than the watcher itself.
///
/// Failure is not fatal (design decision 15): the caller logs it once and leaves the
/// root on the 30-second poll.
pub fn build(
    root: &Path,
    git_dir: Option<&Path>,
    events: UnboundedSender<()>,
) -> notify::Result<WatchGuard> {
    let filter_root = root.to_path_buf();
    let filter_git_dir: Option<PathBuf> = git_dir.map(Path::to_path_buf);
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            // A watcher-level error (an overflowed queue, a vanished directory) is not
            // information about the worktree; the safety poll covers what it lost.
            return;
        };
        if event
            .paths
            .iter()
            .any(|path| accepts(path, &filter_root, filter_git_dir.as_deref()))
        {
            let _ = events.send(());
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    if let Some(git_dir) = git_dir
        && !git_dir.starts_with(root)
    {
        // A linked worktree's git dir lives under the *common* dir, outside this root,
        // so it needs its own watch. An ordinary checkout's `<root>/.git` is already
        // covered by the recursive watch above, and adding a second watch for it is at
        // best redundant and at worst an error on backends that reject duplicates.
        watcher.watch(git_dir, RecursiveMode::NonRecursive)?;
    }
    Ok(WatchGuard::new(watcher))
}

#[cfg(test)]
mod tests {
    //! Where a [`WatchGuard`]'s destructor runs. Both tests assert against the thread
    //! the test itself is on, which under `#[tokio::test]`'s current-thread flavour is
    //! the runtime's *only* thread: anything that ran there would have run on a runtime
    //! worker. A guard that disposed inline — the bug this type exists to prevent —
    //! fails both.

    use super::*;
    use std::sync::mpsc;
    use std::thread::ThreadId;
    use std::time::Duration;

    /// Reports the thread its own `Drop` ran on. Stands in for the watcher, whose
    /// blocking `Drop` is the thing that must not land on a runtime thread.
    struct Reporter(mpsc::Sender<ThreadId>);

    impl Drop for Reporter {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }

    fn disposed_on(rx: &mpsc::Receiver<ThreadId>) -> ThreadId {
        rx.recv_timeout(Duration::from_secs(5))
            .expect("the watcher must actually be disposed of, not leaked")
    }

    #[tokio::test]
    async fn dropping_a_guard_disposes_of_the_watcher_off_the_runtime_thread() {
        let (tx, rx) = mpsc::channel();
        let guard = WatchGuard::new(Reporter(tx));
        let runtime_thread = std::thread::current().id();

        drop(guard);

        assert_ne!(
            disposed_on(&rx),
            runtime_thread,
            "the watcher's blocking Drop ran on the runtime thread"
        );
    }

    #[tokio::test]
    async fn aborting_a_task_holding_a_guard_disposes_off_the_runtime_thread() {
        // The path `GitRegistry::unregister` takes: the future is never resumed, so
        // nothing after an await can do the teardown — only the guard's own `Drop`,
        // which the runtime runs when it throws the aborted future away.
        let (tx, rx) = mpsc::channel();
        let (started, has_started) = tokio::sync::oneshot::channel();
        let runtime_thread = std::thread::current().id();
        let task = tokio::spawn(async move {
            let _guard = WatchGuard::new(Reporter(tx));
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        has_started.await.expect("the task must reach its await");

        task.abort();
        // `abort` takes effect at the next yield point, so let the runtime drive the
        // task's drop before blocking this thread on the report.
        let _ = task.await;

        assert_ne!(
            disposed_on(&rx),
            runtime_thread,
            "an aborted task dropped its watcher on the runtime thread"
        );
    }
}
