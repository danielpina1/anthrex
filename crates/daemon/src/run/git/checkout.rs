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

use super::merge_state::{put, put_denied};
use super::sandbox::{engine_child, put_beneath};
use super::{Git, os};
use crate::worktree::pinned::{self, PinAs};

#[path = "checkout_access.rs"]
mod access;
pub(crate) use access::restore_owner_access;
pub use access::restore_owner_access_with;

/// The marker, in [`Repo::engine`], that the checkout's files are in place.
const READY: &str = "ready";

/// Milestone 9.11 decision 15 (A15): the file, in [`Repo::engine`], holding the
/// identity the user's repository resolves; the checkout's config includes it after
/// the repository config ([`config`]), by a path relative to the git directory.
///
/// Review m2: git treats an *unreadable* include target as fatal, so every git command
/// in the checkout, the worker's included, needs this file readable: no session's
/// `deny_read` may cover the run's data directory.
const IDENTITY: &str = "identity";

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
/// included (after a default hooks path, which it may override), then the identity the
/// user's repository resolves (`../engine/identity`, [`identity`]: an `includeIf
/// "gitdir:…"` in their config is evaluated against this git directory, where it never
/// matches, milestone 9.11 decision 15), and then what the engine needs: not bare, the
/// checkout as the work tree, no reflogs, no automatic gc. The format settings come
/// first: git reads a repository's format from its own file only, never from an
/// include.
pub(crate) fn config(common: &Path, path: &Path, sha256: bool) -> String {
    let mut text = String::from("[core]\n");
    if sha256 {
        text.push_str("\trepositoryformatversion = 1\n");
    } else {
        text.push_str("\trepositoryformatversion = 0\n");
    }
    text.push_str(&format!(
        "\thooksPath = {}\n[include]\n\tpath = {}\n\tpath = ../engine/{IDENTITY}\n[core]\n\tbare = false\n\tworktree = {}\n\tlogAllRefUpdates = false\n[gc]\n\tauto = 0\n",
        quoted(&common.join("hooks")),
        quoted(&common.join("config")),
        quoted(path),
    ));
    if sha256 {
        text.push_str("[extensions]\n\tobjectformat = sha256\n");
    }
    text
}

