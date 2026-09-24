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
//! refused. It also verifies `HEAD`, which a worker may write: it must name the
//! worktree's own branch or be detached, so no engine merge, reset or checkout can be
//! steered onto another branch (fix round 2, R1). The git directory is found once,
//! uniquely, comparing `<worktree>/.git` without following it (R2). And it verifies the
//! worktree's own branch ref, which a worker may also write: it must hold a commit, never
//! a symbolic ref or a symbolic link to another branch (fix round 4, S1).
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
    /// The only branch the worktree's `HEAD` may name (`refs/heads/…`), or `None` for a
    /// worktree that is always detached. A detached `HEAD` is always allowed: a
    /// rebase detaches it, and no engine git call writes a branch through it (fix round
    /// 2, R1).
    pub head: Option<String>,
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

/// Pins `worktree` to its git directory in `common_dir`, whose `HEAD` may name only
/// `head` (or be detached). A worktree already pinned keeps the git directory it was
/// pinned to: it is found once, from the repository's side, and never recomputed while
/// the daemon runs (fix round 2, R2). Otherwise it is found by [`find_git_dir`]; one that
/// cannot be found is pinned as broken. Blocking (reads the common directory); returns
/// the pin.
pub fn pin(common_dir: &Path, worktree: &Path, head: Option<&str>) -> Pin {
    let key_path = key(worktree);
    let common = key(common_dir);
    let head = head.map(str::to_string);
    let existing = crate::lock(&PINS).get(&key_path).cloned();
    let pin = match existing {
        Some(pin) if pin.broken.is_none() && pin.common_dir == common && pin.git_dir.is_dir() => {
            Pin { head, ..pin }
        }
        _ => match find_git_dir(common_dir, worktree) {
            Ok(git_dir) => Pin {
                git_dir,
                common_dir: common,
                head,
                broken: None,
            },
            Err(reason) => Pin {
                git_dir: PathBuf::new(),
                common_dir: common,
                head,
                broken: Some(reason),
            },
        },
    };
    crate::lock(&PINS).insert(key_path, pin.clone());
    pin
}

/// `<worktree>/.git` spelled without resolving `.git` itself: the worktree directory
/// canonical, `.git` appended. A worker can make `.git` a symlink; this never follows
/// it (fix round 2, R2).
fn dot_git(worktree: &Path) -> PathBuf {
    key(worktree).join(".git")
}

/// A path as written in a `gitdir` file, compared without resolving its last component.
fn lexical(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => key(parent).join(name),
        _ => path.to_path_buf(),
    }
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

/// Fix round 3: the one branch ref (`refs/heads/…`) an engine write in `dir` may move,
/// when `dir` is a pinned worktree on a branch of its own. Engine writes name it
/// explicitly, with a compare-and-swap old value, and never write a ref through `HEAD`.
pub fn own_ref(dir: &Path) -> Option<String> {
    pinned(dir).filter(|pin| pin.broken.is_none())?.head
}

/// The git directory `<common>/worktrees/<name>` whose `gitdir` file names
/// `<worktree>/.git`. Read from the repository, never from the worktree.
pub fn find_git_dir(common_dir: &Path, worktree: &Path) -> Result<PathBuf, String> {
    let common = key(common_dir);
    let own = dot_git(worktree);
    let admin = common.join("worktrees");
    let entries = std::fs::read_dir(&admin)
        .map_err(|err| format!("cannot read {}: {err}", admin.display()))?;
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(text) = std::fs::read_to_string(dir.join("gitdir")) else {
            continue;
        };
        if lexical(Path::new(text.trim_end_matches(['\n', '\r']))) == own {
            found.push(key(&dir));
        }
    }
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!(
            "{} is not a linked worktree of {}",
            worktree.display(),
            common.display()
        )),
        _ => Err(format!(
            "{} is named by more than one git directory of {}",
            worktree.display(),
            common.display()
        )),
    }
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
    if lexical(back) != dot_git(worktree) {
        return refused(format!("gitdir names {}", back.display()));
    }
    if pin.git_dir.join("config.worktree").exists() {
        return refused("it has a config.worktree".to_string());
    }
    check_head(pin).or_else(refused)?;
    check_own_ref(pin).map_err(|what| {
        format!(
            "refusing git in {}: {what}; the task's branch was tampered with",
            worktree.display()
        )
    })
}

/// Fix round 4, S1: the worktree's own branch ref, which a worker may write (a commit
/// moves it), holds a commit and nothing else. A worker could make it `ref:
/// refs/heads/main` or a symbolic link, and every engine read of the branch (the
/// hand-back's `onto`, the done check's `HEAD`, a salvage's parent) would then judge
/// another branch's tip, and a dereferencing write would move that branch. A missing
/// loose ref is fine: `pack-refs` moves it into `packed-refs`, which a worker cannot
/// write and which holds no symbolic ref.
fn check_own_ref(pin: &Pin) -> Result<(), String> {
    let Some(own) = pin.head.as_deref() else {
        return Ok(());
    };
    let file = pin.common_dir.join(own);
    let meta = match std::fs::symlink_metadata(&file) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(format!("its branch {own} cannot be read ({err})")),
    };
    if meta.file_type().is_symlink() {
        return Err(format!("its branch {own} is a symbolic link"));
    }
    if !meta.is_file() {
        return Err(format!("its branch {own} is not a plain file"));
    }
    let text = std::fs::read_to_string(&file)
        .map_err(|err| format!("its branch {own} cannot be read ({err})"))?;
    let text = text.trim_end_matches(['\n', '\r']);
    if let Some(named) = text.strip_prefix("ref:") {
        return Err(format!(
            "its branch {own} is a symbolic ref to {}",
            named.trim()
        ));
    }
    if is_sha(text) {
        Ok(())
    } else {
        Err(format!("its branch {own} holds {text:?}"))
    }
}

fn is_sha(text: &str) -> bool {
    (text.len() == 40 || text.len() == 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Fix round 2, R1: the worktree's `HEAD` names its own branch, or is detached. A
/// worker may write `HEAD` (a rebase needs it), so a `HEAD` naming any other branch,
/// the base branch included, would make the engine's own merge or reset write that
/// branch.
fn check_head(pin: &Pin) -> Result<(), String> {
    let file = pin.git_dir.join("HEAD");
    let meta = std::fs::symlink_metadata(&file)
        .map_err(|err| format!("its HEAD cannot be read ({err})"))?;
    if !meta.is_file() {
        return Err("its HEAD is not a plain file".to_string());
    }
    let text =
        std::fs::read_to_string(&file).map_err(|err| format!("its HEAD cannot be read ({err})"))?;
    let text = text.trim_end_matches(['\n', '\r']);
    match text.strip_prefix("ref: ") {
        Some(named) if Some(named) == pin.head.as_deref() => Ok(()),
        Some(named) => Err(format!(
            "its HEAD names {named}, not {}",
            pin.head.as_deref().unwrap_or("a detached commit")
        )),
        None if is_sha(text) => Ok(()),
        None => Err(format!("its HEAD holds {text:?}")),
    }
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
