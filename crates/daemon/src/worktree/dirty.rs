//! The one question a removal must never get wrong: does this worktree hold work?
//!
//! Design decision 12, as amended by ruling during task M5.3. It lives beside
//! [`super::ops`] rather than inside it because it is the safety-critical half of
//! removal and answers a question of its own — `ops` decides *what to do*, this decides
//! *whether it is safe to*. Every question here fails in the refusing direction: an
//! answer that cannot be obtained is an error, never a "no".

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Instant;

use super::{WorktreeError, run_git};

/// The per-worktree files and directories whose existence means git has an operation
/// paused in this worktree, each with the reason a removal would give for refusing.
/// Each marker is resolved with `rev-parse --git-path`, because a linked worktree's are
/// under `.git/worktrees/<name>/`, not the repository's `.git/`.
///
/// `BISECT_LOG` is the whole-branch review's finding 5: verified against git 2.50.1, a
/// bisect in progress leaves `status --porcelain` empty, matches none of the other
/// markers, counts zero unreachable commits, and is deleted by a plain
/// `git worktree remove` — taking `BISECT_LOG`, the record of every good/bad answer the
/// user has given, which exists nowhere else.
///
/// `REVERT_HEAD` is the same hole, found by looking for another instance of it rather
/// than reported. A paused `git revert` usually leaves conflict markers, so question 2
/// refuses the removal — but with the wrong reason, which is finding 2 again. And when
/// the conflict is resolved to content identical to `HEAD`, `git add` leaves *nothing*
/// staged: verified against git 2.50.1, `status --porcelain` is then empty, the count of
/// unreachable commits is 0, and `git worktree remove` exits 0 and takes `REVERT_HEAD`
/// with it. `git am` needs no entry of its own — it uses `rebase-apply`.
const OPERATION_MARKERS: [(&str, DirtyReason); 6] = [
    ("rebase-merge", DirtyReason::Rebase),
    ("rebase-apply", DirtyReason::Rebase),
    ("MERGE_HEAD", DirtyReason::Merge),
    ("CHERRY_PICK_HEAD", DirtyReason::CherryPick),
    ("REVERT_HEAD", DirtyReason::Revert),
    ("BISECT_LOG", DirtyReason::Bisect),
];

/// Why a worktree is not safe to remove — which of [`dirty_reason`]'s questions fired.
///
/// It exists so the refusal can say what is actually true. A single `bool` left
/// [`WorktreeError::Dirty`] with one sentence for six different states, and that sentence
/// —"uncommitted or untracked changes" — is *false* for the states that hold commits
/// rather than files. That matters more than tidiness: the daemon's message is the sole
/// basis for the user's force-or-keep decision (design decision 24), and a user told
/// about uncommitted changes who runs `git status`, sees a spotlessly clean tree, and
/// concludes the daemon is confused will force — destroying the paused rebase, the
/// unreachable commits or the bisect that `git status` never mentioned.
///
/// Every variant's wording describes what would happen to the worktree's **files and
/// commits**, never to the agent's process: the dirty check runs before the agent is
/// signalled, so on the common path it is still running while this text is on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirtyReason {
    /// `rebase-merge` or `rebase-apply`: a rebase stopped part-way.
    Rebase,
    /// `MERGE_HEAD`: a merge stopped, usually at a conflict.
    Merge,
    /// `CHERRY_PICK_HEAD`: a cherry-pick stopped, usually at a conflict.
    CherryPick,
    /// `REVERT_HEAD`: a revert stopped, usually at a conflict.
    Revert,
    /// `BISECT_LOG`: a bisect in progress.
    Bisect,
    /// `status --porcelain` reported something: modified or untracked files.
    Changes,
    /// `HEAD` reaches commits that no branch, tag or remote does.
    UnreachableHead,
}

impl DirtyReason {
    /// The sentence that follows `worktree <path> ` in the refusal.
    fn describe(self) -> &'static str {
        match self {
            Self::Rebase => {
                "has a rebase in progress; removing it discards the rebase and every \
                 commit it has already replayed"
            }
            Self::Merge => {
                "has a merge in progress; removing it discards the merge and everything \
                 resolved in it so far"
            }
            Self::CherryPick => {
                "has a cherry-pick in progress; removing it discards the cherry-pick and \
                 everything resolved in it so far"
            }
            Self::Revert => {
                "has a revert in progress; removing it discards the revert and everything \
                 resolved in it so far"
            }
            Self::Bisect => {
                "has a bisect in progress; removing it discards every good and bad answer \
                 recorded so far"
            }
            Self::Changes => "has uncommitted or untracked changes",
            Self::UnreachableHead => {
                "is detached at commits no branch, tag or remote reaches; removing it \
                 discards those commits"
            }
        }
    }
}

