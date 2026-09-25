//! Final fix batch F1b: a worker's commits reach the repository only through the
//! engine. A worker never writes the repository's git common directory: its git writes
//! objects to a private object directory (`GIT_OBJECT_DIRECTORY`, under the run's data
//! directory, with the common object store as a read-only alternate) and commits on a
//! detached `HEAD` in its worktree. Before the engine judges or merges a task's work it
//! [`sync`]s it: it reads the commit id in the worktree's `HEAD` file (worker-written, so
//! validated as a plain id), imports every object reachable from it that the common
//! store lacks, and moves the task's branch there by compare-and-swap.
//!
//! The import re-hashes every object. `pack-objects --revs` runs in an engine-owned bare
//! "staging" repository (next to the private directory, never writable by the worker)
//! whose `objects/info/alternates` names the private directory and the common store; the
//! pack it writes is fed to `index-pack --stdin --strict` in the repository, which
//! computes each object's id from its content and refuses a pack with a broken object
//! or a link to an object that is neither in the pack nor in the repository. So an
//! object in the private directory whose content does not match its name is never
//! trusted under that name, and no engine git command reads an object of the common
//! store that was not written by git itself or verified on the way in. The engine only
//! ever reads the private directory; it never writes there.
//!
//! Blocking; call only from `spawn_blocking`, behind the caller's
//! [`super::GitQueue::write`] (it writes objects and a ref).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::merge::read;
use super::{Git, failure, nul_fields, os};
use crate::worktree::pinned::{self, Pin};

/// Git configuration for the one engine git command that reads the private object
/// directory (`pack-objects`): no commit-graph, bitmap or multi-pack index, which the
/// worker could write there to make the walk lie (a lie can only drop objects, which
/// `index-pack --strict` then refuses, but there is no reason to read them at all).
const STAGING_FLAGS: [&str; 6] = [
    "-c",
    "core.commitGraph=false",
    "-c",
    "pack.useBitmaps=false",
    "-c",
    "core.multiPackIndex=false",
];

/// What a task worktree's `HEAD` file holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeadFile {
    /// A commit id (lower-case hex, 40 or 64 digits): the worker's detached `HEAD`.
    Commit(String),
    /// `ref: <name>`: a branch checked out, which a task worktree never has.
    Symbolic(String),
    /// Anything else (a link, a directory, garbage), with a description.
    Other(String),
}

/// A lower-case commit id of either object format.
pub(crate) fn is_id(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64)
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The pin of the task worktree `worktree`: one the engine made, not broken, with a
/// branch of its own.
pub(crate) fn task_pin(worktree: &Path) -> Result<Pin, String> {
    match pinned::pinned(worktree) {
        Some(pin) if pin.broken.is_some() => Err(pin.broken.unwrap_or_default()),
        Some(pin) if pin.own.is_some() => Ok(pin),
        _ => Err(format!(
            "{} is not an engine task worktree; refusing to import from it",
            worktree.display()
        )),
    }
}

/// The worktree's `HEAD`, read as a file from its pinned git directory (never through
/// a link, and never through git, which would follow a symbolic ref).
pub fn head_file(pin: &Pin) -> Result<HeadFile, String> {
    use std::io::Read as _;
    let file = pin.git_dir.join("HEAD");
    let meta = std::fs::symlink_metadata(&file)
        .map_err(|err| format!("{} cannot be read ({err})", file.display()))?;
    if !meta.is_file() {
        return Ok(HeadFile::Other("HEAD is not a plain file".to_string()));
    }
    let mut start = Vec::new();
    std::fs::File::open(&file)
        .and_then(|f| f.take(512).read_to_end(&mut start))
        .map_err(|err| format!("{} cannot be read ({err})", file.display()))?;
    let text = String::from_utf8_lossy(&start);
    let text = text.trim_end_matches(['\n', '\r']);
    if let Some(named) = text.strip_prefix("ref:") {
        return Ok(HeadFile::Symbolic(named.trim().to_string()));
    }
    Ok(if is_id(text) {
        HeadFile::Commit(text.to_string())
    } else {
        HeadFile::Other(format!("HEAD holds {text:?}"))
    })
}

