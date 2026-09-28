//! Milestone 9's git reads (task M9.6): a task branch's commits and diff size for
//! `task_result` (decision 18), and the `(base, head)` a review task's target names
//! (decision 36). Blocking — call from `spawn_blocking` — and read-only: no write
//! queue. Every command goes through [`Git::read`], so `-C <dir> --no-optional-locks`,
//! the scrubbed environment and [`super::NO_HOOKS`] hold by construction (AGENTS.md
//! rule 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, failure, os};
use crate::run::orch::result::TaskGit;

/// Commits `task_result` lists, newest first.
const LOG_MAX: &str = "50";
/// Lines of `git diff --stat` kept; the last is always the total.
const DIFFSTAT_LINES: usize = 60;

/// `git log --format=%h%x1f%s -n 50 --end-of-options <start>..<branch>` and `git diff
/// --stat=100 --end-of-options <start>...<branch>`, in `root`: the task's commits
/// (short sha, subject) and its diffstat, at most 60 lines with the total kept. A
/// `start` or `branch` beginning with `-` is refused.
pub fn task_summary(
    git: &OsStr,
    root: &Path,
    start: &str,
    branch: &str,
    timeout: Duration,
) -> Result<TaskGit, String> {
    // A revision that reads as an option (`--output=<path>`) is refused before git
    // sees it; `--end-of-options` below keeps the range a revision regardless.
    if let Some(rev) = [start, branch].into_iter().find(|r| r.starts_with('-')) {
        return Err(format!("{rev}: a revision cannot start with '-'"));
    }
    let g = Git::new(git, timeout);
    let range = format!("{start}..{branch}");
    let log = g.ok(
        root,
        &[
            os("log"),
            os("--no-color"),
            os("--format=%h%x1f%s"),
            os("-n"),
            os(LOG_MAX),
            os("--end-of-options"),
            os(&range),
        ],
    )?;
    let commits = log
        .lines()
        .filter_map(|line| line.split_once('\u{1f}'))
        .map(|(sha, subject)| (sha.to_string(), subject.to_string()))
        .collect();
    let range = format!("{start}...{branch}");
    let mut args = vec![os("diff")];
    args.extend(DIFF_FLAGS.iter().map(|flag| os(flag)));
    args.extend([os("--stat=100"), os("--end-of-options"), os(&range)]);
    let stat = g.ok(root, &args)?;
    Ok(TaskGit {
        commits,
        diffstat: clamp_stat(&stat),
    })
}

/// At most [`DIFFSTAT_LINES`] lines: the first files, then the total line.
fn clamp_stat(stat: &str) -> String {
    let lines: Vec<&str> = stat.lines().collect();
    if lines.len() <= DIFFSTAT_LINES {
        return lines.join("\n");
    }
    let mut kept = lines[..DIFFSTAT_LINES - 1].to_vec();
    kept.extend(lines.last());
    kept.join("\n")
}

/// Decision 36: `<a>..<b>` resolves each side, `git rev-parse --verify
/// <rev>^{commit}`; a single revision `r` means `merge-base(<base branch>, r)..r`.
/// `Err` names an unresolvable side with git's stderr tail (`nope: fatal: Needed a
/// single revision`), else holds the failure.
pub fn resolve_target(
    git: &OsStr,
    root: &Path,
    target: &str,
    base_branch: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let g = Git::new(git, timeout);
    match target.split_once("..") {
        Some((base, head)) => Ok((commit(g, root, base)?, commit(g, root, head)?)),
        None => {
            let head = commit(g, root, target)?;
            let branch = commit(g, root, base_branch)?;
            let base = g.ok(root, &[os("merge-base"), os(&branch), os(&head)])?;
            Ok((base.trim().to_string(), head))
        }
    }
}

/// The full sha of the commit `rev` names.
fn commit(g: Git<'_>, root: &Path, rev: &str) -> Result<String, String> {
    let spec = format!("{rev}^{{commit}}");
    let args = [
        os("rev-parse"),
        os("--verify"),
        os("--end-of-options"),
        os(&spec),
    ];
    let output = g.read(root, &args)?;
    if output.success {
        Ok(output.stdout.trim().to_string())
    } else if output.stderr.trim().is_empty() {
        Err(failure(&args, &output))
    } else {
        Err(format!("{rev}: {}", output.stderr_tail()))
    }
}
