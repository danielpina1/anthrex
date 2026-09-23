//! Run git operations (M8a.8): start preflight (decision 17), the project-settings scan
//! (decision 53), the protected-file listing (decision 56), the done check that splits a
//! task's changed paths (decisions 32, 55 and 56), and the commit count and diff a
//! fresh session is handed (decision 30). The done check itself is in [`done`], the
//! run, task and review worktrees in [`worktrees`], and the per-repository write queue
//! in [`queue`]. M8a.9 adds [`merge`] (the merge candidate, the compare-and-swap, the
//! hand-back and the ref guard, decisions 21 and 36) and [`salvage`] (salvage, removal,
//! branch deletion and accept, decision 20).
//!
//! Does I/O (design decision 2). Every function here is **blocking** — call it only from
//! `spawn_blocking` — and none is `async` except [`GitQueue::write`]. Every function
//! takes the git program first (`git: &OsStr`, so a test can pass a recording script)
//! and a per-command timeout last. Every git command goes through
//! [`crate::worktree::run_git_with_cap`], so AGENTS.md rules 10 and 11 hold by
//! construction: `-C <dir> --no-optional-locks` and a scrubbed environment on every
//! invocation. Every engine **write** also carries [`WRITE_FLAGS`] (decision 18).

mod done;
mod merge;
mod queue;
mod salvage;
mod worktrees;

pub use done::{DoneChecked, verify_done};
pub use merge::{
    ACCEPT_LIST_MAX, AcceptOutcome, CandidateStep, RefCheck, cas_update, commit_tree,
    commits_since, guard_refs, hand_back, materialize, merge_tree, read_ref, reattach,
};
pub use salvage::{
    ACCEPT_MERGE_TIMEOUT, accept, accept_with_merge_timeout, delete_branches, remove_worktree,
    salvage,
};

pub use queue::{GitQueue, LOCK_RETRY_DELAYS_MS};
pub use worktrees::{create_run_branch, lock_worktree, prepare_review, prepare_worktree};

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::contract::{REVIEW_DIFF_MAX, clamp_diff};
use super::globs::ProtectedMatcher;
use super::plan::Preflight;
use crate::project;
use crate::subprocess::HeadTail;
use crate::worktree::{GitOutput, run_git_head_tail, run_git_with_cap};

/// Decision 18: every engine write passes these ahead of its subcommand, so a user's
/// hooks or a signing pinentry can never hang a run.
pub const WRITE_FLAGS: [&str; 4] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "commit.gpgSign=false",
];

/// Decision 17: `merge-tree --write-tree` needs git 2.38.
const MIN_GIT: (u32, u32) = (2, 38);

/// The stdout cap for whole-tree listings, name lists and stats, which a real
/// repository can push far past `run_git`'s default 256 KiB. A patch is never read
/// under a cap: [`diff`] keeps only its head and tail (fix round 1, finding 1).
pub(crate) const LARGE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// Every diff, stat and name list ignores the user's diff configuration: no colour
/// (`color.ui=always`), no external diff driver, no textconv filter (a
/// `.gitattributes`-selected command could run, or stall, inside the diff). Fix round
/// 1, finding 6.
pub(crate) const DIFF_FLAGS: [&str; 3] = ["--no-color", "--no-ext-diff", "--no-textconv"];

/// A patch's `a/` and `b/` prefixes, whatever `diff.noprefix` or
/// `diff.mnemonicPrefix` say (`--default-prefix` needs git 2.41; runs need 2.38).
const PATCH_PREFIXES: [&str; 2] = ["--src-prefix=a/", "--dst-prefix=b/"];

/// The files Claude Code reads as project settings (decision 53).
const CLAUDE_SETTINGS: [&str; 2] = [".claude/settings.json", ".claude/settings.local.json"];
const CLAUDE_MCP: &str = ".mcp.json";

/// One git program with one per-command timeout: the deadline is taken afresh for
/// every command (decision 18, "a deadline of `now + git_timeout`").
#[derive(Clone, Copy)]
pub(crate) struct Git<'a> {
    program: &'a OsStr,
    timeout: Duration,
}

pub(crate) fn os(s: &str) -> &OsStr {
    OsStr::new(s)
}

impl<'a> Git<'a> {
    pub(crate) fn new(program: &'a OsStr, timeout: Duration) -> Self {
        Git { program, timeout }
    }

    /// A read of output that may be arbitrarily large: its first and last
    /// `REVIEW_DIFF_MAX` bytes (fix round 1, finding 1).
    fn head_tail(&self, dir: &Path, args: &[&OsStr]) -> Result<(GitOutput, HeadTail), String> {
        run_git_head_tail(
            self.program,
            dir,
            args,
            Instant::now() + self.timeout,
            REVIEW_DIFF_MAX,
            REVIEW_DIFF_MAX,
        )
        .map_err(|err| err.to_string())
    }

