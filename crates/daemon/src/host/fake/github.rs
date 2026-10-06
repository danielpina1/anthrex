//! Decision 14: `FakeGithub`, the state of the scripted GitHub, in `<dir>/github.json`,
//! read and written under a blocking exclusive `flock` on `<dir>/github.lock` so a test
//! process and the daemon share it; `<dir>/calls.jsonl` (every `gh` argv) and
//! `<dir>/forbidden.jsonl` (every ask to land something) are appended under the same
//! lock. The remote is real git: [`Bare`] reads and writes the local bare repository
//! through `worktree::run_git` (AGENTS.md rule 11). Blocking: only on a blocking thread.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::rules::{CiRule, CiRun};
use crate::host::{PrState, RepoPermission, ReviewState};
use crate::run::git::NO_HOOKS;
use crate::worktree::{self, GitOutput};

/// The login `gh` is authenticated as: every comment anthrex posts is by it.
pub const FAKE_LOGIN: &str = "tester";

const STATE: &str = "github.json";
const LOCK: &str = "github.lock";
pub(super) const CALLS: &str = "calls.jsonl";
pub(super) const FORBIDDEN: &str = "forbidden.jsonl";
const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const GIT_OUTPUT_MAX: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeRepo {
    pub owner: String,
    pub name: String,
    pub bare: PathBuf,
    pub default_branch: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeComment {
    pub id: u64,
    /// A login; one ending in `[bot]` is a bot (REST shows it so, GraphQL without it).
    pub user: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeReview {
    pub id: u64,
    pub user: String,
    pub state: ReviewState,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeThreadComment {
    pub id: u64,
    pub user: String,
    pub body: String,
    pub diff_hunk: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeThread {
    pub path: String,
    pub line: u32,
    pub resolved: bool,
    pub comments: Vec<FakeThreadComment>,
}

/// A comment anthrex posted through `gh`: in a thread (`thread` is its first comment's
/// id) or in the conversation (`None`). It is also in `comments` or the thread.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakeReply {
    pub id: u64,
    pub thread: Option<u64>,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FakePr {
    pub number: u64,
    pub base: String,
    pub head: String,
    /// The head branch's commit when last seen (kept once the branch is deleted).
    pub head_oid: String,
    pub title: String,
    pub body: String,
    pub state: PrState,
    pub merged_at: Option<u64>,
    pub merge_commit: Option<String>,
    pub comments: Vec<FakeComment>,
    pub reviews: Vec<FakeReview>,
    pub threads: Vec<FakeThread>,
    pub replies: Vec<FakeReply>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FakeGithub {
    pub repo: Option<FakeRepo>,
    pub logged_in: Vec<String>,
    pub permissions: BTreeMap<String, RepoPermission>,
    pub ci: Vec<CiRule>,
    /// How many runs each rule of `ci` has turned red (for `times`).
    pub ci_used: Vec<u32>,
    pub ci_runs: Vec<CiRun>,
    pub prs: Vec<FakePr>,
    pub last_pr: u64,
    pub last_id: u64,
    pub last_run: u64,
    pub rate_limited: u32,
}

/// Comment and review ids start above `u32::MAX`, as GitHub's issue comments already
/// are (`rest_issue_comments.json`: 5 093 042 634; M9.2.1 check 2).
const FIRST_ID: u64 = 5_000_000_000;
/// Actions run ids start where GitHub's are today (M9.2.1's fixtures).
const FIRST_RUN: u64 = 28_000_000_000;

impl FakeGithub {
    pub fn next_id(&mut self) -> u64 {
        self.last_id = self.last_id.max(FIRST_ID) + 1;
        self.last_id
    }

    pub fn next_run(&mut self) -> u64 {
        self.last_run = self.last_run.max(FIRST_RUN) + 1;
        self.last_run
    }

    pub fn pr(&self, number: u64) -> Option<&FakePr> {
        self.prs.iter().find(|p| p.number == number)
    }

    pub fn pr_mut(&mut self, number: u64) -> Option<&mut FakePr> {
        self.prs.iter_mut().find(|p| p.number == number)
    }

    /// `<owner>/<name>` names this repository (GitHub compares without case).
    pub fn is_repo(&self, full: &str) -> bool {
        self.repo.as_ref().is_some_and(|r| {
            full.split_once('/').is_some_and(|(o, n)| {
                o.eq_ignore_ascii_case(&r.owner) && n.eq_ignore_ascii_case(&r.name)
            })
        })
    }

    /// GitHub's own reaction to the bare repository's branches, run before every
    /// answer: an open PR's head follows its branch; when a merged PR's head branch is
    /// gone, the open PRs based on it are retargeted to its base (M9.2.1 check 5);
    /// then an open PR whose head or base branch is gone is closed. An open PR's
    /// `refs/pull/<n>/head` follows its head; a merged or closed one's keeps its last
    /// (FW-4: what anthrex reads when the merged branch is gone).
    pub fn refresh(&mut self) -> Result<(), String> {
        let Some(repo) = &self.repo else {
            return Ok(());
        };
        let bare = Bare::new(&repo.bare);
        let heads = bare.heads()?;
        let gone_merged: Vec<(String, String)> = self
            .prs
            .iter()
            .filter(|p| p.state == PrState::Merged && !heads.contains_key(&p.head))
            .map(|p| (p.head.clone(), p.base.clone()))
            .collect();
        for pr in self.prs.iter_mut().filter(|p| p.state == PrState::Open) {
            if let Some((_, base)) = gone_merged.iter().find(|(head, _)| *head == pr.base) {
                pr.base = base.clone();
            }
            match heads.get(&pr.head) {
                Some(oid) if heads.contains_key(&pr.base) => pr.head_oid = oid.clone(),
                _ => pr.state = PrState::Closed,
            }
        }
        let pulls = bare.pulls()?;
        for pr in self.prs.iter().filter(|p| p.state == PrState::Open) {
            if pulls.get(&pr.number) != Some(&pr.head_oid) {
                bare.set_pull(pr.number, &pr.head_oid)?;
            }
        }
        Ok(())
    }
}

/// Runs `change` on the state in `dir`, under the lock, and writes it back.
pub fn with_state<T>(dir: &Path, change: impl FnOnce(&mut FakeGithub) -> T) -> Result<T, String> {
    let _lock = Lock::take(dir)?;
    let path = dir.join(STATE);
    let mut state = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| format!("FakeGithub: {} is not valid: {e}", path.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => FakeGithub::default(),
        Err(e) => return Err(format!("FakeGithub: cannot read {}: {e}", path.display())),
    };
    let answer = change(&mut state);
    let bytes = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    let tmp = dir.join(format!("{STATE}.tmp"));
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, &path))
        .map_err(|e| format!("FakeGithub: cannot write {}: {e}", path.display()))?;
    Ok(answer)
}

/// Appends `argv` as one JSON line to `<dir>/<file>`, under the lock.
pub fn append(dir: &Path, file: &str, argv: &[String]) -> Result<(), String> {
    let _lock = Lock::take(dir)?;
    let mut line = serde_json::to_string(argv).map_err(|e| e.to_string())?;
    line.push('\n');
    let path = dir.join(file);
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut f| f.write_all(line.as_bytes()))
        .map_err(|e| format!("FakeGithub: cannot append to {}: {e}", path.display()))
}

/// Every line of `<dir>/<file>`; none when it does not exist. Read under the lock, so
/// a line being appended is never seen half written.
pub fn read_lines(dir: &Path, file: &str) -> Result<Vec<Vec<String>>, String> {
    let _lock = Lock::take(dir)?;
    let path = dir.join(file);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("FakeGithub: cannot read {}: {e}", path.display())),
    };
    text.lines()
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{}: {e}", path.display())))
        .collect()
}

