//! Milestone 9.11 decision 14 (A14): before a task checkout is removed, the nested
//! repositories in it whose work its salvage cannot keep. The salvage leaves every
//! gitlink out of `add -A` (fix round 1, N1), and a nested repository's commits live
//! in its own object store, so removing the checkout would lose them; the caller keeps
//! the checkout instead.
//!
//! A nested repository is the worker's: its configuration may name any command (a
//! clean filter, an fsmonitor), so no git command here reads it. Its refs and `HEAD`
//! are read as plain files, and its working tree is compared through a probe git
//! directory the daemon writes. Blocking; call only from `spawn_blocking`, behind the
//! caller's [`super::super::GitQueue::write`].

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::{Git, failure, import, nul_fields, os};

/// Milestone 9.11 decision 14 (A14): the nested repositories in `worktree` whose work
/// its salvage cannot keep, as paths relative to it. The salvage leaves every gitlink
/// out of `add -A`, and a nested repository's commits live in its own object store, so
/// removing the checkout would lose them. The candidates are the index's gitlinks and
/// the untracked directories git reports as repositories (`ls-files --others` stops at
/// one and names it `<dir>/`). One holds work as [`probe`] decides: an unsaved commit
/// or a working tree that differs from its `HEAD` ([`nested_dirty`]). Anything the probe cannot read counts as work: keeping a
/// checkout loses nothing.
pub fn nested_work(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    // The index's gitlinks, each with the commit the checkout records for it.
    let staged = g.ok(worktree, &[os("ls-files"), os("-s"), os("-z")])?;
    let recorded: Vec<(String, String)> = nul_fields(&staged)
        .filter_map(|entry| entry.strip_prefix("160000 "))
        .filter_map(|rest| {
            let (id, path) = rest.split_once('\t')?;
            Some((path.to_string(), id.split(' ').next()?.to_string()))
        })
        .collect();
    let mut subs: Vec<String> = recorded.iter().map(|(path, _)| path.clone()).collect();
    let others = [
        os("ls-files"),
        os("-z"),
        os("--others"),
        os("--exclude-standard"),
    ];
    let listing = g.ok(worktree, &others)?;
    subs.extend(nul_fields(&listing).filter_map(|e| e.strip_suffix('/').map(str::to_string)));
    subs.sort_unstable();
    subs.dedup();
    let mut kept = Vec::new();
    for sub in subs {
        let gitlink = recorded
            .iter()
            .find(|(path, _)| *path == sub)
            .map(|(_, id)| id.as_str());
        if nested_holds_work(g, worktree, &worktree.join(&sub), gitlink) {
            kept.push(sub);
        }
    }
    Ok(kept)
}

/// Whether the nested repository at `dir` holds work (see [`nested_work`]). Read only
/// with `lstat` and plain reads, never through a link, and never by a git command that
/// reads its (worker-written) configuration.
fn nested_holds_work(g: Git<'_>, worktree: &Path, dir: &Path, gitlink: Option<&str>) -> bool {
    let dot = dir.join(".git");
    match std::fs::symlink_metadata(&dot) {
        // A gitlink with no repository: only an empty directory is nothing.
        Err(_) => std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()),
        // Fix round 1 (I3): an initialized submodule's `.git` file, naming a directory
        // under the checkout's own git directory, is probed there.
        Ok(meta) if meta.is_file() => match module_dir(worktree, dir, &dot) {
            Some(module) => probe(g, (worktree, dir), &module, gitlink),
            None => true,
        },
        // A link names a repository elsewhere: not probed.
        Ok(meta) if !meta.is_dir() => true,
        Ok(_) => probe(g, (worktree, dir), &dot, gitlink),
    }
}

