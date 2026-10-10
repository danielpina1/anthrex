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

/// A plain file's text (at most 64 KiB), never read through a link.
fn plain_text(path: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    (meta.is_file() && meta.len() <= 64 * 1024).then(|| std::fs::read_to_string(path).ok())?
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
    let packed = plain_text(&dot.join("packed-refs")).unwrap_or_default();
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
        // The index and the excludes, copied when each is a plain file.
        std::fs::create_dir_all(probe.join("info")).map_err(failed)?;
        for name in ["index", "info/exclude"] {
            let from = dot.join(name);
            if std::fs::symlink_metadata(&from).is_ok_and(|meta| meta.is_file()) {
                std::fs::copy(&from, probe.join(name)).map_err(failed)?;
            }
        }
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
