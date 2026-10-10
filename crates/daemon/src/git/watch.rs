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
/// `ignore` is `config.toml`'s `git.ignore` (git-surface spec 3.7: "an `ignore` list
/// appended to the built-in filter"). It is appended, never substituted: a name here
/// can only make the filter reject more, so a mistyped entry cannot switch
/// [`DENY_COMPONENTS`] off.
pub fn accepts(path: &Path, root: &Path, git_dir: Option<&Path>, ignore: &[String]) -> bool {
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
            if DENY_COMPONENTS.iter().any(|deny| name == OsStr::new(deny))
                || ignore.iter().any(|deny| name == OsStr::new(deny)))
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
    /// Takes ownership of `watcher`; dropping the guard disposes of it off the runtime.
    /// Public so a test can hand [`crate::git::GitRegistry::with_seams`] a stand-in.
    pub fn new<W: Send + 'static>(watcher: W) -> Self {
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

/// The production [`crate::git::ArmFn`]: resolves `root`'s git dir and [`build`]s the
/// watcher over both. Blocking, like `build` itself.
pub fn arm(
    root: &Path,
    ignore: &[String],
    events: UnboundedSender<()>,
) -> notify::Result<WatchGuard> {
    // The watcher reports canonical paths, so the git dir the filter compares them
    // against has to be canonical too; on macOS a `/var/folders/...` root is reported
    // under `/private/var/folders/...`.
    let canonical = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let git_dir = crate::git::probe::resolve_git_dir(&canonical)
        .and_then(|git_dir| std::fs::canonicalize(git_dir).ok());
    build(&canonical, git_dir.as_deref(), ignore, events)
}

/// Builds and arms the watcher for one root, sending one `()` per accepted event.
///
/// **Blocking**: on Linux the root is walked ([`plan`]) and each kept directory
/// watched, which on a large checkout is real work, so the caller runs this on
/// `spawn_blocking`. The returned [`WatchGuard`] must be kept alive — dropping it
/// unwatches everything, which is exactly how the registry's per-root task tears its
/// watcher down when it is aborted, and why the guard exists rather than the watcher
/// itself.
///
/// Failure is not fatal (design decision 15): the caller logs it once and leaves the
/// root on the 30-second poll.
///
/// Milestone 9.11 decision 18 (B12, git-surface spec section 9's descriptor-exhaustion
/// item): on Linux (inotify, one descriptor per directory) only the directories
/// [`accepts`] keeps are registered, each `NonRecursive`; a directory created or
/// renamed in later is walked and watched by [`per_dir`]'s own thread before its event
/// is forwarded. Elsewhere (macOS's FSEvents: one stream, no descriptor per directory)
/// the root is watched recursively and `ignore` filters events only.
pub fn build(
    root: &Path,
    git_dir: Option<&Path>,
    ignore: &[String],
    events: UnboundedSender<()>,
) -> notify::Result<WatchGuard> {
    #[cfg(target_os = "linux")]
    {
        per_dir::build(root, git_dir, ignore, events)
    }
    #[cfg(not(target_os = "linux"))]
    {
        recursive(root, git_dir, ignore, events)
    }
}

