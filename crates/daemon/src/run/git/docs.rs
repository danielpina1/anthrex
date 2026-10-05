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
//! symbolic link tracked in the run head's tree, in any case, is refused before
//! anything is written ([`docs_symlink`], which a design run's start asks too). The
//! commit's one parent is the run head; the branch moves by compare-and-swap, and a
//! branch already at a commit of the same tree and message on the run head (the same
//! commit sent again after a restart) is that commit. The integration worktree is then put back on
//! the branch, as a merge leaves it.
//!
//! Task M9.6.15 (decision 24): a later round's spec is appended to the file at its
//! path in the run head ([`DocsCommit::appends`], read with `cat-file`, nothing on
//! disk), and its commit is that round's first stage ([`DocsCommit::stage`]): the
//! stage branch is created at the commit and the branch (its `integration` alias)
//! moves to it, in one `update-ref --stdin` transaction.

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
/// A tree entry's mode for a folder.
const TREE_MODE: &str = "040000";

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
    /// Task M9.6.15: (repository path, bytes) appended after a blank line to the
    /// regular file at that path in `expected` (written whole when there is none).
    pub appends: &'a [(String, Vec<u8>)],
    /// Task M9.6.15: the stage branch created at the commit, with `branch` (its alias)
    /// moved to it from `expected`; `None` moves `branch` alone.
    pub stage: Option<&'a str>,
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
    let mut folders: Vec<String> = (docs.files.iter().chain(docs.appends))
        .filter_map(|(path, _)| path.rsplit_once('/').map(|(dir, _)| dir.to_string()))
        .collect();
    folders.dedup();
    if let Some(path) = walk(&c, docs.root, docs.expected, &folders)? {
        return Ok(DocsOutcome::Symlink { path });
    }
    let tree = build_tree(&c, docs)?;
    let refname = format!("refs/heads/{}", docs.branch);
    let head = match docs.stage {
        Some(stage) => on_stage(&c, docs, (&tree, &refname), stage)?,
        None => on_branch(&c, docs, &tree, &refname)?,
    };
    let checkout = ["checkout", "-q", "--force", docs.branch, "--"];
    let reattach = c.ok(docs.integration, Mode::Write, &checkout, None).err();
    Ok(DocsOutcome::Committed { head, reattach })
}

/// Ruling T12-1: the first tracked symbolic link on the way to any of `folders` in
/// `head`'s tree, by its path there; `None` when every component is a plain folder or
/// absent. The run's start asks it against the base head (`driver/design_commit.rs`),
/// and [`commit_docs`] against the run head. Ruling T12-3 (m3): components compare
/// ignoring case, so a `Docs` link is found for `docs` (with `core.ignorecase`, as on
/// macOS, a checkout would write through it), and every case variant is followed.
pub fn docs_symlink(
    git: &OsStr,
    root: &Path,
    head: &str,
    folders: &[String],
    timeout: Duration,
) -> Result<Option<String>, String> {
    let c = Call {
        git,
        timeout,
        index: Path::new(""),
    };
    walk(&c, root, head, folders)
}

/// [`docs_symlink`]'s walk: per folder, per component, the tree entries that match it
/// ignoring case, each tree listed once.
fn walk(
    c: &Call<'_>,
    root: &Path,
    head: &str,
    folders: &[String],
) -> Result<Option<String>, String> {
    let mut listed: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for folder in folders {
        // The trees reached so far, by their paths in `head`; the root first.
        let mut at = vec![String::new()];
        for part in folder.split('/') {
            let mut next = Vec::new();
            for tree in &at {
                let entries = match listed.iter().find(|(t, _)| t == tree) {
                    Some((_, entries)) => entries.clone(),
                    None => {
                        let entries = entries(c, root, head, tree)?;
                        listed.push((tree.clone(), entries.clone()));
                        entries
                    }
                };
                let part = part.to_lowercase();
                for (mode, name) in entries.iter().filter(|(_, n)| n.to_lowercase() == part) {
                    let path = match tree.is_empty() {
                        true => name.clone(),
                        false => format!("{tree}/{name}"),
                    };
                    match mode.as_str() {
                        SYMLINK_MODE => return Ok(Some(path)),
                        TREE_MODE => next.push(path),
                        _ => {}
                    }
                }
            }
            if next.is_empty() {
                // Not in the tree: nothing below it is either.
                break;
            }
            at = next;
        }
    }
    Ok(None)
}

