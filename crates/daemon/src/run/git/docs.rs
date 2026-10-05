//! Milestone 9.6 decision 23 (DF §5.3, task M9.6.12): the documents commit's plumbing.
//! Blocking, as everything under `run/git/`; the driver runs it inside one
//! `GitQueue::write` (`driver/design_commit.rs`).
//!
//! Nothing goes through a working tree or the user's index. The tree is built in an
//! index file the driver owns (`GIT_INDEX_FILE`, set after the inherited one is
//! scrubbed, on the index commands only): `read-tree` of the run head, each document a
//! blob from `hash-object -w --stdin`, added with `update-index --add --cacheinfo`
//! (git's own `verify_path`), then `write-tree`. Every call carries
//! `core.protectHFS` and `core.protectNTFS`, so a path HFS+ or NTFS would read as
//! `.git` is refused by git itself; `mktree` is never used. A folder that goes through a
//! symbolic link tracked in the run head's tree is refused before anything is written.
//! The commit's one parent is the run head; the branch moves by compare-and-swap, and a
//! branch already at a commit of the same tree on the run head (the same commit sent
//! again after a restart) is that commit. The integration worktree is then put back on
//! the branch, as a merge leaves it.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use super::merge::short;
use super::{NO_HOOKS, WRITE_FLAGS, failure, os};
use crate::worktree::{GitOutput, run_git_with_index};

/// Passed on every call of the documents commit, after the read or write flags.
pub const PROTECT_FLAGS: [&str; 4] = ["-c", "core.protectHFS=true", "-c", "core.protectNTFS=true"];

/// A tree entry's mode for a symbolic link.
const SYMLINK_MODE: &str = "120000";

/// One documents commit: `files` (repository path, bytes) on top of `expected`'s tree,
/// committed with `message` on `branch`, whose checkout is `integration`; the tree is
/// built in `index`, a file the caller owns (removed before and after).
pub struct DocsCommit<'a> {
    pub root: &'a Path,
    pub index: &'a Path,
    pub branch: &'a str,
    pub expected: &'a str,
    pub integration: &'a Path,
    pub files: &'a [(String, Vec<u8>)],
    pub message: &'a str,
}

/// What a documents commit did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocsOutcome {
    /// The branch is at `head`; `reattach` is why the integration worktree could not be
    /// put back on it (the commit stands; the next candidate checks out over it).
    Committed {
        head: String,
        reattach: Option<String>,
    },
    /// `path`, a folder on the way to a document, is a tracked symbolic link.
    Symlink { path: String },
}

/// Decision 23's commit (module doc). An error is git's own text, or the branch that
/// moved.
pub fn commit_docs(
    git: &OsStr,
    docs: &DocsCommit<'_>,
    timeout: Duration,
) -> Result<DocsOutcome, String> {
    let c = Call {
        git,
        timeout,
        index: docs.index,
    };
    if let Some(path) = symlink_on_the_way(&c, docs)? {
        return Ok(DocsOutcome::Symlink { path });
    }
    let tree = build_tree(&c, docs)?;
    let root = docs.root;
    let refname = format!("refs/heads/{}", docs.branch);
    let head = match read_ref(&c, root, &refname)? {
        Some(at) if at == docs.expected => {
            let args = [
                "commit-tree",
                &tree,
                "-p",
                docs.expected,
                "-m",
                docs.message,
            ];
            let commit = c.ok(root, Mode::Write, &args, None)?.trim().to_string();
            match swap(&c, root, &refname, &commit, docs.expected)? {
                None => commit,
                Some(now) => own(&c, docs, &tree, &refname, &now)?,
            }
        }
        Some(now) => own(&c, docs, &tree, &refname, &now)?,
        None => return Err(format!("{refname} does not exist")),
    };
    let checkout = ["checkout", "-q", "--force", docs.branch, "--"];
    let reattach = c.ok(docs.integration, Mode::Write, &checkout, None).err();
    Ok(DocsOutcome::Committed { head, reattach })
}

/// The first folder on the way to a document that is a symbolic link in the run head's
/// tree (`ls-tree` per path component), in the documents' order.
fn symlink_on_the_way(c: &Call<'_>, docs: &DocsCommit<'_>) -> Result<Option<String>, String> {
    let mut seen: Vec<String> = Vec::new();
    for (path, _) in docs.files {
        let parts: Vec<&str> = path.split('/').collect();
        for end in 1..parts.len() {
            let folder = parts[..end].join("/");
            if seen.contains(&folder) {
                continue;
            }
            seen.push(folder.clone());
            let args = ["ls-tree", "-z", docs.expected, "--", &folder];
            let listed = c.ok(docs.root, Mode::Read, &args, None)?;
            let Some(entry) = listed.split('\0').find(|e| !e.is_empty()) else {
                // Not in the tree: nothing below it is either.
                break;
            };
            if entry.split(' ').next() == Some(SYMLINK_MODE) {
                return Ok(Some(folder));
            }
        }
    }
    Ok(None)
}