/// Whether a rebase is stopped in the worktree (its `rebase-merge` or `rebase-apply`
/// directory exists): its `HEAD` is then an intermediate commit, not the worker's work.
pub fn rebase_in_progress(pin: &Pin) -> bool {
    ["rebase-merge", "rebase-apply"]
        .iter()
        .any(|dir| std::fs::symlink_metadata(pin.git_dir.join(dir)).is_ok())
}

/// [`sync_in`] with its own [`Git`]: the worker's `HEAD` imported and the task's
/// branch moved to it. Returns the commit.
pub fn sync(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<String, String> {
    sync_in(Git::new(git, timeout), worktree)
}

/// The worker's `HEAD` in the task worktree `worktree`, imported into the repository
/// (when the repository lacks it) and recorded on the task's branch by an `update-ref
/// --no-deref <own> <head> <old>` compare-and-swap. Refused when `HEAD` is not a plain
/// commit id. Idempotent: a `HEAD` already imported and recorded writes nothing.
pub(crate) fn sync_in(g: Git<'_>, worktree: &Path) -> Result<String, String> {
    let pin = task_pin(worktree)?;
    let own = pin.own.clone().unwrap_or_default();
    let head = match head_file(&pin)? {
        HeadFile::Commit(head) => head,
        HeadFile::Symbolic(named) => {
            return Err(format!(
                "{}'s HEAD names {named}; a task worktree works on a detached HEAD \
                 (git checkout --detach)",
                worktree.display()
            ));
        }
        HeadFile::Other(what) => {
            return Err(format!("{}'s {what}", worktree.display()));
        }
    };
    let old = read(g, worktree, &own)?;
    if !has_commit(g, worktree, &head)? {
        import_revs(g, worktree, &pin, &head, old.as_deref())?;
    }
    // The blobs the worker staged and has not committed, which the engine's `status`
    // reads to detect a rename (and a salvage commits). Best effort: an index naming an
    // object that cannot be imported only fails the engine command that needs it.
    let _ = import_index(g, worktree);
    if old.as_deref() != Some(head.as_str()) {
        let args = [
            os("update-ref"),
            os("--no-deref"),
            os(&own),
            os(&head),
            os(old.as_deref().unwrap_or("")),
        ];
        let output = g.write_raw(worktree, &args)?;
        if !output.success {
            return Err(format!(
                "{own} moved while the worker's HEAD was recorded on it: {}",
                failure(&args, &output)
            ));
        }
    }
    Ok(head)
}

/// Whether the repository (the common store; the engine's git never reads the private
/// directory) has `id` as a commit.
fn has_commit(g: Git<'_>, dir: &Path, id: &str) -> Result<bool, String> {
    let spec = format!("{id}^{{commit}}");
    Ok(g.read(dir, &[os("cat-file"), os("-e"), os(&spec)])?.success)
}

/// Imports the objects reachable from `head` and not from `old` (the task branch's
/// tip, whose objects the repository has) from the private directory, then checks the
/// repository has `head` and everything it reaches.
fn import_revs(
    g: Git<'_>,
    worktree: &Path,
    pin: &Pin,
    head: &str,
    old: Option<&str>,
) -> Result<(), String> {
    let Some(objects) = pin.objects.as_deref() else {
        return Err(format!(
            "the commit {head} in {}'s HEAD is not in the repository",
            worktree.display()
        ));
    };
    let mut revs = format!("{head}\n");
    if let Some(old) = old {
        revs.push_str(&format!("--not\n{old}\n"));
    }
    import(g, worktree, pin, objects, &revs, true)?;
    if !has_commit(g, worktree, head)? {
        return Err(format!(
            "the worker's commit {head} did not import: its object in the task's object \
             directory does not hash to its name"
        ));
    }
    let not = old.map(|old| format!("^{old}"));
    let mut args = vec![os("rev-list"), os("--objects"), os("--quiet"), os(head)];
    args.extend(not.as_deref().map(os));
    let output = g.read(worktree, &args)?;
    if !output.success {
        return Err(format!(
            "the worker's commit {head} imported incompletely: {}",
            failure(&args, &output)
        ));
    }
    Ok(())
}

/// Final fix batch F1b: the blobs a task worktree's index names that the repository
/// lacks (staged by the worker, so in its private directory only), imported, so a
/// salvage's `write-tree` can use them. Each is re-hashed; one whose content does not
/// match its name is refused.
pub(crate) fn import_index(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    let pin = task_pin(worktree)?;
    let listing = g.ok(worktree, &[os("ls-files"), os("-s"), os("-z")])?;
    let mut ids: Vec<&str> = nul_fields(&listing)
        .filter(|entry| !entry.starts_with("160000 "))
        .filter_map(|entry| entry.split(' ').nth(1))
        .filter(|id| is_id(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let missing = missing(g, worktree, &ids)?;
    if missing.is_empty() {
        return Ok(());
    }
    let Some(objects) = pin.objects.as_deref() else {
        return Err(format!(
            "{}'s index names {} object(s) the repository does not have",
            worktree.display(),
            missing.len()
        ));
    };
    let list: String = missing.iter().map(|id| format!("{id}\n")).collect();
    import(g, worktree, &pin, objects, &list, false)?;
    let still = missing_refs(g, worktree, &missing)?;
    if still.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} staged object(s) of {} did not import (first {}): their content does not \
             hash to their names",
            still.len(),
            worktree.display(),
            still[0]
        ))
    }
}