impl std::fmt::Display for DirtyReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.describe())
    }
}

/// Design decision 12 (extended by the M5.3 review's ruling, and again by the
/// whole-branch review's findings 2 and 5): whether the worktree at `path` holds anything
/// a removal would destroy, and — because the answer is shown to the user as the reason
/// to force or keep — *which* of the questions said so.
///
/// Three questions, any one of which means dirty:
///
/// 1. **A paused git operation** — [`OPERATION_MARKERS`]: a rebase, a merge, a
///    cherry-pick or a bisect. A rebase stopped at `edit` with a clean tree reports
///    *nothing* in `status --porcelain`, and a plain `git worktree remove` deletes it and
///    every commit it had already replayed (verified against git 2.50.1). An agent told
///    to try an approach on scratch commits leaves exactly this state.
/// 2. **Working-tree changes** — `status --porcelain --ignore-submodules=none`. Untracked
///    files count; ignored files do not, and a plain `git worktree remove` deletes those
///    itself. *Any* output means dirty, not "any non-whitespace output": git prints
///    nothing for a clean tree, so the two agree in practice, and where they could ever
///    disagree the answer that refuses to delete is the right one.
/// 3. **A detached `HEAD` holding commits no ref reaches** — `rev-list --count HEAD --not
///    --branches --remotes --tags`. Also silent in `status`, and also deleted without
///    `--force`, at which point the commits are unreachable. On a branch the count is
///    always zero, because `--branches` covers that branch, so this question answers
///    itself for the ordinary case.
///
/// Questions 1 and 3 are why the caller must ask *before* `git worktree remove` rather
/// than only classifying a refusal afterwards: git does not refuse either state.
///
/// `None` is the only answer that permits a removal, so every failure to obtain one is an
/// `Err` and never a `None`.
pub fn dirty_reason(
    git: &OsStr,
    path: &Path,
    deadline: Instant,
) -> Result<Option<DirtyReason>, WorktreeError> {
    if let Some(reason) = operation_in_progress(git, path, deadline)? {
        return Ok(Some(reason));
    }

    let output = run_git(
        git,
        path,
        &[
            OsStr::new("status"),
            OsStr::new("--porcelain"),
            OsStr::new("--ignore-submodules=none"),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "status".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    if !output.stdout.is_empty() {
        return Ok(Some(DirtyReason::Changes));
    }

    Ok(head_is_unreachable(git, path, deadline)?.then_some(DirtyReason::UnreachableHead))
}

/// Which of [`OPERATION_MARKERS`] exists in this worktree's git directory, if any.
///
/// One `rev-parse` for every marker, so the answers come back in the order they were
/// asked and can be paired with their reasons positionally. That pairing is why a line
/// count that does not match the marker count is an error rather than a partial answer:
/// a short or ragged reply would silently shift every reason onto the wrong marker, and
/// a refusal that names the wrong state is barely better than no refusal at all.
fn operation_in_progress(
    git: &OsStr,
    path: &Path,
    deadline: Instant,
) -> Result<Option<DirtyReason>, WorktreeError> {
    let mut args = vec![OsStr::new("rev-parse")];
    for (marker, _) in OPERATION_MARKERS {
        args.push(OsStr::new("--git-path"));
        args.push(OsStr::new(marker));
    }
    let output = run_git(git, path, &args, deadline)?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "rev-parse --git-path".to_string(),
            stderr: output.stderr_tail(),
        });
    }

    first_existing_marker(path, &output.stdout).map_err(|stderr| WorktreeError::Git {
        action: "rev-parse --git-path".to_string(),
        stderr,
    })
}

/// Pairs `rev-parse`'s reply with [`OPERATION_MARKERS`] positionally and returns the
/// reason for the first marker that is on disk.
///
/// Split out of [`operation_in_progress`] so the pairing can be tested without a
/// repository: it is the step where a wrong answer becomes a refusal that names the wrong
/// state, or none at all.
fn first_existing_marker(worktree: &Path, stdout: &str) -> Result<Option<DirtyReason>, String> {
    let lines: Vec<&str> = stdout.lines().collect();
    if lines.len() != OPERATION_MARKERS.len() {
        return Err(format!(
            "expected {} paths, got {}",
            OPERATION_MARKERS.len(),
            lines.len()
        ));
    }

    for ((_, reason), line) in OPERATION_MARKERS.into_iter().zip(lines) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // git 2.50.1 answers with absolute paths, but older versions answer relative
        // to the working directory, which is `worktree` here.
        let candidate = Path::new(line);
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            worktree.join(candidate)
        };
        match fs::symlink_metadata(&candidate) {
            Ok(_) => return Ok(Some(reason)),
            // Only "it is not there" means it is not there. Anything else is a marker we
            // cannot rule out, and the answer that refuses to delete wins.
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Ok(Some(reason)),
            Err(_) => {}
        }
    }
    Ok(None)
}