/// The repository whose git directory is `dot` and whose working tree is `dir`
/// holds work (see [`nested_work`]): a commit named by its `HEAD` or one of its own
/// refs that the user's repository lacks, that is not a remote-tracking tip of it (its
/// remote has it) and not `gitlink` (the commit the checkout records for it), or a
/// working tree that differs from its `HEAD`.
fn probe(g: Git<'_>, (worktree, dir): (&Path, &Path), dot: &Path, gitlink: Option<&str>) -> bool {
    let Some((head, refs)) = nested_ids(dot) else {
        return true;
    };
    let upstream: Vec<&str> = (refs.iter())
        .filter(|(name, _)| name.starts_with("refs/remotes/"))
        .map(|(_, id)| id.as_str())
        .chain(gitlink)
        .collect();
    let mut ids: Vec<&str> = (refs.iter())
        .filter(|(name, _)| !name.starts_with("refs/remotes/"))
        .map(|(_, id)| id.as_str())
        .chain(head.as_deref())
        .filter(|id| !upstream.contains(id))
        .collect();
    ids.sort_unstable();
    ids.dedup();
    match import::missing(g, worktree, &ids) {
        Ok(missing) if missing.is_empty() => {}
        _ => return true,
    }
    nested_dirty(g, dir, dot, head.as_deref()).unwrap_or(true)
}

/// Fix round 1 (I3): the git directory a submodule's `.git` file at `dot` names
/// (`gitdir: <path>`, relative to `dir` or absolute), when it lies, lexically and with
/// no link in any component below it, under the git directory of the checkout
/// `worktree` (an engine pin's: no worker grant covers it on macOS, and the Linux grant
/// denies its `modules/`), or under a linked checkout's common directory's `modules/`.
/// `None` otherwise: the caller keeps the checkout.
fn module_dir(worktree: &Path, dir: &Path, dot: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let pin = crate::worktree::pinned::pinned(worktree).filter(|pin| pin.broken.is_none())?;
    let text = plain_text(dot)?;
    let named = Path::new(text.strip_prefix("gitdir:")?.trim());
    let base = dir.canonicalize().ok()?;
    let mut resolved = PathBuf::new();
    for component in base.join(named).components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => resolved.push(other),
        }
    }
    let roots = [
        Some(pin.git_dir.clone()),
        (!pin.standalone).then(|| pin.common_dir.join("modules")),
    ];
    let root = roots
        .into_iter()
        .flatten()
        .find(|root| resolved.starts_with(root))?;
    let mut at = root.clone();
    for component in resolved.strip_prefix(&root).ok()?.components() {
        at.push(component);
        let meta = std::fs::symlink_metadata(&at).ok()?;
        if !meta.is_dir() {
            return None;
        }
    }
    (at != root).then_some(at)
}

/// The cap on a ref, `HEAD`, `packed-refs`, `.git` file or `info/exclude` the probe
/// reads; one larger counts as work.
const TEXT_CAP: u64 = 64 * 1024;

/// The cap on a nested index the probe copies; one larger counts as work.
const INDEX_CAP: u64 = 256 * 1024 * 1024;

/// Fix round 1 (I4): `path` opened once, `O_RDONLY | O_NOFOLLOW | O_NONBLOCK |
/// O_CLOEXEC`, and kept only when the opened descriptor is a regular file: a link, a
/// FIFO or a device swapped in after any earlier look is refused, never followed,
/// waited on or read.
fn open_plain(path: &Path) -> Option<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    before_open(path);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    file.metadata().ok().filter(|meta| meta.is_file())?;
    Some(file)
}

/// A plain file's text, at most [`TEXT_CAP`], read through the one descriptor
/// [`open_plain`] gives; `None` for anything else.
fn plain_text(path: &Path) -> Option<String> {
    use std::io::Read;
    let mut text = String::new();
    open_plain(path)?
        .take(TEXT_CAP + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= TEXT_CAP).then_some(text)
}

