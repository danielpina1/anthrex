//! The pool's blocking file-system steps (milestone 9.14 decisions 3, 12, 13 and 14):
//! loading and saving `leases.json`, preparing, wiping and deleting pool directories,
//! and the scrub that keeps "`Fresh` means empty" true (task-4 review M7). Blocking:
//! call only from `spawn_blocking` (AGENTS.md rule 2).

use std::collections::BTreeSet;
use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{
    DirId, DirState, LEASES_VERSION, LeasesFile, POOL_LEASES_FILE, POOL_TRASH_DIR, PoolBook,
    PoolClass,
};

/// Distinguishes trash names made in the same second.
static TRASH_SEQ: AtomicU64 = AtomicU64::new(0);

/// `task-<k>` or `engine-<k>` as a [`DirId`]; `None` for any other name.
pub(super) fn parse_name(name: &str) -> Option<DirId> {
    let (class, k) = if let Some(k) = name.strip_prefix("task-") {
        (PoolClass::Task, k)
    } else {
        (PoolClass::Engine, name.strip_prefix("engine-")?)
    };
    if k.starts_with('0') || !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let k = k.parse().ok().filter(|k| *k > 0)?;
    Some(DirId { class, k })
}

/// The book `root/leases.json` records, sized `task_dirs` and `engine_dirs`, then
/// scrubbed. A missing file is an empty pool; an unreadable one, or one of another
/// `version` (review M3), makes every directory `Dirty`. Duplicate entries are dropped,
/// keeping the first (review M4).
pub(super) fn load(root: &Path, task_dirs: u16, engine_dirs: u16) -> PoolBook {
    let path = root.join(POOL_LEASES_FILE);
    let book = match std::fs::read(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            PoolBook::new(task_dirs, engine_dirs)
        }
        Err(error) => {
            tracing::warn!(?error, path = %path.display(), "target pool: unreadable table");
            all_dirty(task_dirs, engine_dirs)
        }
        Ok(bytes) => match serde_json::from_slice::<LeasesFile>(&bytes) {
            Ok(file) if file.version == LEASES_VERSION => {
                PoolBook::from_file(&dedup(file, &path), task_dirs, engine_dirs)
            }
            Ok(file) => {
                tracing::warn!(version = file.version, path = %path.display(),
                    "target pool: a table of another version; every directory is dirty");
                all_dirty(task_dirs, engine_dirs)
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "target pool: unreadable table");
                all_dirty(task_dirs, engine_dirs)
            }
        },
    };
    for name in scrub_candidates(root, &book) {
        if let Err(error) = scrub(root, &name, &book) {
            tracing::warn!(?error, name, "target pool: could not scrub a directory");
        }
    }
    book
}

fn all_dirty(task_dirs: u16, engine_dirs: u16) -> PoolBook {
    let mut file = PoolBook::new(task_dirs, engine_dirs).to_file();
    for dir in &mut file.dirs {
        dir.state = DirState::Dirty;
    }
    PoolBook::from_file(&file, task_dirs, engine_dirs)
}

/// Review M4: the first entry of each directory, and the first leased directory of each
/// owner, are kept; every later one is dropped (the scrub then wipes what it held).
fn dedup(mut file: LeasesFile, path: &Path) -> LeasesFile {
    let mut ids = BTreeSet::new();
    let mut owners = BTreeSet::new();
    file.dirs.retain(|dir| {
        let fresh_id = ids.insert((dir.class, dir.k));
        let fresh_owner = dir.state != DirState::Leased
            || dir.owner.as_ref().is_none_or(|o| owners.insert(o.clone()));
        let keep = fresh_id && fresh_owner;
        if !keep {
            tracing::warn!(class = ?dir.class, k = dir.k, path = %path.display(),
                "target pool: dropped a duplicate table entry");
        }
        keep
    });
    file
}

/// Writes `file` as `root/leases.json`, atomically and durably (decision 14).
pub(super) fn save(root: &Path, file: &LeasesFile) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(file).map_err(io::Error::other)?;
    crate::profile::store::write_atomic(&root.join(POOL_LEASES_FILE), &bytes)
}

/// The pool directory `root/<name>`, a real directory spelled with `root` resolved
/// (decision 3): created when missing, wiped when `wipe_first`, and replaced when it is
/// a link or not a directory (never followed).
pub(super) fn prepare(root: &Path, name: &str, wipe_first: bool) -> io::Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    let dir = root.join(name);
    match std::fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => make_dir(&dir)?,
        Err(error) => return Err(error),
        Ok(meta) if wipe_first || meta.file_type().is_symlink() || !meta.is_dir() => {
            wipe(&root, name)?
        }
        Ok(_) => {}
    }
    Ok(dir)
}

fn make_dir(dir: &Path) -> io::Result<()> {
    DirBuilder::new().mode(0o700).create(dir)
}

