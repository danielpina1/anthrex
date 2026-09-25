//! Run git operations (M8a.8): start preflight (decision 17), the project-settings scan
//! (decision 53), the protected-file listing (decision 56), the done check that splits a
//! task's changed paths (decisions 32, 55 and 56), and the commit count and diff a
//! fresh session is handed (decision 30). The done check itself is in [`done`], the
//! run, task and review worktrees in [`worktrees`], and the per-repository write queue
//! in [`queue`]. M8a.9 adds [`merge`] (the merge candidate, the compare-and-swap, the
//! hand-back and the ref guard, decisions 21 and 36; the hand-back is in [`handback`])
//! and [`salvage`] (salvage, removal,
//! branch deletion and accept, decision 20).
//!
//! Does I/O (design decision 2). Every function here is **blocking** — call it only from
//! `spawn_blocking` — and none is `async` except [`GitQueue::write`]. Every function
//! takes the git program first (`git: &OsStr`, so a test can pass a recording script)
//! and a per-command timeout last. Every git command goes through
//! [`crate::worktree::run_git_with_cap`], so AGENTS.md rules 10 and 11 hold by
//! construction: `-C <dir> --no-optional-locks` and a scrubbed environment on every
//! invocation, and `-c core.fsmonitor=false` ([`crate::worktree::NO_FSMONITOR`]). Every
//! engine **read**, and `run accept`'s merge, carries [`NO_HOOKS`]; every engine
//! **write** carries [`WRITE_FLAGS`] (decision 18), which include it (final fix batch
//! F1, findings C-C1 and D-5).

mod accept;
mod checkout;
mod done;
mod handback;
mod import;
mod merge;
mod merge_state;
mod queue;
mod resolution;
mod salvage;
mod sandbox;
mod worktrees;

pub use accept::{ACCEPT_MERGE_TIMEOUT, accept, accept_with_merge_timeout};
pub use checkout::{Repo, checkout_repo_dir, default_repo_dir};
pub use done::{DoneChecked, count_commits, diff_so_far, verify_done};
pub use handback::{HandBack, hand_back};
pub use import::{HeadFile, head_file, rebase_in_progress, sync};
pub use merge::{
    ACCEPT_LIST_MAX, AcceptOutcome, CandidateStep, RefCheck, cas_update, commit_tree,
    commits_since, guard_refs, materialize, merge_tree, read_ref, reattach, run_work_on_base,
};
pub use merge_state::abort_merge;
pub use salvage::{delete_branches, remove_checkout, remove_worktree, salvage};

/// Reads reconcile (M8a.21) shares with the ops it checks.
pub(crate) use handback::{finish_clean, interrupted_conflict};
pub(crate) use import::{is_id, sync_in};
pub(crate) use merge::{read, reattach_in, short};
pub(crate) use merge_state::{
    Leftover, clear as clear_merge_state, leftover, undo_clean_merge, unmerged,
};
pub(crate) use worktrees::{forget_missing, is_ancestor, listed as listed_worktree_in};

pub use queue::{GitQueue, LOCK_RETRY_DELAYS_MS};
pub use resolution::resolution_only;
pub use sandbox::{private_dir, worker_git_dirs};
pub use worktrees::{
    absolute_git_dir, create_run_branch, lock_worktree, pin_worktrees, prepare_review,
    prepare_review_in, prepare_scratch, prepare_scratch_in, prepare_task_worktree,
    prepare_worktree,
};

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::contract::{REVIEW_DIFF_MAX, clamp_diff};
use super::globs::ProtectedMatcher;
use super::plan::Preflight;
use crate::project;
use crate::subprocess::HeadTail;
use crate::worktree::{
    GitOutput, run_git_head_tail, run_git_with_cap, run_git_with_input, run_git_with_stdin_file,
};

/// Every engine git call that is not a [`WRITE_FLAGS`] write passes this ahead of its
/// subcommand: reads, and `run accept`'s merge and its abort in the user's checkout. A
/// hook planted in the repository's hooks directory (a worker could once write it)
/// never runs inside the daemon, nor inside accept. Final fix batch F1 (C-C1, D-5); for
/// accept this departs from decision 18, which let the user's hooks run there
/// (Implementation notes).
pub const NO_HOOKS: [&str; 2] = ["-c", "core.hooksPath=/dev/null"];