/// Copies the nested git directory's `name` into the probe's, through [`open_plain`]
/// and under `cap`. A missing file is not copied; one that is not a plain file or is
/// over `cap` is an error (the probe then counts the repository as work).
fn copy_plain(from: &Path, to: &Path, cap: u64) -> Result<(), String> {
    use std::io::Read;
    if std::fs::symlink_metadata(from).is_err_and(|err| err.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(());
    }
    let file = open_plain(from).ok_or_else(|| format!("{} is not a plain file", from.display()))?;
    let mut out =
        std::fs::File::create(to).map_err(|err| format!("cannot write {}: {err}", to.display()))?;
    let copied = std::io::copy(&mut file.take(cap + 1), &mut out)
        .map_err(|err| format!("cannot copy {}: {err}", from.display()))?;
    match copied > cap {
        true => Err(format!("{} is over {cap} bytes", from.display())),
        false => Ok(()),
    }
}

/// A test seam: runs the test's hook, if any, just before a file is opened.
fn before_open(path: &Path) {
    #[cfg(test)]
    tests::BEFORE_OPEN.with(|hook| {
        if let Some(hook) = &*hook.borrow() {
            hook(path);
        }
    });
    let _ = path;
}

/// A ref's name and the commit id it holds.
type NamedId = (String, String);

/// The nested git directory `dot`'s `HEAD` commit (`None` when unborn) and every ref
/// (loose and packed) naming a commit id, as `(name, id)`. `None` when something is
/// not a plain file.
fn nested_ids(dot: &Path) -> Option<(Option<String>, Vec<NamedId>)> {
    let text = plain_text(&dot.join("HEAD"))?;
    let mut ids = Vec::new();
    let mut pending = vec![dot.join("refs")];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).ok()? {
            let entry = entry.ok()?;
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let text = plain_text(&entry.path())?;
                let id = text.trim().to_string();
                let name = entry.path().strip_prefix(dot).ok()?.display().to_string();
                ids.extend(import::is_id(&id).then_some((name, id)));
            } else {
                return None;
            }
        }
    }
    // Missing is no packed refs; there but unreadable as a plain file under the cap
    // (a FIFO, an oversized file) is not nothing: the caller counts it as work.
    let packed_path = dot.join("packed-refs");
    let packed = match std::fs::symlink_metadata(&packed_path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        _ => plain_text(&packed_path)?,
    };
    ids.extend(
        packed
            .lines()
            .filter(|line| !line.starts_with(['#', '^']))
            .filter_map(|line| line.split_once(' '))
            .filter(|(id, _)| import::is_id(id))
            .map(|(id, name)| (name.trim().to_string(), id.to_string())),
    );
    let head = match text.trim().strip_prefix("ref:") {
        // A branch's tip is among `ids` already; an unborn branch has none.
        Some(name) => {
            let name = name.trim();
            let loose = plain_text(&dot.join(name)).map(|t| t.trim().to_string());
            let packed = packed.lines().find_map(|line| {
                line.split_once(' ')
                    .filter(|(_, n)| *n == name)
                    .map(|(id, _)| id.to_string())
            });
            loose.or(packed).filter(|id| import::is_id(id))
        }
        None => Some(text.trim().to_string()).filter(|id| import::is_id(id)),
    };
    Some((head, ids))
}

