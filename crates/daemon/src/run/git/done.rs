//! Decision 32's done check (`verify_done`), with decision 55 and 56's split of the
//! changed paths. Blocking; split from `mod.rs` for AGENTS.md rule 8.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::import::{HeadFile, head_file, rebase_in_progress, sync_in, task_pin};
use super::{DIFF_FLAGS, Git, NO_NESTED, diff, nul_fields, os};
use crate::run::globs::{OwnsMatcher, ProtectedMatcher, names_literally};

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
    /// The task's branch, without `refs/heads/`, once the engine has recorded `head` on
    /// it (final fix batch F1b; fix round 1, finding 11). `None` when the worktree's
    /// `HEAD` is not a detached commit (a branch checked out, a rebase stopped): the
    /// engine rejects that claim, whose commits the branch would not carry.
    pub head_branch: Option<String>,
}

/// Decision 32's done check in `worktree`. Final fix batch F1b: the worker's detached
/// `HEAD` is first imported and recorded on the task's branch ([`super::sync`]; a
/// branch checked out or a rebase stopped gives `head_branch: None` and nothing else).
/// Then: `<head>`, judged throughout (final fix batch F1, A-I4), the task's own
/// commits (`<head> ^start ^run_head`), `status`, `MERGE_HEAD`, the spill diff
/// (`git diff --name-only <run_head>...<head>`), and — only with a `red` — `red`
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
    protected: &ProtectedMatcher,
    red: Option<&str>,
    timeout: Duration,
) -> Result<DoneChecked, String> {
    let owns_matcher = OwnsMatcher::new(owns)?;
    let g = Git::new(git, timeout);
    let pin = task_pin(worktree)?;
    let ready = matches!(head_file(&pin)?, HeadFile::Commit(_)) && !rebase_in_progress(&pin);
    if !ready {
        return Ok(DoneChecked::default());
    }
    let head = sync_in(g, worktree)?;
    let head_branch = pin
        .own
        .as_deref()
        .and_then(|own| own.strip_prefix("refs/heads/"))
        .map(str::to_string);

    let not_start = format!("^{start}");
    let not_run_head = format!("^{run_head}");
    // Final fix batch F1, finding A-I4: every later check judges `head`, the commit
    // the engine gates and merges, never a `HEAD` the worker has moved on since.
    let own = g.ok(
        worktree,
        &[os("rev-list"), os(&head), os(&not_start), os(&not_run_head)],
    )?;
    let own: Vec<&str> = own.lines().filter(|line| !line.is_empty()).collect();

    let status = g.ok(
        worktree,
        &[
            os("status"),
            os("--porcelain"),
            os("-z"),
            os("--untracked-files=all"),
            os(NO_NESTED),
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

    let spill_range = format!("{run_head}...{head}");
    let mut spill_args = vec![os("diff")];
    spill_args.extend(DIFF_FLAGS.map(os));
    spill_args.extend([
        os("--name-only"),
        os("-z"),
        os("--no-renames"),
        os(&spill_range),
    ]);
    let changed = g.ok(worktree, &spill_args)?;
    let mut result = DoneChecked {
        commits: own.len() as u32,
        dirty_tracked,
        merge_in_progress: merge_head.success,
        untracked_in_owns,
        head,
        head_branch,
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
        // Either status byte can be a rename or copy: `R ` staged, ` R` in the working
        // tree (an intent-to-add rename). Fix round 1, finding 7.
        if code[..code.len().min(2)].contains(['R', 'C']) {
            fields.next();
        }
    }
    (dirty, untracked)
}

/// The task's own commits (`git rev-list --count <head> ^<start> ^<run_head>`, as
/// [`verify_done`] counts them) and `<head>`: the turn-end fallback's question (decision
/// 32). With `run_head` excluded, a run head merged in by a hand-back is not counted
/// as the task's work (fix round 1, finding 10). Final fix batch F1b: `<head>` is the
/// worker's detached `HEAD`, imported and recorded on the task's branch first
/// ([`super::sync`]), so this writes: call it behind the run's [`super::GitQueue::write`].
pub fn count_commits(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    run_head: &str,
    timeout: Duration,
) -> Result<(u32, String), String> {
    let g = Git::new(git, timeout);
    let head = sync_in(g, worktree)?;
    let not_start = format!("^{start}");
    let not_run_head = format!("^{run_head}");
    let count = g.ok(
        worktree,
        &[
            os("rev-list"),
            os("--count"),
            os(&head),
            os(&not_start),
            os(&not_run_head),
        ],
    )?;
    let count = count
        .trim()
        .parse::<u32>()
        .map_err(|_| format!("git rev-list --count printed {:?}", count.trim()))?;
    Ok((count, head))
}

/// Decision 30's hand-over material: the stat and the diff of the task's net change,
/// clamped to [`crate::run::contract::REVIEW_DIFF_MAX`]. Uncommitted work is not in either. The range is
/// `<start>..HEAD` until a hand-back merges a newer run head in, then
/// `<run_head>..HEAD`: `<run_head>...HEAD` gives exactly that, since the merge base of
/// the run head and `HEAD` is the start commit or the merged run head (fix round 1,
/// finding 10). Final fix batch F1b: `HEAD` is the worker's detached `HEAD`, imported
/// and recorded on the task's branch first ([`super::sync`]); this writes.
pub fn diff_so_far(
    git: &OsStr,
    worktree: &Path,
    start: &str,
    run_head: &str,
    timeout: Duration,
) -> Result<(String, String), String> {
    let g = Git::new(git, timeout);
    let head = sync_in(g, worktree)?;
    // A run head that has moved on without a hand-back has `start` as its merge base
    // with `HEAD`; one that was merged in is its own. So `run_head...HEAD` never needs
    // `start`, which is kept for the op's record and to match `count_commits`.
    let _ = start;
    let range = format!("{run_head}...{head}");
    let mut stat_args = vec![os("diff")];
    stat_args.extend(DIFF_FLAGS.map(os));
    stat_args.extend([os("--stat"), os(&range)]);
    let stat = g.ok(worktree, &stat_args)?;
    let patch = diff(g, worktree, &range)?;
    Ok((stat, patch))
}