/// The exclusive `flock` on `<dir>/github.lock`, held until dropped. Blocking (unlike
/// the daemon lock's `LOCK_NB`): the other side holds it only for one change.
struct Lock {
    _file: std::fs::File,
}

impl Lock {
    fn take(dir: &Path) -> Result<Lock, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("FakeGithub: cannot create {}: {e}", dir.display()))?;
        let path = dir.join(LOCK);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| format!("FakeGithub: cannot open {}: {e}", path.display()))?;
        loop {
            // SAFETY: `file` owns a live, open descriptor for the whole call.
            let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if rc == 0 {
                return Ok(Lock { _file: file });
            }
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(format!("FakeGithub: cannot lock {}: {err}", path.display()));
            }
        }
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `YYYY-MM-DDTHH:MM:SSZ`, as GitHub prints a time.
pub fn iso(unix: u64) -> String {
    crate::run::report::format_utc(unix).replacen(' ', "T", 1)
}

/// The bare repository GitHub "hosts": every read and write of it, through
/// `worktree::run_git` with hooks off.
pub struct Bare<'a> {
    dir: &'a Path,
}

impl<'a> Bare<'a> {
    pub fn new(dir: &'a Path) -> Self {
        Bare { dir }
    }

    pub fn run(&self, args: &[&str], input: Option<&[u8]>) -> Result<GitOutput, String> {
        let all: Vec<&OsStr> = NO_HOOKS
            .iter()
            .chain(args)
            .map(|a| OsStr::new(*a))
            .collect();
        let git = OsStr::new("git");
        let deadline = Instant::now() + GIT_TIMEOUT;
        let out = match input {
            None => worktree::run_git_with_cap(git, self.dir, &all, deadline, GIT_OUTPUT_MAX),
            Some(bytes) => worktree::run_git_with_input(git, self.dir, &all, deadline, bytes),
        };
        out.map_err(|e| format!("FakeGithub: git {}: {e}", args.join(" ")))
    }