/// Milestone 9.11 fix round 1 (I1): the config text anthrex 0.1.0 to 0.1.2 wrote, frozen
/// here, for both object formats. A checkout made by those releases still holds it;
/// the pin check accepts it by exact equality, so an upgrade does not mark every
/// in-flight checkout as tampered with, and the next [`ensure`] rewrites it to
/// [`config`]. It differs from [`config`] only in lacking the identity include. Never
/// derive it from [`config`]: a later change there must not widen what is accepted.
pub(crate) fn config_v0_1(common: &Path, path: &Path, sha256: bool) -> String {
    let format = if sha256 { "1" } else { "0" };
    let mut text = format!(
        "[core]\n\trepositoryformatversion = {format}\n\thooksPath = {}\n[include]\n\tpath = {}\n[core]\n\tbare = false\n\tworktree = {}\n\tlogAllRefUpdates = false\n[gc]\n\tauto = 0\n",
        quoted(&common.join("hooks")),
        quoted(&common.join("config")),
        quoted(path),
    );
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

/// Milestone 9.11 decision 15 (A15): `user.name` and `user.email` as the user's
/// repository resolves them (`git config --get` in its git directory `common`, where
/// their `includeIf "gitdir:…"` matches), as a config file; a key git cannot resolve
/// is left out, so the checkout falls back to what its include gives, as before.
fn identity(g: Git<'_>, common: &Path) -> String {
    let mut text = String::new();
    for key in ["name", "email"] {
        let name = format!("user.{key}");
        let args = [os("config"), os("--get"), os(&name)];
        let value = match g.read(common, &args) {
            Ok(output) if output.success => {
                output.stdout.trim_end_matches(['\n', '\r']).to_string()
            }
            _ => continue,
        };
        if !value.is_empty() {
            text.push_str(&format!("\t{key} = {}\n", value_quoted(&value)));
        }
    }
    match text.is_empty() {
        true => text,
        false => format!("[user]\n{text}"),
    }
}

/// A config value quoted, with `\`, `"`, newline and tab escaped.
fn value_quoted(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
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
    // Review I1 of the Linux worker-git fix: every file here is staged in the engine
    // directory, never in the git directory a Linux worker's grant names whole. Re-review
    // N1: `config` and `info/exclude`, which a Linux grant denies, are never renamed
    // over (that would detach a live sandbox's read-only bind): unchanged, they are left
    // alone; changed, rewritten in place ([`put_denied`]).
    put_denied(
        &engine,
        &git_dir,
        "config",
        config(common, &path, sha256).as_bytes(),
    )?;
    put(&engine, &engine, IDENTITY, identity(g, common).as_bytes())?;
    let alternates = format!("{}\n", common.join("objects").display());
    put_beneath(
        &git_dir,
        &["objects", "info"],
        "alternates",
        alternates.as_bytes(),
    )?;
    if let Ok(exclude) = std::fs::read(common.join("info/exclude")) {
        put_denied(&engine, &git_dir.join("info"), "exclude", &exclude)?;
    }
    if std::fs::symlink_metadata(git_dir.join("HEAD")).is_err() {
        put(&engine, &git_dir, "HEAD", format!("{at}\n").as_bytes())?;
    }
    let dot_git = format!("gitdir: {}\n", git_dir.display());
    if std::fs::read_to_string(path.join(".git")).ok().as_deref() != Some(dot_git.as_str()) {
        if path.join(".git").is_dir() {
            std::fs::remove_dir_all(path.join(".git")).map_err(|err| failed(&path, err))?;
        }
        // The worktree may be on another filesystem than the engine directory; the
        // daemon's git never reads this file (it is pinned to `--git-dir`).
        put(&path, &path, ".git", dot_git.as_bytes())?;
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
                put(&engine, &git_dir, "HEAD", format!("{at}\n").as_bytes())?;
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
    put(&engine, &engine, READY, b"")?;
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
        restore_owner_access(path)?;
        std::fs::remove_dir_all(path)
            .map_err(|err| format!("cannot remove {}: {err}", path.display()))?;
    }
    removal_step("checkout");
    // The temporary directory goes before the repository: it is named only from the
    // repository's path ([`super::tmp::task_tmp`]), so while the repository is there a
    // test's teardown or the next daemon's reconcile can still find it if the daemon
    // stops between the two. The other order leaves it in the shared root for good.
    super::tmp::remove(&repo.dir)?;
    removal_step("temporary directory");
    if repo.dir.exists() {
        restore_owner_access(&repo.dir)?;
        std::fs::remove_dir_all(&repo.dir)
            .map_err(|err| format!("cannot remove {}: {err}", repo.dir.display()))?;
    }
    removal_step("repository");
    pinned::unpin(path);
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// The step of [`remove`] after which a test makes the daemon "stop".
    static STOP_AFTER: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
}

/// A point in [`remove`] at which the daemon's process may end (`anthrex daemon stop`,
/// a crash, a kill): its ops end with it, wherever they are. A test ends it there.
fn removal_step(_step: &'static str) {
    #[cfg(test)]
    if STOP_AFTER.get() == Some(_step) {
        panic!("the daemon stopped after removing the {_step}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    /// A daemon that stops in the middle of a checkout's removal leaves nothing in the
    /// shared temporary root that no directory names any more. The task temporary
    /// directory is named only from its repository's path (`tmp::task_tmp`), so a test's
    /// teardown, a smoke's and a restarted daemon's reconcile can find it only while
    /// that repository is still there: the CI-only `/tmp/ax-<uid>/<hash>` leak of PRs
    /// #61 and #65, a test daemon stopped between the two removals.
    #[test]
    fn a_stop_at_any_step_of_a_removal_leaves_a_findable_temporary_directory() {
        for step in ["checkout", "temporary directory", "repository"] {
            let base = super::super::tmp::TestDir::new();
            let dir = base.path().join("data/runs/r1/tasks/t1");
            std::fs::create_dir_all(dir.join("engine")).unwrap();
            let tmp = super::super::tmp::task_tmp(&dir);
            super::super::tmp::ensure_root().unwrap();
            std::fs::create_dir_all(tmp.join("s1-check")).unwrap();
            let checkout = base.path().join("wt/runs/r1/t1");

            super::STOP_AFTER.set(Some(step));
            let stopped =
                std::panic::catch_unwind(|| super::remove(&checkout, &super::Repo::at(&dir)));
            super::STOP_AFTER.set(None);
            assert!(stopped.is_err(), "the removal never reached the {step}");

            // What a test's teardown (or the next daemon) can still find and remove.
            super::super::tmp::remove_task_tmps_beneath(base.path()).unwrap();
            assert!(
                !tmp.exists(),
                "stopped after the {step}: {} is left, and nothing names it",
                tmp.display()
            );
        }
    }

    /// Fix round 2 (n6): the frozen 0.1.x text, pinned to the literal bytes anthrex
    /// 0.1.0 to 0.1.2 wrote, for both object formats.
    #[test]
    fn the_legacy_config_is_the_released_text() {
        let base = "[core]\n\trepositoryformatversion = {F}\n\thooksPath = \"/c/hooks\"\n[include]\n\tpath = \"/c/config\"\n[core]\n\tbare = false\n\tworktree = \"/p\"\n\tlogAllRefUpdates = false\n[gc]\n\tauto = 0\n";
        let (common, path) = (Path::new("/c"), Path::new("/p"));
        assert_eq!(
            super::config_v0_1(common, path, false),
            base.replace("{F}", "0")
        );
        assert_eq!(
            super::config_v0_1(common, path, true),
            base.replace("{F}", "1") + "[extensions]\n\tobjectformat = sha256\n"
        );
    }
}