/// Decision 18: every engine write passes these ahead of its subcommand, so a user's
/// hooks or a signing pinentry can never hang a run. Final fix batch F1, fix round 5:
/// and no engine write creates a reflog, so an engine worktree never has one a worker's
/// symbolic link could redirect (git appends to an existing reflog through a link).
pub const WRITE_FLAGS: [&str; 6] = [
    "-c",
    "core.hooksPath=/dev/null",
    "-c",
    "commit.gpgSign=false",
    "-c",
    "core.logAllRefUpdates=false",
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
/// 1, finding 6. Nor can `diff.ignoreSubmodules` or a `.gitmodules` `ignore = all` hide
/// a gitlink change from the owns, protected and resolution checks (ruling T14-R3):
/// `dirty` still reports every gitlink whose commit changed, but never looks inside a
/// nested repository's working tree, which would run `git status` there under that
/// repository's own (worker-written) config (M8a final fix batch F1, fix round 1, N1).
pub(crate) const DIFF_FLAGS: [&str; 4] =
    ["--no-color", "--no-ext-diff", "--no-textconv", NO_NESTED];

/// On every engine `status` and worktree `diff`: never recurse into a nested
/// repository (N1). Gitlink commit changes are still reported.
pub(crate) const NO_NESTED: &str = "--ignore-submodules=dirty";

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
            &Self::unhooked(args),
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
            &Self::unhooked(args),
            Instant::now() + self.timeout,
            head_bytes,
            0,
        )
        .map_err(|err| err.to_string())
    }

    /// `args` as given: the caller has put [`NO_HOOKS`] or [`WRITE_FLAGS`] first.
    fn raw(&self, dir: &Path, args: &[&OsStr], cap: usize) -> Result<GitOutput, String> {
        run_git_with_cap(self.program, dir, args, Instant::now() + self.timeout, cap)
            .map_err(|err| err.to_string())
    }

    /// [`NO_HOOKS`], then `args`.
    fn unhooked<'b>(args: &[&'b OsStr]) -> Vec<&'b OsStr> {
        let mut full: Vec<&OsStr> = NO_HOOKS.iter().map(|flag| os(flag)).collect();
        full.extend_from_slice(args);
        full
    }

    /// A read whose failure the caller interprets.
    pub(crate) fn read(&self, dir: &Path, args: &[&OsStr]) -> Result<GitOutput, String> {
        self.raw(dir, &Self::unhooked(args), LARGE_OUTPUT_BYTES)
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

    /// A write (decision 18's flags first) fed `input` on stdin, that must succeed; its
    /// stdout. For `update-index --index-info` (final fix batch F1, fix round 5).
    pub(crate) fn write_input(
        &self,
        dir: &Path,
        args: &[&OsStr],
        input: &[u8],
    ) -> Result<String, String> {
        let mut full: Vec<&OsStr> = WRITE_FLAGS.iter().map(|flag| os(flag)).collect();
        full.extend_from_slice(args);
        let output = run_git_with_input(
            self.program,
            dir,
            &full,
            Instant::now() + self.timeout,
            input,
        )
        .map_err(|err| err.to_string())?;
        succeeded(args, output)
    }

    /// A read fed `input` on stdin, that must succeed; its stdout (`cat-file
    /// --batch-check`, final fix batch F1b).
    pub(crate) fn read_input(
        &self,
        dir: &Path,
        args: &[&OsStr],
        input: &[u8],
    ) -> Result<String, String> {
        let output = run_git_with_input(
            self.program,
            dir,
            &Self::unhooked(args),
            Instant::now() + self.timeout,
            input,
        )
        .map_err(|err| err.to_string())?;
        succeeded(args, output)
    }

    /// A write (decision 18's flags first) with `file` as its stdin, that must succeed;
    /// its stdout (`index-pack --stdin` of an imported pack, final fix batch F1b).
    pub(crate) fn write_file(
        &self,
        dir: &Path,
        args: &[&OsStr],
        file: &std::fs::File,
    ) -> Result<String, String> {
        let mut full: Vec<&OsStr> = WRITE_FLAGS.iter().map(|flag| os(flag)).collect();
        full.extend_from_slice(args);
        let output = run_git_with_stdin_file(
            self.program,
            dir,
            &full,
            Instant::now() + self.timeout,
            file,
        )
        .map_err(|err| err.to_string())?;
        succeeded(args, output)
    }

    /// A write into the user's own checkout (`run accept`), which decision 18 exempts
    /// from [`WRITE_FLAGS`]: their signing applies to their merge. Their hooks do not
    /// ([`NO_HOOKS`], final fix batch F1). The caller interprets its failure.
    pub(crate) fn user_write(&self, dir: &Path, args: &[&OsStr]) -> Result<GitOutput, String> {
        self.raw(dir, &Self::unhooked(args), LARGE_OUTPUT_BYTES)
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
        // Final fix batch F1, D-13: `anthrex/` is reserved for runs; a run based on
        // another run's branch would merge into it and lose its base to its discard.
        Some(branch) if head_ref.success && branch.starts_with("anthrex/") => {
            return Err(format!(
                "{} is on {branch}, a branch reserved for runs; check out your own branch first",
                root.display()
            ));
        }
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

    // M8a final fix batch F1, fix round 1 (N2): per-worktree config would live in each
    // engine worktree's git directory, where the engine cannot keep it inert.
    let worktree_config = g.read(
        &root,
        &[
            os("config"),
            os("--bool"),
            os("--get"),
            os("extensions.worktreeConfig"),
        ],
    )?;
    if worktree_config.success && worktree_config.stdout.trim() == "true" {
        return Err(format!(
            "{} uses per-worktree config (extensions.worktreeConfig), which anthrex runs do not support",
            root.display()
        ));
    }

    let status = g.ok(
        &root,
        &[
            os("status"),
            os("--porcelain"),
            os("--untracked-files=no"),
            os(NO_NESTED),
        ],
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