    /// A read whose stdout matters even when git exits non-zero (`merge-tree`'s
    /// conflict is exit 1), which the capped capture discards; also a read of output
    /// of any size of which only the start matters. The first `head_bytes` of stdout
    /// are kept and the rest is read and dropped, so it never fails over a cap; the
    /// returned [`HeadTail`] says how much there was.
    pub(crate) fn read_head(
        &self,
        dir: &Path,
        args: &[&OsStr],
        head_bytes: usize,
    ) -> Result<(GitOutput, HeadTail), String> {
        run_git_head_tail(
            self.program,
            dir,
            args,
            Instant::now() + self.timeout,
            head_bytes,
            0,
        )
        .map_err(|err| err.to_string())
    }

    fn raw(&self, dir: &Path, args: &[&OsStr], cap: usize) -> Result<GitOutput, String> {
        run_git_with_cap(self.program, dir, args, Instant::now() + self.timeout, cap)
            .map_err(|err| err.to_string())
    }

    /// A read whose failure the caller interprets.
    pub(crate) fn read(&self, dir: &Path, args: &[&OsStr]) -> Result<GitOutput, String> {
        self.raw(dir, args, LARGE_OUTPUT_BYTES)
    }

    /// A read that must succeed; its stdout.
    pub(crate) fn ok(&self, dir: &Path, args: &[&OsStr]) -> Result<String, String> {
        let output = self.read(dir, args)?;
        succeeded(args, output)
    }

    /// A write (decision 18's flags first) whose failure the caller interprets.
    pub(crate) fn write_raw(&self, dir: &Path, args: &[&OsStr]) -> Result<GitOutput, String> {
        let mut full: Vec<&OsStr> = WRITE_FLAGS.iter().map(|flag| os(flag)).collect();
        full.extend_from_slice(args);
        self.raw(dir, &full, LARGE_OUTPUT_BYTES)
    }

    /// A write into the user's own checkout (`run accept`), which decision 18 exempts
    /// from [`WRITE_FLAGS`]: their hooks and signing apply to their merge. The caller
    /// interprets its failure.
    pub(crate) fn user_write(&self, dir: &Path, args: &[&OsStr]) -> Result<GitOutput, String> {
        self.raw(dir, args, LARGE_OUTPUT_BYTES)
    }

    /// A write that must succeed; its stdout.
    pub(crate) fn write(&self, dir: &Path, args: &[&OsStr]) -> Result<String, String> {
        let output = self.write_raw(dir, args)?;
        succeeded(args, output)
    }
}

pub(crate) fn succeeded(args: &[&OsStr], output: GitOutput) -> Result<String, String> {
    if output.success {
        Ok(output.stdout)
    } else {
        Err(failure(args, &output))
    }
}