fn missing_refs(g: Git<'_>, dir: &Path, ids: &[String]) -> Result<Vec<String>, String> {
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    missing(g, dir, &ids)
}

/// Those of `ids` the repository does not have (`cat-file --batch-check`).
fn missing(g: Git<'_>, dir: &Path, ids: &[&str]) -> Result<Vec<String>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
    let args = [
        os("cat-file"),
        os("--batch-check=%(objectname)"),
        os("--buffer"),
    ];
    let out = g.read_input(dir, &args, input.as_bytes())?;
    Ok(out
        .lines()
        .filter_map(|line| line.strip_suffix(" missing"))
        .map(str::to_string)
        .collect())
}

/// The engine-owned staging repository for the private directory `objects`:
/// `<objects>/../staging.git`, bare, with no refs, whose alternates are `objects` and
/// the common store. Written afresh (its files by rename) before each import.
fn staging(pin: &Pin, objects: &Path, id_len: usize) -> Result<PathBuf, String> {
    let parent = objects
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", objects.display()))?;
    let dir = parent.join("staging.git");
    let failed = |err: std::io::Error| format!("cannot prepare {}: {err}", dir.display());
    std::fs::create_dir_all(dir.join("objects/info")).map_err(failed)?;
    std::fs::create_dir_all(dir.join("refs")).map_err(failed)?;
    let config = if id_len == 64 {
        "[core]\n\trepositoryformatversion = 1\n\tbare = true\n[extensions]\n\tobjectformat = sha256\n"
    } else {
        "[core]\n\trepositoryformatversion = 0\n\tbare = true\n"
    };
    let alternates = format!(
        "{}\n{}\n",
        objects.display(),
        pin.common_dir.join("objects").display()
    );
    for (name, content) in [
        ("HEAD", "ref: refs/heads/anthrex-staging\n"),
        ("config", config),
        ("objects/info/alternates", alternates.as_str()),
    ] {
        let temp = dir.join(format!("{name}.anthrex-tmp"));
        std::fs::write(&temp, content)
            .and_then(|()| std::fs::rename(&temp, dir.join(name)))
            .map_err(failed)?;
    }
    Ok(dir)
}

/// The private directory must be a real directory, reached through no symbolic link
/// the worker could have made: it (and its task directory) are the daemon's own, but
/// the worker's grant covers the directory itself, so it could replace it with a link.
fn check_private(objects: &Path) -> Result<(), String> {
    let meta = std::fs::symlink_metadata(objects)
        .map_err(|err| format!("{} cannot be read ({err})", objects.display()))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(format!(
            "{} is not a plain directory; the task's object directory was tampered with",
            objects.display()
        ));
    }
    let parent = objects.parent().and_then(|p| p.canonicalize().ok());
    let canonical = objects.canonicalize().ok();
    match (parent, canonical, objects.file_name()) {
        (Some(parent), Some(canonical), Some(name)) if parent.join(name) == canonical => {}
        _ => {
            return Err(format!(
                "{} does not resolve to itself; the task's object directory was tampered with",
                objects.display()
            ));
        }
    }
    // Git follows a directory's own alternates: the import would read whatever object
    // store the worker named there. Its git never writes one.
    for name in ["info/alternates", "info/http-alternates"] {
        if std::fs::symlink_metadata(objects.join(name)).is_ok() {
            return Err(format!(
                "{} has {name}, which the worker's git never writes; the task's object \
                 directory was tampered with",
                objects.display()
            ));
        }
    }
    Ok(())
}