/// The `(mode, name)` entries of the tree at `tree` (`""`: the root) in `head`.
fn entries(
    c: &Call<'_>,
    root: &Path,
    head: &str,
    tree: &str,
) -> Result<Vec<(String, String)>, String> {
    let at = match tree.is_empty() {
        true => head.to_string(),
        false => format!("{head}:{tree}"),
    };
    let listed = c.ok(root, Mode::Read, &["ls-tree", "-z", &at], None)?;
    let entry = |e: &str| {
        let (meta, name) = e.split_once('\t')?;
        Some((meta.split(' ').next()?.to_string(), name.to_string()))
    };
    Ok(listed.split('\0').filter_map(entry).collect())
}

/// The tree of the run head with the documents added, built in the caller's index,
/// which is removed before and after, whatever happened.
fn build_tree(c: &Call<'_>, docs: &DocsCommit<'_>) -> Result<String, String> {
    remove_index(docs.index)?;
    let built = (|| {
        let root = docs.root;
        c.ok(root, Mode::Indexed, &["read-tree", docs.expected], None)?;
        let appended = (docs.appends.iter())
            .map(|(path, tail)| Ok((path.clone(), appended(c, docs, path, tail)?)))
            .collect::<Result<Vec<_>, String>>()?;
        for (path, bytes) in docs.files.iter().chain(&appended) {
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

/// Task M9.6.15: the regular file at `path` in the run head (none: empty), a blank line,
/// then `tail`, ending with a newline.
fn appended(
    c: &Call<'_>,
    docs: &DocsCommit<'_>,
    path: &str,
    tail: &[u8],
) -> Result<Vec<u8>, String> {
    let listed = ["ls-tree", "-z", docs.expected, "--", path];
    let entry = c.ok(docs.root, Mode::Read, &listed, None)?;
    let regular = entry.starts_with("100644 ") || entry.starts_with("100755 ");
    let mut out = match regular {
        true => {
            let at = format!("{}:{path}", docs.expected);
            c.ok(docs.root, Mode::Read, &["cat-file", "blob", &at], None)?
                .into_bytes()
        }
        false => Vec::new(),
    };
    if !out.is_empty() {
        if !out.ends_with(b"\n") {
            out.push(b'\n');
        }
        out.push(b'\n');
    }
    out.extend_from_slice(tail);
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    Ok(out)
}

/// The commit of `tree` on `refname` (the run branch), moved from the run head by
/// compare-and-swap; a branch already elsewhere is this commit's when [`own`] says so.
fn on_branch(
    c: &Call<'_>,
    docs: &DocsCommit<'_>,
    tree: &str,
    refname: &str,
) -> Result<String, String> {
    let root = docs.root;
    match read_ref(c, root, refname)? {
        Some(at) if at == docs.expected => {
            let args = ["commit-tree", tree, "-p", docs.expected, "-m", docs.message];
            let commit = c.ok(root, Mode::Write, &args, None)?.trim().to_string();
            match swap(c, root, refname, &commit, docs.expected)? {
                None => Ok(commit),
                Some(now) => own(c, docs, tree, refname, &now),
            }
        }
        Some(now) => own(c, docs, tree, refname, &now),
        None => Err(format!("{refname} does not exist")),
    }
}

/// Task M9.6.15: the commit of `tree` as stage branch `stage`, created at it, with
/// `alias` (the run branch) moved to it from the run head, in one transaction. A stage
/// already there is this commit's when [`own`] says so (sent again after a restart),
/// and the alias then follows it.
fn on_stage(
    c: &Call<'_>,
    docs: &DocsCommit<'_>,
    (tree, alias): (&str, &str),
    stage: &str,
) -> Result<String, String> {
    let root = docs.root;
    let stage_ref = format!("refs/heads/{stage}");
    if let Some(now) = read_ref(c, root, &stage_ref)? {
        let head = own(c, docs, tree, &stage_ref, &now)?;
        follow(c, root, alias, (&head, docs.expected))?;
        return Ok(head);
    }
    match read_ref(c, root, alias)? {
        Some(at) if at == docs.expected => {}
        Some(now) => return Err(moved(alias, docs.expected, &now)),
        None => return Err(format!("{alias} does not exist")),
    }
    let args = ["commit-tree", tree, "-p", docs.expected, "-m", docs.message];
    let commit = c.ok(root, Mode::Write, &args, None)?.trim().to_string();
    let input = format!(
        "create {stage_ref} {commit}\nupdate {alias} {commit} {}\n",
        docs.expected
    );
    let args = ["update-ref", "--no-deref", "--stdin"];
    let output = c.run(root, Mode::Write, &args, Some(input.as_bytes()))?;
    if output.success {
        return Ok(commit);
    }
    match read_ref(c, root, &stage_ref)? {
        Some(now) => {
            let head = own(c, docs, tree, &stage_ref, &now)?;
            follow(c, root, alias, (&head, docs.expected))?;
            Ok(head)
        }
        None => Err(c.failure(Mode::Write, &args, &output)),
    }
}

/// `alias` at `head`: moved there from `expected` when it is still there.
fn follow(
    c: &Call<'_>,
    root: &Path,
    alias: &str,
    (head, expected): (&str, &str),
) -> Result<(), String> {
    match read_ref(c, root, alias)? {
        Some(at) if at == head => Ok(()),
        Some(at) if at == expected => match swap(c, root, alias, head, expected)? {
            Some(now) if now != head => Err(moved(alias, expected, &now)),
            _ => Ok(()),
        },
        Some(now) => Err(moved(alias, expected, &now)),
        None => Err(format!("{alias} does not exist")),
    }
}

fn moved(refname: &str, from: &str, to: &str) -> String {
    format!("{refname} moved from {} to {}", short(from), short(to))
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

/// The commit `refname` points at, `None` when it does not exist. Ruling T12-3 (m5):
/// `-q` keeps a missing ref quiet, so anything git says is an error.
fn read_ref(c: &Call<'_>, root: &Path, refname: &str) -> Result<Option<String>, String> {
    let wanted = format!("{refname}^{{commit}}");
    let args = ["rev-parse", "-q", "--verify", &wanted];
    let output = c.run(root, Mode::Read, &args, None)?;
    match (output.success, output.stderr.trim().is_empty()) {
        (true, _) => Ok(Some(output.stdout.trim().to_string())),
        (false, true) => Ok(None),
        (false, false) => Err(c.failure(Mode::Read, &args, &output)),
    }
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
/// head; the same tree; the same message, ruling T12-3 m2), or a branch someone else
/// moved.
fn own(
    c: &Call<'_>,
    docs: &DocsCommit<'_>,
    tree: &str,
    refname: &str,
    now: &str,
) -> Result<String, String> {
    let args = ["show", "-s", "--format=%T %P%n%B", now];
    let shown = c.ok(docs.root, Mode::Read, &args, None)?;
    let (first, message) = shown.split_once('\n').unwrap_or((shown.as_str(), ""));
    let mut fields = first.split_whitespace();
    let (t, parents) = (fields.next(), fields.collect::<Vec<_>>());
    let same = message.trim_end_matches('\n') == docs.message.trim_end_matches('\n');
    if t == Some(tree) && parents == [docs.expected] && same {
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