/// [`build`] off Linux: one recursive watch over the root.
#[cfg(not(target_os = "linux"))]
fn recursive(
    root: &Path,
    git_dir: Option<&Path>,
    ignore: &[String],
    events: UnboundedSender<()>,
) -> notify::Result<WatchGuard> {
    let filter_root = root.to_path_buf();
    let filter_git_dir: Option<PathBuf> = git_dir.map(Path::to_path_buf);
    let filter_ignore = ignore.to_vec();
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
        let Ok(event) = result else {
            // A watcher-level error (an overflowed queue, a vanished directory) is not
            // information about the worktree; the safety poll covers what it lost.
            return;
        };
        if event.paths.iter().any(|path| {
            accepts(
                path,
                &filter_root,
                filter_git_dir.as_deref(),
                &filter_ignore,
            )
        }) {
            let _ = events.send(());
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    watch_outside_git_dir(&mut watcher, root, git_dir)?;
    Ok(WatchGuard::new(watcher))
}

/// A linked worktree's git dir lives under the *common* dir, outside this root, so it
/// needs its own watch. An ordinary checkout's `<root>/.git` is already covered by the
/// root's watch, and adding a second watch for it is at best redundant and at worst an
/// error on backends that reject duplicates.
fn watch_outside_git_dir(
    watcher: &mut impl Watcher,
    root: &Path,
    git_dir: Option<&Path>,
) -> notify::Result<()> {
    if let Some(git_dir) = git_dir
        && !git_dir.starts_with(root)
    {
        watcher.watch(git_dir, RecursiveMode::NonRecursive)?;
    }
    Ok(())
}

/// Milestone 9.11 decision 18 (B12): the directories under `root` a Linux watch
/// registers, `root` first: a walk that keeps each directory [`accepts`] keeps (so
/// [`DENY_COMPONENTS`], `ignore` and the git dir's `objects/` and `lfs/` are never
/// entered) and never follows a symbolic link. Blocking (it reads the tree).
pub fn plan(root: &Path, git_dir: Option<&Path>, ignore: &[String]) -> Vec<PathBuf> {
    plan_from(root, root, git_dir, ignore)
}

/// [`plan`] of the subtree at `start` (a directory under `root`, created or renamed in
/// after the watch was armed): `start` itself when accepted, then what it holds.
fn plan_from(start: &Path, root: &Path, git_dir: Option<&Path>, ignore: &[String]) -> Vec<PathBuf> {
    let mut kept = Vec::new();
    let is_dir = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir());
    if !is_dir(start) || !accepts(start, root, git_dir, ignore) {
        return kept;
    }
    let mut pending = vec![start.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                // `file_type` does not follow a link.
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let path = entry.path();
                    if accepts(&path, root, git_dir, ignore) {
                        pending.push(path);
                    }
                }
            }
        }
        kept.push(dir);
    }
    kept
}

/// Milestone 9.11 decision 18 (B12): the Linux watcher, one `NonRecursive` inotify
/// watch per directory [`plan`] keeps.
#[cfg(target_os = "linux")]
mod per_dir {
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, Weak, mpsc};

    use notify::event::{EventKind, ModifyKind};
    use notify::{RecommendedWatcher, RecursiveMode, Watcher};
    use tokio::sync::mpsc::UnboundedSender;

    use super::{WatchGuard, accepts, plan, plan_from, watch_outside_git_dir};

    /// The filter's inputs, owned.
    #[derive(Clone)]
    struct Scope {
        root: PathBuf,
        git_dir: Option<PathBuf>,
        ignore: Vec<String>,
    }

    impl Scope {
        fn accepts(&self, path: &Path) -> bool {
            accepts(path, &self.root, self.git_dir.as_deref(), &self.ignore)
        }
    }

    /// [`super::build`] on Linux. An accepted event on a path that is a directory
    /// created or renamed in is handed to [`adder`], which watches it (and what it
    /// holds) and only then forwards it, so a forwarded event for a new directory means
    /// it is watched. The callback runs on notify's event-loop thread, which a
    /// `watch` call itself waits on, so the adder is a thread of its own.
    pub(super) fn build(
        root: &Path,
        git_dir: Option<&Path>,
        ignore: &[String],
        events: UnboundedSender<()>,
    ) -> notify::Result<WatchGuard> {
        let scope = Scope {
            root: root.to_path_buf(),
            git_dir: git_dir.map(Path::to_path_buf),
            ignore: ignore.to_vec(),
        };
        let (dirs_tx, dirs_rx) = mpsc::channel::<PathBuf>();
        let (filter, forward) = (scope.clone(), events.clone());
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            let Ok(event) = result else {
                // A watcher-level error is not information about the worktree; the
                // safety poll covers what it lost.
                return;
            };
            let named = matches!(
                event.kind,
                EventKind::Create(_) | EventKind::Modify(ModifyKind::Name(_))
            );
            let mut accepted = false;
            for path in event.paths.iter().filter(|path| filter.accepts(path)) {
                // `lstat`: a link is never walked.
                if named && std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
                    let _ = dirs_tx.send(path.clone());
                } else {
                    accepted = true;
                }
            }
            if accepted {
                let _ = forward.send(());
            }
        })?;
        let watcher = Arc::new(Mutex::new(watcher));
        {
            let mut armed = crate::lock(&watcher);
            for dir in plan(root, git_dir, ignore) {
                // Review m14: a directory gone again since the walk (a build's churn)
                // is skipped, not a reason to leave the whole root on the poll.
                match armed.watch(&dir, RecursiveMode::NonRecursive) {
                    Err(err) if dir != root && gone(&err) => {}
                    other => other?,
                }
            }
            watch_outside_git_dir(&mut *armed, root, git_dir)?;
        }
        // The adder holds the watcher weakly: dropping the guard drops the watcher,
        // whose callback owns the sender, which ends the adder.
        let weak = Arc::downgrade(&watcher);
        std::thread::Builder::new()
            .name("anthrex-watch-dirs".to_string())
            .spawn(move || adder(&weak, &dirs_rx, &scope, &events))
            .map_err(notify::Error::io)?;
        Ok(WatchGuard::new(watcher))
    }

    /// Whether `err` says the path is no longer there.
    fn gone(err: &notify::Error) -> bool {
        match &err.kind {
            notify::ErrorKind::PathNotFound => true,
            notify::ErrorKind::Io(io) => io.kind() == std::io::ErrorKind::NotFound,
            _ => false,
        }
    }

    /// Watches each new directory's accepted subtree, then forwards its event.
    fn adder(
        watcher: &Weak<Mutex<RecommendedWatcher>>,
        dirs: &mpsc::Receiver<PathBuf>,
        scope: &Scope,
        events: &UnboundedSender<()>,
    ) {
        while let Ok(dir) = dirs.recv() {
            let Some(watcher) = watcher.upgrade() else {
                return;
            };
            {
                let mut armed = crate::lock(&watcher);
                for sub in plan_from(&dir, &scope.root, scope.git_dir.as_deref(), &scope.ignore) {
                    // A directory gone again already is not an error worth a log.
                    let _ = armed.watch(&sub, RecursiveMode::NonRecursive);
                }
            }
            drop(watcher);
            let _ = events.send(());
        }
    }
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

