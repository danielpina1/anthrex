//! Pinned worktrees (M8a final fix batch F1, fix round 1, finding N2): the linked
//! worktrees the run engine created, each with the git directory the daemon found for it
//! in the repository's own `<common>/worktrees/*/gitdir` files. Every daemon git call in
//! a pinned worktree (through [`super::run_git`], and the probe) passes `--git-dir=<its
//! git dir> --work-tree=<it>`, so the worktree's `.git` file, which a sandboxed worker
//! can rewrite, is never read. Before each call, [`check`] verifies that the git dir's
//! `commondir` still names the repository's common directory, that its `gitdir` still
//! points back at the worktree, and that it holds no `config.worktree`; a worker cannot
//! write those files (its sandbox grant inside the git dir names only the files a commit
//! needs), so a mismatch means something else tampered with them, and the call is
//! refused.
//!
//! The registry is process-wide and tiny: a map behind [`crate::lock`], read and
//! written without I/O under the lock.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// A pinned worktree's git directory and its repository's common directory, both
/// canonical. `broken` is set when the daemon knew the worktree but could not find its
/// git directory: every call in it is then refused rather than trusting `.git`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pin {
    pub git_dir: PathBuf,
    pub common_dir: PathBuf,
    pub broken: Option<String>,
}

static PINS: LazyLock<Mutex<HashMap<PathBuf, Pin>>> = LazyLock::new(Default::default);

/// `path` canonical where it exists, else its canonical parent joined with its name.
fn key(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map(|parent| parent.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

/// Pins `worktree` to its git directory in `common_dir`, found by [`find_git_dir`]; a
/// worktree whose git directory cannot be found is pinned as broken. Blocking (reads the
/// common directory); returns the pin.
pub fn pin(common_dir: &Path, worktree: &Path) -> Pin {
    let pin = match find_git_dir(common_dir, worktree) {
        Ok(git_dir) => Pin {
            git_dir,
            common_dir: key(common_dir),
            broken: None,
        },
        Err(reason) => Pin {
            git_dir: PathBuf::new(),
            common_dir: key(common_dir),
            broken: Some(reason),
        },
    };
    crate::lock(&PINS).insert(key(worktree), pin.clone());
    pin
}

/// Forgets `worktree` (it was removed).
pub fn unpin(worktree: &Path) {
    crate::lock(&PINS).remove(&key(worktree));
}

/// The pin of `dir`, when `dir` is a pinned worktree itself.
pub fn pinned(dir: &Path) -> Option<Pin> {
    let key = key(dir);
    crate::lock(&PINS).get(&key).cloned()
}

/// The git directory `<common>/worktrees/<name>` whose `gitdir` file names
/// `<worktree>/.git`. Read from the repository, never from the worktree.
pub fn find_git_dir(common_dir: &Path, worktree: &Path) -> Result<PathBuf, String> {
    let common = key(common_dir);
    let own = key(&worktree.join(".git"));
    let admin = common.join("worktrees");
    let entries = std::fs::read_dir(&admin)
        .map_err(|err| format!("cannot read {}: {err}", admin.display()))?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = std::fs::read_to_string(dir.join("gitdir")) else {
            continue;
        };
        if key(Path::new(text.trim_end_matches(['\n', '\r']))) == own {
            return Ok(key(&dir));
        }
    }
    Err(format!(
        "{} is not a linked worktree of {}",
        worktree.display(),
        common.display()
    ))
}

/// Refuses a call in `worktree` when its git directory no longer belongs to it: its
/// `commondir` names another repository, its `gitdir` points elsewhere, or it has a
/// `config.worktree` (per-worktree config the engine never writes).
pub fn check(worktree: &Path, pin: &Pin) -> Result<(), String> {
    if let Some(reason) = &pin.broken {
        return Err(format!("refusing git in {}: {reason}", worktree.display()));
    }
    let refused = |what: String| {
        Err(format!(
            "refusing git in {}: {what}; the worktree's git directory {} was tampered with",
            worktree.display(),
            pin.git_dir.display()
        ))
    };
    let common = std::fs::read_to_string(pin.git_dir.join("commondir")).unwrap_or_default();
    let common = common.trim_end_matches(['\n', '\r']);
    let common = if Path::new(common).is_absolute() {
        PathBuf::from(common)
    } else {
        pin.git_dir.join(common)
    };
    if common.as_os_str().is_empty() || key(&common) != pin.common_dir {
        return refused(format!("commondir names {}", common.display()));
    }
    let back = std::fs::read_to_string(pin.git_dir.join("gitdir")).unwrap_or_default();
    let back = Path::new(back.trim_end_matches(['\n', '\r']));
    if key(back) != key(&worktree.join(".git")) {
        return refused(format!("gitdir names {}", back.display()));
    }
    if pin.git_dir.join("config.worktree").exists() {
        return refused("it has a config.worktree".to_string());
    }
    Ok(())
}

/// The flags that pin a call in `worktree`: `--git-dir=<git dir>` and
/// `--work-tree=<worktree>`.
pub fn flags(worktree: &Path, pin: &Pin) -> [std::ffi::OsString; 2] {
    let mut git_dir = std::ffi::OsString::from("--git-dir=");
    git_dir.push(&pin.git_dir);
    let mut work_tree = std::ffi::OsString::from("--work-tree=");
    work_tree.push(worktree);
    [git_dir, work_tree]
}
