//! The filesystem watcher for one worktree, and the pure path filter in front of it.
//!
//! Design decision 11: `notify`'s recommended watcher, recursive over the worktree
//! root plus non-recursive over that worktree's own git dir. Design decision 12: the
//! events it produces are filtered **by path only** — no gitignore parsing, because
//! parsing `.gitignore` correctly is a project of its own and getting it subtly wrong
//! would drop real edits.

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
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

/// Builds and arms the watcher for one root, sending one `()` per accepted event.
///
/// **Blocking**: on the inotify backend `watch(.., Recursive)` walks the tree, which
/// on a large checkout is real work, so the caller runs this on `spawn_blocking`. The
/// returned watcher must be kept alive — dropping it unwatches everything, which is
/// exactly how the registry's per-root task tears its watcher down when it is aborted.
///
/// Failure is not fatal (design decision 15): the caller logs it once and leaves the
/// root on the 30-second poll.
pub fn build(
    root: &Path,
    git_dir: Option<&Path>,
    events: UnboundedSender<()>,
) -> notify::Result<RecommendedWatcher> {
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
    Ok(watcher)
}