    /// `git <args>` that must succeed: its stdout, trimmed.
    pub fn ok(&self, args: &[&str]) -> Result<String, String> {
        self.ok_input(args, None)
    }

    pub fn ok_input(&self, args: &[&str], input: Option<&[u8]>) -> Result<String, String> {
        let out = self.run(args, input)?;
        if out.success {
            Ok(out.stdout.trim().to_string())
        } else {
            Err(format!(
                "FakeGithub: git {}: {}",
                args.join(" "),
                out.stderr_tail()
            ))
        }
    }

    /// Every branch, by name, with its commit.
    pub fn heads(&self) -> Result<BTreeMap<String, String>, String> {
        let listed = self.ok(&[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/heads/",
        ])?;
        Ok(listed
            .lines()
            .filter_map(|l| {
                let (oid, name) = l.split_once(' ')?;
                Some((
                    name.strip_prefix("refs/heads/")?.to_string(),
                    oid.to_string(),
                ))
            })
            .collect())
    }

    /// Every `refs/pull/<n>/head`, by PR number, with its commit.
    pub fn pulls(&self) -> Result<BTreeMap<u64, String>, String> {
        let listed = self.ok(&[
            "for-each-ref",
            "--format=%(objectname) %(refname)",
            "refs/pull/",
        ])?;
        Ok(listed
            .lines()
            .filter_map(|l| {
                let (oid, name) = l.split_once(' ')?;
                let number = name.strip_prefix("refs/pull/")?.strip_suffix("/head")?;
                Some((number.parse().ok()?, oid.to_string()))
            })
            .collect())
    }

    /// GitHub's `refs/pull/<n>/head`, moved to `oid` (GitHub's own ref, not a branch).
    pub fn set_pull(&self, number: u64, oid: &str) -> Result<(), String> {
        let refname = format!("refs/pull/{number}/head");
        self.ok(&["update-ref", &refname, oid]).map(|_| ())
    }

    pub fn branch(&self, branch: &str) -> Result<Option<String>, String> {
        Ok(self.heads()?.remove(branch))
    }

    /// `path`'s text at `commit`, or `None` when it is not there.
    pub fn file(&self, commit: &str, path: &str) -> Result<Option<String>, String> {
        let out = self.run(&["cat-file", "blob", &format!("{commit}:{path}")], None)?;
        Ok(out.success.then_some(out.stdout))
    }

    /// `git merge-tree --write-tree`: the merged tree, or `None` on a conflict.
    pub fn merge_tree(
        &self,
        ours: &str,
        theirs: &str,
        base: Option<&str>,
    ) -> Result<Option<String>, String> {
        let merge_base = base.map(|b| format!("--merge-base={b}"));
        let mut args = vec!["merge-tree", "--write-tree", "--no-messages"];
        if let Some(b) = &merge_base {
            args.push(b);
        }
        args.extend([ours, theirs]);
        let out = self.run(&args, None)?;
        let tree = out.stdout.lines().next().unwrap_or("").trim().to_string();
        match (out.success, tree.is_empty()) {
            (true, false) => Ok(Some(tree)),
            (false, _) if out.stderr.trim().is_empty() => Ok(None),
            _ => Err(format!("FakeGithub: git merge-tree: {}", out.stderr_tail())),
        }
    }

    /// A commit of `tree` on `parents` by `who`, with `message`.
    pub fn commit_tree(
        &self,
        tree: &str,
        parents: &[&str],
        who: &str,
        message: &str,
    ) -> Result<String, String> {
        let name = format!("user.name={who}");
        let email = format!("user.email={who}@users.noreply.github.com");
        let mut args = vec!["-c", &name, "-c", &email, "-c", "commit.gpgSign=false"];
        args.extend(["commit-tree", tree]);
        for parent in parents {
            args.extend(["-p", parent]);
        }
        args.extend(["-m", message]);
        self.ok(&args)
    }

    /// Moves `branch` from `old` (`None`: it must not exist) to `new`; never a force.
    pub fn set_branch(&self, branch: &str, new: &str, old: Option<&str>) -> Result<(), String> {
        let refname = format!("refs/heads/{branch}");
        let zero = "0".repeat(new.len());
        self.ok(&["update-ref", &refname, new, old.unwrap_or(&zero)])
            .map(|_| ())
    }

    pub fn delete_branch(&self, branch: &str, old: &str) -> Result<(), String> {
        self.ok(&["update-ref", "-d", &format!("refs/heads/{branch}"), old])
            .map(|_| ())
    }
}
