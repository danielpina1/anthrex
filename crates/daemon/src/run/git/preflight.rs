//! Decision 17's start preflight (split from `mod.rs`): the repository checks in their
//! order, the bare-repository answer and the minimum git version.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::plan::Preflight;
use super::{Git, NO_NESTED, os};
use crate::project;

/// Decision 17: `merge-tree --write-tree` needs git 2.38.
const MIN_GIT: (u32, u32) = (2, 38);

/// Decision 17's start preflight, in its order. `protected_files` is left empty: the
/// protected list depends on the resolved profile, which the caller has and passes to
/// [`super::protected_files`]; decision 53's project-settings refusal is likewise the
/// caller's, through [`super::project_settings`], only when it applies.
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
        gate_files: Vec::new(),
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
