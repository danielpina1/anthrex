//! Milestone 9's git reads (task M9.6): a task branch's commits and diff size for
//! `task_result` (decision 18), and the `(base, head)` a review task's target names
//! (decision 36). Blocking — call from `spawn_blocking` — and read-only: no write
//! queue. Every command goes through [`Git::read`], so `-C <dir> --no-optional-locks`,
//! the scrubbed environment and [`super::NO_HOOKS`] hold by construction (AGENTS.md
//! rule 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, RefreshedIn, failure, os};
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
    task_summary_excluding(git, root, start, branch, &RefreshedIn::default(), timeout)
}

/// [`task_summary`] for a task its refreshes merged the run branch into (milestone 9
/// decision 42e; M9.13a review, item 2, and re-review): merged run work is never the
/// task's. The log leaves out everything a recorded target (a run head a refresh
/// merged, clean or conflicted) reaches, and each clean refresh merge; the diff is from
/// the newest recorded target the branch has, `<target>...<branch>`, instead of from
/// `start`. A recorded target `root` does not have is ignored.
pub fn task_summary_excluding(
    git: &OsStr,
    root: &Path,
    start: &str,
    branch: &str,
    refreshed: &RefreshedIn,
    timeout: Duration,
) -> Result<TaskGit, String> {
    // A revision that reads as an option (`--output=<path>`) is refused before git
    // sees it; `--end-of-options` below keeps the range a revision regardless.
    let revs = [start, branch].into_iter();
    let revs = revs.chain(refreshed.targets.iter().map(String::as_str));
    let revs = revs.chain(refreshed.merges.iter().map(String::as_str));
    if let Some(rev) = revs.into_iter().find(|r| r.starts_with('-')) {
        return Err(format!("{rev}: a revision cannot start with '-'"));
    }
    let g = Git::new(git, timeout);
    // Each recorded target `root` has, oldest first, once.
    let mut targets: Vec<&str> = Vec::new();
    for target in &refreshed.targets {
        let spec = format!("{target}^{{commit}}");
        let args = [os("rev-parse"), os("-q"), os("--verify"), os(&spec)];
        if !targets.contains(&target.as_str()) && g.read(root, &args)?.success {
            targets.push(target);
        }
    }
    let range = format!("{start}..{branch}");
    let mut args = vec![
        os("log"),
        os("--no-color"),
        os("--format=%h%x1f%s"),
        os("-n"),
        os(LOG_MAX),
        os("--end-of-options"),
        os(&range),
    ];
    let excluded: Vec<String> = targets.iter().map(|t| format!("^{t}")).collect();
    args.extend(excluded.iter().map(|rev| os(rev)));
    let log = g.ok(root, &args)?;
    let merges = &refreshed.merges;
    let commits = log
        .lines()
        .filter_map(|line| line.split_once('\u{1f}'))
        // A short sha is unique in the repository, so it prefixes one merge at most.
        .filter(|(sha, _)| !merges.iter().any(|m| m.starts_with(sha)))
        .map(|(sha, subject)| (sha.to_string(), subject.to_string()))
        .collect();
    // The newest recorded target the branch has is the base.
    let mut base = start.to_string();
    for target in targets.iter().rev() {
        let args = [
            os("merge-base"),
            os("--is-ancestor"),
            os(target),
            os(branch),
        ];
        if g.read(root, &args)?.success {
            base = target.to_string();
            break;
        }
    }
    let range = format!("{base}...{branch}");
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