/// Whether `HEAD` reaches commits that no branch, remote-tracking branch or tag does.
fn head_is_unreachable(git: &OsStr, path: &Path, deadline: Instant) -> Result<bool, WorktreeError> {
    let output = run_git(
        git,
        path,
        &[
            OsStr::new("rev-list"),
            OsStr::new("--count"),
            OsStr::new("HEAD"),
            OsStr::new("--not"),
            OsStr::new("--branches"),
            OsStr::new("--remotes"),
            OsStr::new("--tags"),
        ],
        deadline,
    )?;
    if !output.success {
        return Err(WorktreeError::Git {
            action: "rev-list".to_string(),
            stderr: output.stderr_tail(),
        });
    }
    let count: u64 = output
        .stdout
        .trim()
        .parse()
        .map_err(|_| WorktreeError::Git {
            action: "rev-list".to_string(),
            stderr: format!(
                "could not read a commit count from {:?}",
                output.stdout.trim()
            ),
        })?;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// One reply line per marker, in the order they were asked: `git rev-parse` answers
    /// `--git-path` once per argument, so the pairing below is what the real reply looks
    /// like. Derived from [`OPERATION_MARKERS`] rather than written out, so that adding a
    /// marker extends these tests instead of breaking them.
    fn reply_for(root: &Path) -> String {
        OPERATION_MARKERS
            .iter()
            .map(|(marker, _)| format!("{}\n", root.join(marker).display()))
            .collect()
    }

    /// The reason must be read from the *position* of the path that exists. A mutation
    /// that dropped the pairing and answered with the first reason in the table would
    /// call the last marker a rebase, and the user would go looking for a rebase that is
    /// not there.
    #[test]
    fn a_marker_is_named_by_its_position_in_the_reply() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (last_marker, last_reason) = *OPERATION_MARKERS.last().unwrap();
        let (first_marker, first_reason) = OPERATION_MARKERS[0];
        assert_ne!(
            last_reason, first_reason,
            "this test needs the ends of the table to differ"
        );
        fs::write(root.join(last_marker), "x").unwrap();

        assert_eq!(
            first_existing_marker(root, &reply_for(root)),
            Ok(Some(last_reason)),
            "the reason must follow the marker that is on disk, not the table's order"
        );

        // And the first marker wins when both are there, which is the order the table
        // itself fixes.
        fs::write(root.join(first_marker), "x").unwrap();
        assert_eq!(
            first_existing_marker(root, &reply_for(root)),
            Ok(Some(first_reason))
        );
    }

    /// A reply with the wrong number of lines cannot be paired at all, and pairing it
    /// anyway would shift every reason onto the wrong marker. `dirty_reason`'s rule is
    /// that an answer it cannot obtain is an error, never a "clean".
    #[test]
    fn a_reply_that_cannot_be_paired_is_an_error_not_a_clean_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("rebase-merge"), "x").unwrap();

        let short = format!("{}\n", root.join("rebase-merge").display());

        let error = first_existing_marker(root, &short)
            .expect_err("a ragged reply must not be read as an answer");
        assert!(
            error.contains(&format!(
                "expected {} paths, got 1",
                OPERATION_MARKERS.len()
            )),
            "{error}"
        );
        assert!(
            first_existing_marker(root, "").is_err(),
            "and neither must an empty one, which is what a stubbed git would produce"
        );
    }

    /// Every marker must be reachable: a table entry that no reply position can select
    /// is a state the daemon believes it checks and does not.
    #[test]
    fn every_marker_in_the_table_can_fire() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (index, (marker, reason)) in OPERATION_MARKERS.into_iter().enumerate() {
            let lines: Vec<String> = OPERATION_MARKERS
                .iter()
                .enumerate()
                .map(|(other, (name, _))| {
                    // Only the marker under test exists on disk.
                    let name = if other == index { marker } else { name };
                    root.join(format!("{name}-{other}")).display().to_string()
                })
                .collect();
            let present = root.join(format!("{marker}-{index}"));
            fs::write(&present, "x").unwrap();

            assert_eq!(
                first_existing_marker(root, &format!("{}\n", lines.join("\n"))),
                Ok(Some(reason)),
                "marker {marker} must report {reason:?}"
            );
            fs::remove_file(&present).unwrap();
        }
    }
}
