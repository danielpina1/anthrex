//! Final fix batch F1c (re-review 4, concern 3a): a run's task, review and proof
//! checkouts are each **their own repository** in anthrex's data directory, not linked
//! worktrees of the user's repository. The user's `.git` gains no `worktrees/<id>`
//! entry per task, so their `git fetch`, `pull`, `gc`, `prune`, `fsck` and `log --all`
//! never walk a worker's `HEAD` whose objects only the worker's store holds.
//!
//! A checkout at `<wt>/runs/<run>/<name>` has its repository at
//! `<data>/runs/<run>/tasks/<name>/` ([`Repo`]):
//! - `git/`: the checkout's git directory. Its `objects/` is the worker's private
//!   object store, with the user's common object store as its only alternate; its
//!   `config`, written by the engine, includes the user's repository config (their
//!   identity, filters, hooks path), and turns reflogs and automatic gc off. It has no
//!   refs: the worker commits on a detached `HEAD`. The checkout's `.git` file names it.
//! - `engine/`: the engine's own, never in any grant: its copies of the index
//!   (`GIT_INDEX_FILE`, finding C1), the import's staging repository, and `ready`,
//!   written once the checkout's files are in place.
//!
//! The task's temporary directory (its worker's and its checks' `TMPDIR`) is not in
//! it: since final fix batch F1d it is a short directory under the daemon's own root
//! ([`super::tmp`]), so a Unix socket fits under it.
//!
//! The engine's git in the checkout (`--git-dir=<git> --work-tree=<checkout>`, pinned)
//! uses the common store as its object directory ([`crate::worktree::pinned::Pin`]'s
//! `standalone`), so it reads and writes only objects git wrote or verified on import;
//! every ref it names lives in the user's repository and is read or written there.
//!
//! Blocking; call only from `spawn_blocking`, behind the run's
//! [`super::GitQueue::write`].

use std::path::{Path, PathBuf};

use super::merge_state::put;
use super::sandbox::{engine_child, put_beneath};
use super::{Git, os};
use crate::worktree::pinned::{self, PinAs};

/// The marker, in [`Repo::engine`], that the checkout's files are in place.
const READY: &str = "ready";

/// Where a checkout's repository and the engine's own files live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    pub dir: PathBuf,
}

impl Repo {
    pub fn at(dir: &Path) -> Repo {
        Repo {
            dir: dir.to_path_buf(),
        }
    }

    /// The checkout's git directory.
    pub fn git_dir(&self) -> PathBuf {
        self.dir.join("git")
    }

    /// Its object directory: the worker's private store.
    pub fn objects(&self) -> PathBuf {
        self.git_dir().join("objects")
    }

    /// The engine's own directory.
    pub fn engine(&self) -> PathBuf {
        self.dir.join("engine")
    }

    /// The task's temporary directory: short, under the daemon's own root, never inside
    /// this directory (final fix batch F1d, [`super::tmp`]).
    pub fn tmp(&self) -> PathBuf {
        super::tmp::task_tmp(&self.dir)
    }

    /// Whether the checkout's files were put in place.
    pub fn ready(&self) -> bool {
        self.engine().join(READY).is_file()
    }
}

/// `<data>/runs/<run>/tasks/<name>` for the checkout `<wt>/runs/<run>/<name>`: the
/// checkout's name (`<task>`, `<task>.review`, `<task>.proof`) under the run's data
/// directory.
pub fn checkout_repo_dir(run_data_dir: &Path, checkout: &Path) -> PathBuf {
    let name = checkout.file_name().unwrap_or_default();
    run_data_dir.join("tasks").join(name)
}

/// For a caller that names no data directory (the tests' `prepare_worktree`): the
/// repository next to the checkout, `<parent>/.anthrex/<name>`.
pub fn default_repo_dir(checkout: &Path) -> PathBuf {
    let parent = checkout.parent().unwrap_or(Path::new("/"));
    parent
        .join(".anthrex")
        .join(checkout.file_name().unwrap_or_default())
}

/// What the checkout is: a task's (the worker writes it; its own branch, its objects,
/// the engine's own index), or a read-only one (a review, a proof).
pub(crate) enum Kind<'a> {
    Task { own: &'a str },
    ReadOnly,
}

/// The repository's config for the checkout at `path`. The user's repository config is
/// included (after a default hooks path, which it may override), and then what the
/// engine needs: not bare, the checkout as the work tree, no reflogs, no automatic gc.
/// The format settings come first: git reads a repository's format from its own file
/// only, never from an include.
fn config(common: &Path, path: &Path, sha256: bool) -> String {
    let mut text = String::from("[core]\n");
    if sha256 {
        text.push_str("\trepositoryformatversion = 1\n");
    } else {
        text.push_str("\trepositoryformatversion = 0\n");
    }
    text.push_str(&format!(
        "\thooksPath = {}\n[include]\n\tpath = {}\n[core]\n\tbare = false\n\tworktree = {}\n\tlogAllRefUpdates = false\n[gc]\n\tauto = 0\n",
        quoted(&common.join("hooks")),
        quoted(&common.join("config")),
        quoted(path),
    ));
    if sha256 {
        text.push_str("[extensions]\n\tobjectformat = sha256\n");
    }
    text
}