#[cfg(test)]
mod plan_tests {
    //! Milestone 9.11 decision 18 (B12): what a Linux watch registers.

    use super::*;

    fn dirs(root: &Path, list: &[&str]) {
        for dir in list {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    fn relative(root: &Path, planned: Vec<PathBuf>) -> Vec<String> {
        let mut names: Vec<String> = planned
            .iter()
            .map(|path| {
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                if rel.is_empty() { ".".to_string() } else { rel }
            })
            .collect();
        names.sort();
        names
    }

    fn tree() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        dirs(
            &root,
            &[
                "src/a",
                "node_modules/p",
                "gen/q",
                ".git/refs/heads",
                ".git/objects/aa",
                ".git/lfs/x",
            ],
        );
        // `target/x/y` and 60 directories under it: none is entered.
        for i in 0..60 {
            dirs(&root, &[&format!("target/x/y/d{i}")]);
        }
        (tmp, root)
    }

    #[test]
    fn plan_skips_what_the_deny_list_rejects() {
        let (_tmp, root) = tree();
        let git_dir = root.join(".git");
        let planned = plan(&root, Some(&git_dir), &["gen".to_string()]);
        assert_eq!(
            relative(&root, planned),
            [".", ".git", ".git/refs", ".git/refs/heads", "src", "src/a"]
        );
    }

    #[test]
    fn plan_never_follows_a_link() {
        let (_tmp, root) = tree();
        let outside = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(outside.path().join("deep/er")).unwrap();
        std::os::unix::fs::symlink(outside.path(), root.join("src/out")).unwrap();
        let planned = plan(&root, Some(&root.join(".git")), &["gen".to_string()]);
        assert_eq!(
            relative(&root, planned),
            [".", ".git", ".git/refs", ".git/refs/heads", "src", "src/a"]
        );
    }

    /// Linux only (inotify). The watch on a directory created after `build` is added
    /// before the event of its creation is forwarded, so a file written in it after
    /// that event is seen. Bound: the watcher forwards each accepted event at once (no
    /// debounce of its own); `DEBOUNCE` (the scheduler's, 300 ms) plus a 2 s wall-clock
    /// slack for a loaded runner covers the inotify delivery and the notify thread.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_new_directory_is_watched_when_created() {
        use std::time::Duration;
        const WALL_CLOCK_SLACK: Duration = Duration::from_secs(2);
        let bound = crate::git::schedule::DEBOUNCE + WALL_CLOCK_SLACK;
        let (_tmp, root) = tree();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let _guard = build(&root, Some(&root.join(".git")), &[], tx).unwrap();
        let wait = |rx: &mut tokio::sync::mpsc::UnboundedReceiver<()>, what: &str| {
            let deadline = std::time::Instant::now() + bound;
            loop {
                if rx.try_recv().is_ok() {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "no event for {what} within {bound:?}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        std::fs::create_dir(root.join("src/b")).unwrap();
        wait(&mut rx, "src/b's creation");
        while rx.try_recv().is_ok() {}
        std::fs::write(root.join("src/b/f"), "f").unwrap();
        wait(&mut rx, "a file written in src/b");
    }
}
