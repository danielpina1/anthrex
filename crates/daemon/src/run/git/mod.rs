//! Run git operations (M8a.8): start preflight (decision 17), the project-settings scan
//! (decision 53), the protected-file listing (decision 56), the done check that splits a
//! task's changed paths (decisions 32, 55 and 56), and the commit count and diff a
//! fresh session is handed (decision 30). The run, task and review worktrees are in
//! [`worktrees`]; the per-repository write queue is in [`queue`].
//!
//! Does I/O (design decision 2). Every function here is **blocking** — call it only from
//! `spawn_blocking` — and none is `async` except [`GitQueue::write`]. Every function
//! takes the git program first (`git: &OsStr`, so a test can pass a recording script)
//! and a per-command timeout last. Every git command goes through
//! [`crate::worktree::run_git_with_cap`], so AGENTS.md rules 10 and 11 hold by
//! construction: `-C <dir> --no-optional-locks` and a scrubbed environment on every
//! invocation. Every engine **write** also carries [`WRITE_FLAGS`] (decision 18).

mod queue;
mod worktrees;

pub use queue::{GitQueue, LOCK_RETRY_DELAYS_MS};
pub use worktrees::{create_run_branch, lock_worktree, prepare_review, prepare_worktree};

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::contract::{REVIEW_DIFF_MAX, clamp_diff};
use super::globs::{OwnsMatcher, names_literally};
use super::plan::Preflight;
use crate::project;
use crate::worktree::{GitOutput, run_git_with_cap};

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

/// The stdout cap for whole-tree listings and diffs, which a real repository can push
/// far past `run_git`'s default 256 KiB; a diff is clamped to [`REVIEW_DIFF_MAX`]
/// only after it has been read whole.
const LARGE_OUTPUT_BYTES: usize = 64 * 1024 * 1024;

/// The files Claude Code reads as project settings (decision 53).
const CLAUDE_SETTINGS: [&str; 2] = [".claude/settings.json", ".claude/settings.local.json"];
const CLAUDE_MCP: &str = ".mcp.json";

/// What `verify_done` found (decision 32's done gate, split per decisions 55 and 56).
/// The engine's `OpResult::DoneChecked` (M8a.11) carries exactly these fields.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DoneChecked {
    /// The task's own commits: reachable from `HEAD` but from neither the start
    /// commit nor the current run head.
    pub commits: u32,
    /// Tracked files with uncommitted changes, staged or not.
    pub dirty_tracked: u32,
    /// `MERGE_HEAD` exists.
    pub merge_in_progress: bool,
    /// Untracked (not ignored) files that `owns` matches.
    pub untracked_in_owns: Vec<String>,
    /// Changed paths outside `owns`, neither protected nor generated.
    pub outside_owns: Vec<String>,
    /// Changed paths outside `owns` that `generated` matches.
    pub generated_outside_owns: Vec<String>,
    /// Changed paths that `protected` matches and `owns` does not name literally.
    pub protected_changed: Vec<String>,
    /// `None` without a `red`; else whether it is one of the task's own commits after
    /// its start commit.
    pub red_ok: Option<bool>,
    pub head: String,
}

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

fn nul_fields(text: &str) -> impl Iterator<Item = &str> {
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
    protected: &OwnsMatcher,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    Ok(tracked_at(g, root, base_sha, &[])?
        .into_iter()
        .filter(|path| protected.matches(path))
        .collect())
}