/// `git <args> failed: <stderr tail>`.
pub(crate) fn failure(args: &[&OsStr], output: &GitOutput) -> String {
    let joined = args
        .iter()
        .map(|arg| arg.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ");
    format!("git {joined} failed: {}", output.stderr_tail())
}

/// Decision 17's start preflight, in its order. `protected_files` is left empty: the
/// protected list depends on the resolved profile, which the caller has and passes to
/// [`protected_files`]; decision 53's project-settings refusal is likewise the
/// caller's, through [`project_settings`], only when it applies.
pub fn preflight(git: &OsStr, dir: &Path, timeout: Duration) -> Result<Preflight, String> {
    let roots = project::detect_roots_with(git, dir, timeout);
    let Some(root) = roots.worktree else {
        if !roots.detection_failed && is_bare(Git::new(git, timeout), dir) {
            return Err(format!(
                "anthrex runs need a working tree; {} is a bare repository",
                dir.display()
            ));
        }
        return Err(if roots.detection_failed {
            format!(
                "could not tell whether {} is a git repository: git did not answer; try again",
                dir.display()
            )
        } else {
            format!("not a git repository: {}", dir.display())
        });
    };
    let g = Git::new(git, timeout);

    let version = g.ok(&root, &[os("version")])?;
    check_version(&version)?;

    let head_ref = g.read(&root, &[os("symbolic-ref"), os("-q"), os("HEAD")])?;
    let base_branch = match head_ref.stdout.trim().strip_prefix("refs/heads/") {
        Some(branch) if head_ref.success && !branch.is_empty() => branch.to_string(),
        _ => {
            return Err(format!(
                "{} is on a detached HEAD; check out a branch first",
                root.display()
            ));
        }
    };

    let head = g.read(
        &root,
        &[
            os("rev-parse"),
            os("-q"),
            os("--verify"),
            os("HEAD^{commit}"),
        ],
    )?;
    if !head.success {
        return Err("the repository has no commits yet".to_string());
    }
    let base_sha = head.stdout.trim().to_string();

    // `user.useConfigOnly`: without it git invents an identity from the login name and
    // host name, and `git var` succeeds on a machine with nothing configured
    // (Implementation notes, M8a.8).
    let ident = g.read(
        &root,
        &[
            os("-c"),
            os("user.useConfigOnly=true"),
            os("var"),
            os("GIT_COMMITTER_IDENT"),
        ],
    )?;
    if !ident.success {
        return Err(format!(
            "git has no user.name/user.email configured for {}",
            root.display()
        ));
    }

    let status = g.ok(
        &root,
        &[os("status"), os("--porcelain"), os("--untracked-files=no")],
    )?;
    if !status.trim().is_empty() {
        return Err(format!(
            "the working tree at {} has uncommitted changes; commit or stash them first",
            root.display()
        ));
    }

    let common = g.ok(
        &root,
        &[
            os("rev-parse"),
            os("--path-format=absolute"),
            os("--git-common-dir"),
        ],
    )?;
    let common = PathBuf::from(common.trim_end_matches('\n'));
    let git_common_dir = common.canonicalize().unwrap_or(common);

    Ok(Preflight {
        root,
        project: roots.project,
        git_common_dir,
        base_branch,
        base_sha,
        protected_files: Vec::new(),
    })
}

/// Whether `dir` is inside a bare repository (fix round 1, finding 13); any failure
/// counts as no, leaving the plain "not a git repository" answer.
fn is_bare(g: Git<'_>, dir: &Path) -> bool {
    g.read(dir, &[os("rev-parse"), os("--is-bare-repository")])
        .is_ok_and(|output| output.success && output.stdout.trim() == "true")
}

/// `git version 2.50.1 (Apple Git-155)` → at least [`MIN_GIT`], else decision 17's
/// message naming the version token (or the whole line when there is none).
fn check_version(output: &str) -> Result<(), String> {
    let line = output.trim();
    let token = line
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .unwrap_or(line);
    let mut numbers = token.split('.').map(|part| part.parse::<u32>().ok());
    let found = match (numbers.next().flatten(), numbers.next().flatten()) {
        (Some(major), Some(minor)) => (major, minor),
        _ => (0, 0),
    };
    if found >= MIN_GIT {
        Ok(())
    } else {
        Err(format!(
            "anthrex runs need git 2.38 or newer for merge-tree --write-tree (found {token})"
        ))
    }
}

/// Tracked paths at `sha`, limited to `paths` when it is not empty (`ls-tree` matches
/// its path arguments literally, as path prefixes).
fn tracked_at(g: Git<'_>, root: &Path, sha: &str, paths: &[&str]) -> Result<Vec<String>, String> {
    let mut args = vec![
        os("ls-tree"),
        os("-r"),
        os("-z"),
        os("--name-only"),
        os(sha),
        os("--"),
    ];
    args.extend(paths.iter().map(|path| os(path)));
    let listing = g.ok(root, &args)?;
    Ok(nul_fields(&listing).map(str::to_string).collect())
}

pub(crate) fn nul_fields(text: &str) -> impl Iterator<Item = &str> {
    text.split('\0').filter(|field| !field.is_empty())
}

/// Decision 53: the project settings in the base commit's tree that a headless session
/// would load without asking. With `claude`, each tracked `.claude/settings.json` or
/// `.claude/settings.local.json` with a non-empty `hooks` key (or that is not valid
/// JSON, so cannot be shown to be hook-free) and a tracked `.mcp.json`; with
/// `codex_paths`, each of those paths that is tracked. Sorted by path.
pub fn project_settings(
    git: &OsStr,
    root: &Path,
    base_sha: &str,
    claude: bool,
    codex_paths: Option<&[&str]>,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let mut candidates: Vec<&str> = Vec::new();
    if claude {
        candidates.extend(CLAUDE_SETTINGS);
        candidates.push(CLAUDE_MCP);
    }
    if let Some(paths) = codex_paths {
        candidates.extend(paths.iter().copied());
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let g = Git::new(git, timeout);
    let mut found = Vec::new();
    for path in tracked_at(g, root, base_sha, &candidates)? {
        if !candidates.contains(&path.as_str()) {
            continue;
        }
        let codex = codex_paths.is_some_and(|paths| paths.contains(&path.as_str()));
        if claude && CLAUDE_SETTINGS.contains(&path.as_str()) && !codex {
            let blob = format!("{base_sha}:{path}");
            let text = g.ok(root, &[os("cat-file"), os("blob"), os(&blob)])?;
            if !has_hooks(&text) {
                continue;
            }
        }
        found.push(path);
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// A settings file has hooks unless it parses and its `hooks` key is absent, `null`,
/// or an empty object or array.
fn has_hooks(text: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(text) {
        Err(_) => true,
        Ok(value) => match value.get("hooks") {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Object(map)) => !map.is_empty(),
            Some(serde_json::Value::Array(items)) => !items.is_empty(),
            Some(_) => true,
        },
    }
}

/// Decision 56: the files tracked at `base_sha` that `protected` matches, in git's
/// path order.
pub fn protected_files(
    git: &OsStr,
    root: &Path,
    base_sha: &str,
    protected: &ProtectedMatcher,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    Ok(tracked_at(g, root, base_sha, &[])?
        .into_iter()
        .filter(|path| protected.matches(path))
        .collect())
}

/// The task's own commits (`git rev-list --count HEAD ^<start> ^<run_head>`, as
/// [`verify_done`] counts them) and `HEAD`: the turn-end fallback's question (decision
/// 32). With `run_head` excluded, a run head merged in by a hand-back is not counted
/// as the task's work (fix round 1, finding 10).
pub fn count_commits(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    run_head: &str,
    timeout: Duration,
) -> Result<(u32, String), String> {
    let g = Git::new(git, timeout);
    let not_start = format!("^{start}");
    let not_run_head = format!("^{run_head}");
    let count = g.ok(
        worktree,
        &[
            os("rev-list"),
            os("--count"),
            os("HEAD"),
            os(&not_start),
            os(&not_run_head),
        ],
    )?;
    let count = count
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("git rev-list --count printed {:?}", count.trim()))?;
    let head = g.ok(worktree, &[os("rev-parse"), os("HEAD")])?;
    Ok((count, head.trim().to_string()))
}

/// Decision 30's hand-over material: the stat and the diff of the task's net change,
/// clamped to [`REVIEW_DIFF_MAX`]. Uncommitted work is not in either. The range is
/// `<start>..HEAD` until a hand-back merges a newer run head in, then
/// `<run_head>..HEAD`: `<run_head>...HEAD` gives exactly that, since the merge base of
/// the run head and `HEAD` is the start commit or the merged run head (fix round 1,
/// finding 10).
pub fn diff_so_far(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    run_head: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let g = Git::new(git, timeout);
    // A run head that has moved on without a hand-back has `start` as its merge base
    // with `HEAD`; one that was merged in is its own. So `run_head...HEAD` never needs
    // `start`, which is kept for the op's record and to match `count_commits`.
    let _ = start;
    let range = format!("{run_head}...HEAD");
    let mut stat_args = vec![os("diff")];
    stat_args.extend(DIFF_FLAGS.map(os));
    stat_args.extend([os("--stat"), os(&range)]);
    let stat = g.ok(worktree, &stat_args)?;
    let patch = diff(g, worktree, &range)?;
    Ok((stat, patch))
}

/// `git diff <range>` with [`DIFF_FLAGS`] and [`PATCH_PREFIXES`], clamped to
/// [`REVIEW_DIFF_MAX`]. The output is streamed: only its first and last
/// `REVIEW_DIFF_MAX` bytes are kept, so a diff of any size is clamped, never failed
/// and never held in memory (fix round 1, finding 1). Keeping that much of each end is
/// enough, because [`clamp_diff`] reads at most half its budget of a longer text's
/// head and at most its budget of the tail.
pub(crate) fn diff(g: Git<'_>, dir: &Path, range: &str) -> Result<String, String> {
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.map(os));
    args.extend(PATCH_PREFIXES.map(os));
    args.push(os(range));
    let (output, kept) = g.head_tail(dir, &args)?;
    if !output.success {
        return Err(failure(&args, &output));
    }
    let text = if kept.dropped() {
        // Each end on its own: a character cut where the middle was dropped becomes a
        // replacement character only at the inner edge of a part, which `clamp_diff`
        // discards because the combined text is twice its limit.
        let mut text = String::from_utf8_lossy(&kept.head).into_owned();
        text.push_str(&String::from_utf8_lossy(&kept.tail));
        text
    } else {
        let mut bytes = kept.head;
        bytes.extend_from_slice(&kept.tail);
        String::from_utf8_lossy(&bytes).into_owned()
    };
    Ok(clamp_diff(&text, REVIEW_DIFF_MAX))
}