/// The tree of the run head with the documents added, built in the caller's index,
/// which is removed before and after, whatever happened.
fn build_tree(c: &Call<'_>, docs: &DocsCommit<'_>) -> Result<String, String> {
    remove_index(docs.index)?;
    let built = (|| {
        let root = docs.root;
        c.ok(root, Mode::Indexed, &["read-tree", docs.expected], None)?;
        for (path, bytes) in docs.files {
            let hashed = ["hash-object", "-w", "--stdin"];
            let blob = c.ok(root, Mode::Write, &hashed, Some(bytes))?;
            let blob = blob.trim();
            let add = ["update-index", "--add", "--cacheinfo", "100644", blob, path];
            c.ok(root, Mode::Indexed, &add, None)?;
        }
        c.ok(root, Mode::Indexed, &["write-tree"], None)
    })();
    let removed = remove_index(docs.index);
    let tree = built?;
    removed?;
    Ok(tree.trim().to_string())
}

/// The index and its lock gone, its folder there.
fn remove_index(index: &Path) -> Result<(), String> {
    if let Some(dir) = index.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let lock = index.with_extension("index.lock");
    for file in [index, lock.as_path()] {
        match std::fs::remove_file(file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(format!("{}: {e}", file.display()));
            }
            _ => {}
        }
    }
    Ok(())
}

/// The commit `refname` points at, `None` when it does not exist.
fn read_ref(c: &Call<'_>, root: &Path, refname: &str) -> Result<Option<String>, String> {
    let wanted = format!("{refname}^{{commit}}");
    let output = c.run(
        root,
        Mode::Read,
        &["rev-parse", "-q", "--verify", &wanted],
        None,
    )?;
    Ok(output.success.then(|| output.stdout.trim().to_string()))
}

/// `update-ref --no-deref <refname> <new> <old>`: `None` when it moved, else where the
/// branch is now.
fn swap(
    c: &Call<'_>,
    root: &Path,
    refname: &str,
    new: &str,
    old: &str,
) -> Result<Option<String>, String> {
    let args = ["update-ref", "--no-deref", refname, new, old];
    let output = c.run(root, Mode::Write, &args, None)?;
    if output.success {
        return Ok(None);
    }
    match read_ref(c, root, refname)? {
        Some(now) if now != old => Ok(Some(now)),
        _ => Err(c.failure(Mode::Write, &args, &output)),
    }
}

/// The branch is at `now`, not the run head: this commit's own (one parent, the run
/// head; the same tree), or a branch someone else moved.
fn own(
    c: &Call<'_>,
    docs: &DocsCommit<'_>,
    tree: &str,
    refname: &str,
    now: &str,
) -> Result<String, String> {
    let args = ["show", "-s", "--format=%T %P", now];
    let shown = c.ok(docs.root, Mode::Read, &args, None)?;
    let mut fields = shown.split_whitespace();
    let (t, parents) = (fields.next(), fields.collect::<Vec<_>>());
    if t == Some(tree) && parents == [docs.expected] {
        return Ok(now.to_string());
    }
    Err(format!(
        "{refname} moved from {} to {}",
        short(docs.expected),
        short(now)
    ))
}

/// How a call is flagged: a read, a write, or a write in the caller's index.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Write,
    Indexed,
}

struct Call<'a> {
    git: &'a OsStr,
    timeout: Duration,
    index: &'a Path,
}

impl Call<'_> {
    /// `args` with the mode's flags and [`PROTECT_FLAGS`] first.
    fn full<'b>(mode: Mode, args: &[&'b str]) -> Vec<&'b str> {
        let flags: &[&'static str] = match mode {
            Mode::Read => &NO_HOOKS,
            Mode::Write | Mode::Indexed => &WRITE_FLAGS,
        };
        let mut full: Vec<&str> = flags.iter().chain(&PROTECT_FLAGS).copied().collect();
        full.extend_from_slice(args);
        full
    }

    fn run(
        &self,
        dir: &Path,
        mode: Mode,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<GitOutput, String> {
        let full: Vec<&OsStr> = Self::full(mode, args).into_iter().map(os).collect();
        let index = (mode == Mode::Indexed).then_some(self.index);
        let deadline = Instant::now() + self.timeout;
        run_git_with_index(self.git, dir, &full, deadline, input, index).map_err(|e| e.to_string())
    }

    /// A call that must succeed; its stdout.
    fn ok(
        &self,
        dir: &Path,
        mode: Mode,
        args: &[&str],
        input: Option<&[u8]>,
    ) -> Result<String, String> {
        let output = self.run(dir, mode, args, input)?;
        match output.success {
            true => Ok(output.stdout),
            false => Err(self.failure(mode, args, &output)),
        }
    }

    fn failure(&self, mode: Mode, args: &[&str], output: &GitOutput) -> String {
        let full: Vec<&OsStr> = Self::full(mode, args).into_iter().map(os).collect();
        failure(&full, output)
    }
}
