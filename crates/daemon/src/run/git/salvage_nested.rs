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
use std::path::Path;
use std::time::Duration;

use super::super::{Git, failure, import, nul_fields, os};
use super::gitlinks;

/// Milestone 9.11 decision 14 (A14): the nested repositories in `worktree` whose work
/// its salvage cannot keep, as paths relative to it. The salvage leaves every gitlink
/// out of `add -A`, and a nested repository's commits live in its own object store, so
/// removing the checkout would lose them. The candidates are the index's gitlinks and
/// the untracked directories git reports as repositories (`ls-files --others` stops at
/// one and names it `<dir>/`). One holds work when it has a ref or a `HEAD` naming a
/// commit the user's repository lacks, or its working tree differs from its `HEAD`
/// ([`nested_dirty`]). Anything the probe cannot read counts as work: keeping a
/// checkout loses nothing.
pub fn nested_work(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    let mut subs = gitlinks(g, worktree)?;
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
        if nested_holds_work(g, worktree, &worktree.join(&sub)) {
            kept.push(sub);
        }
    }
    Ok(kept)
}

/// Whether the nested repository at `dir` holds work (see [`nested_work`]). Read only
/// with `lstat` and plain reads, never through a link, and never by a git command that
/// reads its (worker-written) configuration.
fn nested_holds_work(g: Git<'_>, worktree: &Path, dir: &Path) -> bool {
    let dot = dir.join(".git");
    match std::fs::symlink_metadata(&dot) {
        // A gitlink with no repository: only an empty directory is nothing.
        Err(_) => std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()),
        // A `.git` file or link names a repository elsewhere: not probed.
        Ok(meta) if !meta.is_dir() => true,
        Ok(_) => {
            let Some((head, mut ids)) = nested_ids(&dot) else {
                return true;
            };
            ids.extend(head.clone());
            let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
            match import::missing(g, worktree, &ids) {
                Ok(missing) if missing.is_empty() => {}
                _ => return true,
            }
            nested_dirty(g, dir, &dot, head.as_deref()).unwrap_or(true)
        }
    }
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

/// The nested git directory `dot`'s `HEAD` commit (`None` when unborn) and the commit
/// ids of every ref (loose and packed). `None` when something is not a plain file.
fn nested_ids(dot: &Path) -> Option<(Option<String>, Vec<String>)> {
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
                ids.extend(Some(text.trim().to_string()).filter(|id| import::is_id(id)));
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
            .filter_map(|line| line.split(' ').next())
            .filter(|id| import::is_id(id))
            .map(str::to_string),
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