/// Decision 32's done check in `worktree`, at most six git calls: `HEAD`, the task's
/// own commits (`HEAD ^start ^run_head`), `status`, `MERGE_HEAD`, the spill diff
/// (`git diff --name-only <run_head>...HEAD`), and — only with a `red` — `red`
/// resolved. The changed paths are split per decision 56 (protected, unless `owns`
/// names them literally) and then decision 55 (the rest outside `owns`: generated or
/// not). Exemptions (an overridden task) are the engine's, not this function's.
#[allow(clippy::too_many_arguments)]
pub fn verify_done(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    run_head: &str,
    owns: &[String],
    generated: &OwnsMatcher,
    protected: &OwnsMatcher,
    red: Option<&str>,
    timeout: Duration,
) -> Result<DoneChecked, String> {
    let owns_matcher = OwnsMatcher::new(owns)?;
    let g = Git::new(git, timeout);
    let head = g
        .ok(worktree, &[os("rev-parse"), os("HEAD")])?
        .trim()
        .to_string();

    let not_start = format!("^{start}");
    let not_run_head = format!("^{run_head}");
    let own = g.ok(
        worktree,
        &[
            os("rev-list"),
            os("HEAD"),
            os(&not_start),
            os(&not_run_head),
        ],
    )?;
    let own: Vec<&str> = own.lines().filter(|line| !line.is_empty()).collect();

    let status = g.ok(
        worktree,
        &[
            os("status"),
            os("--porcelain"),
            os("-z"),
            os("--untracked-files=all"),
        ],
    )?;
    let (dirty_tracked, untracked) = parse_status(&status);
    let untracked_in_owns = untracked
        .into_iter()
        .filter(|path| owns_matcher.matches(path))
        .collect();

    let merge_head = g.read(
        worktree,
        &[os("rev-parse"), os("-q"), os("--verify"), os("MERGE_HEAD")],
    )?;

    let spill_range = format!("{run_head}...HEAD");
    let changed = g.ok(
        worktree,
        &[
            os("diff"),
            os("--name-only"),
            os("-z"),
            os("--no-renames"),
            os(&spill_range),
        ],
    )?;
    let mut result = DoneChecked {
        commits: own.len() as u32,
        dirty_tracked,
        merge_in_progress: merge_head.success,
        untracked_in_owns,
        head,
        ..DoneChecked::default()
    };
    for path in nul_fields(&changed) {
        if protected.matches(path) {
            if !names_literally(owns, path) {
                result.protected_changed.push(path.to_string());
            }
            continue;
        }
        if owns_matcher.matches(path) {
            continue;
        }
        if generated.matches(path) {
            result.generated_outside_owns.push(path.to_string());
        } else {
            result.outside_owns.push(path.to_string());
        }
    }

    if let Some(red) = red {
        let spec = format!("{red}^{{commit}}");
        let resolved = g.read(
            worktree,
            &[os("rev-parse"), os("-q"), os("--verify"), os(&spec)],
        )?;
        let sha = resolved.stdout.trim();
        result.red_ok = Some(resolved.success && own.contains(&sha));
    }
    Ok(result)
}

/// `git status --porcelain -z`: the number of tracked entries with changes, and the
/// untracked paths. A rename or copy entry is followed by its source path, skipped.
fn parse_status(status: &str) -> (u32, Vec<String>) {
    let mut dirty = 0;
    let mut untracked = Vec::new();
    let mut fields = nul_fields(status);
    while let Some(entry) = fields.next() {
        let (code, path) = entry.split_at(entry.len().min(3));
        if code.starts_with("??") {
            untracked.push(path.to_string());
            continue;
        }
        if code.starts_with("!!") {
            continue;
        }
        dirty += 1;
        if code.starts_with(['R', 'C']) {
            fields.next();
        }
    }
    (dirty, untracked)
}

/// The commits on `HEAD` since `start` (`git rev-list --count <start>..HEAD`), and
/// `HEAD`: the turn-end fallback's question (decision 32).
pub fn count_commits(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    timeout: Duration,
) -> Result<(u32, String), String> {
    let g = Git::new(git, timeout);
    let range = format!("{start}..HEAD");
    let count = g.ok(worktree, &[os("rev-list"), os("--count"), os(&range)])?;
    let count = count
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("git rev-list --count printed {:?}", count.trim()))?;
    let head = g.ok(worktree, &[os("rev-parse"), os("HEAD")])?;
    Ok((count, head.trim().to_string()))
}

/// Decision 30's hand-over material: `git diff --stat <start>..HEAD` and the diff
/// itself, clamped to [`REVIEW_DIFF_MAX`]. Uncommitted work is not in either.
pub fn diff_so_far(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let g = Git::new(git, timeout);
    let range = format!("{start}..HEAD");
    let stat = g.ok(worktree, &[os("diff"), os("--stat"), os(&range)])?;
    let patch = diff(g, worktree, &range)?;
    Ok((stat, patch))
}

/// `git diff <range>`, clamped to [`REVIEW_DIFF_MAX`]. Never an external diff driver
/// or colour, whatever the user's config says.
pub(crate) fn diff(g: Git<'_>, dir: &Path, range: &str) -> Result<String, String> {
    let patch = g.ok(
        dir,
        &[os("diff"), os("--no-ext-diff"), os("--no-color"), os(range)],
    )?;
    Ok(clamp_diff(&patch, REVIEW_DIFF_MAX))
}