/// Whether the nested repository's working tree `dir` differs from `head`: `status`
/// run against a probe git directory the daemon writes (its own config; `HEAD` at
/// `head`; the nested index, `info/exclude` and object store copied or named as its
/// alternate), so the nested repository's configuration is never read.
fn nested_dirty(g: Git<'_>, dir: &Path, dot: &Path, head: Option<&str>) -> Result<bool, String> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    use std::os::unix::fs::DirBuilderExt;
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let probe =
        super::super::tmp::ensure_root()?.join(format!("nested-{}-{seq}", std::process::id()));
    let failed = |err: std::io::Error| format!("cannot prepare {}: {err}", probe.display());
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&probe)
        .map_err(failed)?;
    let result = (|| {
        let sha256 = head.is_some_and(|id| id.len() == 64);
        let format = if sha256 {
            "1\n[extensions]\n\tobjectformat = sha256"
        } else {
            "0"
        };
        let files = [
            (
                "config",
                format!("[core]\n\tbare = false\n\trepositoryformatversion = {format}\n"),
            ),
            (
                "HEAD",
                head.map_or("ref: refs/heads/anthrex-probe\n".to_string(), |id| {
                    format!("{id}\n")
                }),
            ),
            (
                "objects/info/alternates",
                format!("{}\n", dot.join("objects").display()),
            ),
        ];
        std::fs::create_dir_all(probe.join("objects/info")).map_err(failed)?;
        std::fs::create_dir_all(probe.join("refs")).map_err(failed)?;
        for (name, text) in files {
            std::fs::write(probe.join(name), text).map_err(failed)?;
        }
        // The index and the excludes, copied through one descriptor each, capped.
        std::fs::create_dir_all(probe.join("info")).map_err(failed)?;
        copy_plain(&dot.join("index"), &probe.join("index"), INDEX_CAP)?;
        copy_plain(
            &dot.join("info/exclude"),
            &probe.join("info/exclude"),
            TEXT_CAP,
        )?;
        let mut git_dir = std::ffi::OsString::from("--git-dir=");
        git_dir.push(&probe);
        let mut work_tree = std::ffi::OsString::from("--work-tree=");
        work_tree.push(dir);
        let args = [
            git_dir.as_os_str(),
            work_tree.as_os_str(),
            os("-c"),
            os("core.commitGraph=false"),
            os("status"),
            os("--porcelain"),
            os("-z"),
            os("--untracked-files=normal"),
            os("--ignore-submodules=all"),
        ];
        let (output, kept) = g.read_head(dir, &args, 0)?;
        if !output.success {
            return Err(failure(&args, &output));
        }
        Ok(kept.total > 0)
    })();
    let _ = std::fs::remove_dir_all(&probe);
    result
}

#[cfg(test)]
mod tests {
    //! Fix round 1 (I4): an entry swapped between the probe's type check and its read
    //! (a FIFO, a file grown past the cap) never blocks or floods the daemon.

    use super::plain_text;
    use std::cell::RefCell;
    use std::path::Path;
    use std::time::Duration;

    type Hook = Box<dyn Fn(&Path)>;

    thread_local! {
        pub(super) static BEFORE_OPEN: RefCell<Option<Hook>> = const { RefCell::new(None) };
    }

    /// `plain_text(path)` on its own thread with `hook` installed, waited for 10 s.
    fn read_swapped(path: &Path, hook: impl Fn(&Path) + Send + 'static) -> Option<Option<String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let path = path.to_path_buf();
        std::thread::spawn(move || {
            BEFORE_OPEN.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
            let _ = tx.send(plain_text(&path));
        });
        rx.recv_timeout(Duration::from_secs(10)).ok()
    }

    fn mkfifo(path: &Path) {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: `c` is a valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    }

    #[test]
    fn a_fifo_swapped_in_is_refused_without_blocking() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("HEAD");
        std::fs::write(&path, "ref: refs/heads/main\n").unwrap();
        let read = read_swapped(&path, |p| {
            std::fs::remove_file(p).unwrap();
            mkfifo(p);
        });
        if read.is_none() {
            // Release the blocked reader before failing.
            let _ = std::fs::OpenOptions::new().write(true).open(&path);
        }
        assert_eq!(
            read,
            Some(None),
            "the read blocked on a FIFO or took it as text"
        );
    }

    #[test]
    fn a_file_grown_past_the_cap_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("packed-refs");
        std::fs::write(&path, "# pack-refs\n").unwrap();
        let read = read_swapped(&path, |p| {
            std::fs::write(p, vec![b'a'; 1024 * 1024]).unwrap();
        });
        let length = read.map(|text| text.map(|text| text.len()));
        assert_eq!(length, Some(None), "an oversized file was read");
    }
}