/// `pack-objects` in the staging repository (reading the private directory and the
/// common store) of `input` (`--revs` input, or object ids), then `index-pack --stdin
/// --strict` of the pack in the repository.
fn import(
    g: Git<'_>,
    worktree: &Path,
    pin: &Pin,
    objects: &Path,
    input: &str,
    revs: bool,
) -> Result<(), String> {
    check_private(objects)?;
    let id_len = input.lines().next().map(str::len).unwrap_or(40);
    let staging = staging(pin, objects, id_len)?;
    clear_packs(&staging)?;
    let base = staging.join("import");
    let mut git_dir = std::ffi::OsString::from("--git-dir=");
    git_dir.push(&staging);
    let mut args: Vec<&OsStr> = vec![git_dir.as_os_str()];
    args.extend(STAGING_FLAGS.map(os));
    args.extend([os("pack-objects"), os("-q")]);
    if revs {
        args.push(os("--revs"));
    }
    args.push(base.as_os_str());
    let hash = g.write_input(&staging, &args, input.as_bytes())?;
    let pack = staging.join(format!("import-{}.pack", hash.trim()));
    let result = index_pack(g, worktree, &pack);
    clear_packs(&staging)?;
    result
}

/// `index-pack --stdin --strict` of `pack` in the repository of `worktree`, unless the
/// pack holds no object.
fn index_pack(g: Git<'_>, worktree: &Path, pack: &Path) -> Result<(), String> {
    use std::io::{Read as _, Seek as _};
    let failed = |err: std::io::Error| format!("cannot read {}: {err}", pack.display());
    let mut file = std::fs::File::open(pack).map_err(failed)?;
    let mut header = [0u8; 12];
    file.read_exact(&mut header).map_err(failed)?;
    if header[8..12] == [0, 0, 0, 0] {
        return Ok(());
    }
    file.rewind().map_err(failed)?;
    g.write_file(
        worktree,
        &[os("index-pack"), os("--stdin"), os("--strict")],
        &file,
    )
    .map(|_| ())
    .map_err(|err| format!("the worker's objects were refused on import: {err}"))
}

/// Removes the staging repository's pack files from an earlier import.
fn clear_packs(staging: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(staging)
        .map_err(|err| format!("cannot read {}: {err}", staging.display()))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("import-") {
            std::fs::remove_file(entry.path())
                .map_err(|err| format!("cannot remove {}: {err}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// `update-ref --no-deref HEAD <new> <old>` in the task worktree `worktree`: its
/// detached `HEAD` moved by the engine (a hand-back's merge commit, a re-point), only if
/// it still holds `old`. The file is replaced by rename, never written through.
pub(crate) fn move_head(g: Git<'_>, worktree: &Path, new: &str, old: &str) -> Result<(), String> {
    let args = [
        os("update-ref"),
        os("--no-deref"),
        os("HEAD"),
        os(new),
        os(old),
    ];
    let output = g.write_raw(worktree, &args)?;
    if output.success {
        Ok(())
    } else {
        Err(format!(
            "{}'s HEAD moved while the engine moved it: {}",
            worktree.display(),
            failure(&args, &output)
        ))
    }
}

/// Salvage (final fix batch F1b): a task worktree whose `HEAD` is not a detached commit
/// (the worker checked a branch out, or wrote something else there) has it put back at
/// the task branch's tip, so the salvage can run. Nothing is lost: the worker's sandbox
/// lets it write no branch, so no commit of its can be on one. The file is replaced by
/// rename, never written through. A detached `HEAD` is left as it is.
pub(crate) fn redetach(g: Git<'_>, worktree: &Path) -> Result<(), String> {
    let pin = task_pin(worktree)?;
    if matches!(head_file(&pin)?, HeadFile::Commit(_)) {
        return Ok(());
    }
    let own = pin.own.clone().unwrap_or_default();
    let tip = read(g, &pin.common_dir, &own)?.ok_or_else(|| format!("{own} does not exist"))?;
    super::merge_state::put(&pin.git_dir, "HEAD", format!("{tip}\n").as_bytes())
}