/// `path` as a git config value: quoted, with `"` and `\` escaped.
fn quoted(path: &Path) -> String {
    let text = path.display().to_string();
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The checkout at `path` and its repository `repo`, made or repaired, and pinned:
/// the repository's directories, config and alternates, `HEAD` at `at` when it has
/// none, the checkout's `.git` file, then (unless [`Repo::ready`] and the checkout is
/// there) its files and index from `HEAD`, and the ready marker last. Each step is
/// idempotent, so an interrupted one is finished by the next call (decision 43).
/// For a task, a `HEAD` holding a commit the worker made is imported first
/// ([`super::sync_in`]), so a checkout the user deleted comes back with its work.
/// Returns whether the files were (re)made now.
pub(crate) fn ensure(
    g: Git<'_>,
    common: &Path,
    path: &Path,
    repo: &Repo,
    at: &str,
    kind: Kind<'_>,
) -> Result<bool, String> {
    let failed =
        |what: &Path, err: std::io::Error| format!("cannot prepare {}: {err}", what.display());
    if !path.is_dir() {
        // The checkout is gone (the user deleted it, or it was never made): its files
        // are put back from `HEAD`.
        unready(repo)?;
    }
    std::fs::create_dir_all(&repo.dir).map_err(|err| failed(&repo.dir, err))?;
    let git_dir = engine_child(&repo.git_dir())?;
    let objects = engine_child(&repo.objects())?;
    let engine = engine_child(&repo.engine())?;
    // F1c round 3 (N2): `objects/` is writable by the worker and by confined commands,
    // so its subdirectories are made and checked like `objects` itself: a link planted
    // at `objects/info` is refused, never written through.
    engine_child(&objects.join("info"))?;
    engine_child(&objects.join("pack"))?;
    for dir in [
        git_dir.join("refs/heads"),
        git_dir.join("refs/tags"),
        git_dir.join("info"),
    ] {
        std::fs::create_dir_all(&dir).map_err(|err| failed(&dir, err))?;
    }
    if matches!(kind, Kind::Task { .. }) {
        engine_child(&repo.tmp())?;
    }
    std::fs::create_dir_all(path).map_err(|err| failed(path, err))?;
    let path = path.canonicalize().map_err(|err| failed(path, err))?;
    let sha256 = at.len() == 64;
    put(&git_dir, "config", config(common, &path, sha256).as_bytes())?;
    let alternates = format!("{}\n", common.join("objects").display());
    put_beneath(
        &git_dir,
        &["objects", "info"],
        "alternates",
        alternates.as_bytes(),
    )?;
    if let Ok(exclude) = std::fs::read(common.join("info/exclude")) {
        put(&git_dir.join("info"), "exclude", &exclude)?;
    }
    if std::fs::symlink_metadata(git_dir.join("HEAD")).is_err() {
        put(&git_dir, "HEAD", format!("{at}\n").as_bytes())?;
    }
    let dot_git = format!("gitdir: {}\n", git_dir.display());
    if std::fs::read_to_string(path.join(".git")).ok().as_deref() != Some(dot_git.as_str()) {
        if path.join(".git").is_dir() {
            std::fs::remove_dir_all(path.join(".git")).map_err(|err| failed(&path, err))?;
        }
        put(&path, ".git", dot_git.as_bytes())?;
    }
    let as_ = match kind {
        Kind::Task { own } => PinAs {
            head: None,
            own: Some(own.to_string()),
            objects: Some(objects.clone()),
            engine: Some(engine.clone()),
            repo: Some(git_dir.clone()),
        },
        Kind::ReadOnly => PinAs {
            repo: Some(git_dir.clone()),
            ..PinAs::default()
        },
    };
    let is_task = as_.own.is_some();
    if let Some(reason) = pinned::pin(common, &path, as_).broken {
        return Err(reason);
    }
    if repo.ready() {
        return Ok(false);
    }
    let head = if is_task {
        // F1c round 3 (N6): only a `HEAD` that names no commit (a symbolic ref, a
        // link, garbage) starts again from `at`; the worker's sandbox let it write no
        // ref, so nothing reachable is lost. Any other failure (an import that timed
        // out, a git that did not start) is returned, and the op is retried with the
        // worker's commits still in place.
        let pin = super::import::task_pin(&path)?;
        match super::import::head_file(&pin)? {
            super::import::HeadFile::Commit(_) => super::import::sync_in(g, &path)?,
            super::import::HeadFile::Symbolic(_) | super::import::HeadFile::Other(_) => {
                put(&git_dir, "HEAD", format!("{at}\n").as_bytes())?;
                at.to_string()
            }
        }
    } else {
        at.to_string()
    };
    g.write(
        &path,
        &[os("read-tree"), os("-u"), os("--reset"), os(&head)],
    )?;
    put(&engine, READY, b"")?;
    Ok(true)
}

/// Marks the checkout of `repo` as not in place, so the next [`ensure`] puts its files
/// back (its directory is gone, or it is about to be replaced).
pub(crate) fn unready(repo: &Repo) -> Result<(), String> {
    match std::fs::remove_file(repo.engine().join(READY)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(format!(
            "cannot remove {}: {err}",
            repo.engine().join(READY).display()
        )),
        _ => Ok(()),
    }
}

/// Removes the checkout `path` and its repository `repo`, and forgets its pin: after
/// its salvage, at the end of a task or a run. The checkout is removed only when the
/// engine pinned it as a standalone checkout of `repo`, so a path nobody made here is
/// never deleted; a link inside it is removed, never followed.
pub(crate) fn remove(path: &Path, repo: &Repo) -> Result<(), String> {
    let ours = pinned::pinned(path).is_some_and(|pin| {
        pin.standalone && pin.git_dir.parent() == repo.dir.canonicalize().ok().as_deref()
    });
    if path.exists() {
        if !ours {
            return Err(format!(
                "{} is not a checkout this daemon made; refusing to remove it",
                path.display()
            ));
        }
        unready(repo)?;
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
    }
    if repo.dir.exists() {
        std::fs::remove_dir_all(&repo.dir)
            .map_err(|err| format!("cannot remove {}: {err}", repo.dir.display()))?;
    }
    super::tmp::remove(&repo.dir)?;
    pinned::unpin(path);
    Ok(())
}
