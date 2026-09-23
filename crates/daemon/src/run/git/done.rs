//! Decision 32's done check (`verify_done`), with decision 55 and 56's split of the
//! changed paths. Blocking; split from `mod.rs` for AGENTS.md rule 8.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::{DIFF_FLAGS, Git, nul_fields, os};
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
    /// The branch `HEAD` is on, without `refs/heads/`; `None` when detached (fix round
    /// 1, finding 11). The engine rejects a claim made off the task's branch, whose
    /// commits the branch would not carry.
    pub head_branch: Option<String>,
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
    protected: &ProtectedMatcher,
    red: Option<&str>,
    timeout: Duration,
) -> Result<DoneChecked, String> {
    let owns_matcher = OwnsMatcher::new(owns)?;
    let g = Git::new(git, timeout);
    // One call: `HEAD`'s sha, then the ref it is on (`HEAD` itself when detached).
    let heads = g.ok(
        worktree,
        &[
            os("rev-parse"),
            os("HEAD"),
            os("--symbolic-full-name"),
            os("HEAD"),
        ],
    )?;
    let mut heads = heads.lines();
    let head = heads.next().unwrap_or_default().trim().to_string();
    let head_branch = heads
        .next()
        .and_then(|line| line.trim().strip_prefix("refs/heads/"))
        .map(str::to_string);

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