/// Decision 12: renames `root/<name>` into `trash/`, recreates it empty (mode 0700), and
/// deletes the trash on a background thread.
pub(super) fn wipe(root: &Path, name: &str) -> io::Result<()> {
    remove(root, name)?;
    make_dir(&root.join(name))
}

/// Renames `root/<name>` into `trash/` (when it exists) and deletes it in the background.
pub(super) fn remove(root: &Path, name: &str) -> io::Result<()> {
    let dir = root.join(name);
    match std::fs::symlink_metadata(&dir) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    let trash = trash_dir(root)?;
    let seq = TRASH_SEQ.fetch_add(1, Ordering::Relaxed);
    let target = trash.join(format!("{name}-{}-{seq}", crate::run::driver::unix_now()));
    std::fs::rename(&dir, &target)?;
    std::thread::spawn(move || delete(&target));
    Ok(())
}

/// Deletes `path` without following a link: a link or a file is unlinked, a directory
/// removed with what it holds.
fn delete(path: &Path) {
    let result = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        tracing::debug!(?error, path = %path.display(), "target pool: could not delete trash");
    }
}

/// `root/trash/` as a plain directory, made when missing; anything else there (a link,
/// a file) is unlinked first, never followed (task-5 review M1).
fn trash_dir(root: &Path) -> io::Result<PathBuf> {
    let trash = root.join(POOL_TRASH_DIR);
    match std::fs::symlink_metadata(&trash) {
        Ok(meta) if meta.is_dir() => return Ok(trash),
        Ok(_) => std::fs::remove_file(&trash)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    make_dir(&trash)?;
    Ok(trash)
}

/// Decision 12: deletes whatever `root/trash/` holds (a crash's leftovers). A `trash`
/// that is not a plain directory is unlinked, never listed (task-5 review M1).
pub(super) fn empty_trash(root: &Path) {
    let trash = root.join(POOL_TRASH_DIR);
    match std::fs::symlink_metadata(&trash) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return delete(&trash),
        Err(_) => return,
    }
    let Ok(entries) = std::fs::read_dir(&trash) else {
        return;
    };
    for entry in entries.flatten() {
        delete(&entry.path());
    }
}

/// Review M7: the pool directories on disk that the book does not know, or knows as
/// `Fresh` while they are not an empty plain directory.
pub(super) fn scrub_candidates(root: &Path, book: &PoolBook) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str().map(str::to_string))
        .filter(|name| parse_name(name).is_some_and(|id| needs_scrub(root, name, id, book)))
        .collect();
    names.sort();
    names
}

fn needs_scrub(root: &Path, name: &str, id: DirId, book: &PoolBook) -> bool {
    match book.dir(id).map(|d| d.state) {
        None => true,
        Some(DirState::Fresh) => !is_empty_plain_dir(&root.join(name)),
        Some(_) => false,
    }
}

fn is_empty_plain_dir(dir: &Path) -> bool {
    let plain = std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir());
    plain && std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

/// Deletes `root/<name>` when the book does not know it, wipes it when the book says
/// `Fresh`; anything else is left alone (re-checked by the caller under the book lock).
pub(super) fn scrub(root: &Path, name: &str, book: &PoolBook) -> io::Result<()> {
    let Some(id) = parse_name(name) else {
        return Ok(());
    };
    match book.dir(id).map(|d| d.state) {
        None => remove(root, name),
        Some(DirState::Fresh) if !is_empty_plain_dir(&root.join(name)) => wipe(root, name),
        Some(_) => Ok(()),
    }
}

/// The repository directories under `data_dir/repos/` that hold a pool (a plain
/// `target-pool/` directory), each spelled by [`resolve`] (task-5 review M8).
pub(super) fn pools_on_disk(data_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(resolve(data_dir).join("repos")) else {
        return Vec::new();
    };
    let mut repos: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| resolve(&entry.path()))
        .filter(|repo| {
            std::fs::symlink_metadata(repo.join(super::POOL_DIR_NAME)).is_ok_and(|m| m.is_dir())
        })
        .collect();
    repos.sort();
    repos
}

/// The owners of `owners` that do not exist (`lstat` fails).
pub(super) fn absent(owners: Vec<PathBuf>) -> BTreeSet<PathBuf> {
    owners
        .into_iter()
        .filter(|owner| std::fs::symlink_metadata(owner).is_err())
        .collect()
}

/// `path` resolved: its nearest existing ancestor resolved, with the rest of its
/// components appended (a repository directory may not exist yet at its first lease);
/// as given when nothing resolves. The one spelling the pool keys a repository by
/// (task-5 review M8) and compares `disabled_repos` on (C27).
pub(super) fn resolve(path: &Path) -> PathBuf {
    for ancestor in path.ancestors() {
        if let Ok(resolved) = ancestor.canonicalize() {
            let rest = path.strip_prefix(ancestor).unwrap_or(Path::new(""));
            return if rest.as_os_str().is_empty() {
                resolved
            } else {
                resolved.join(rest)
            };
        }
    }
    path.to_path_buf()
}
