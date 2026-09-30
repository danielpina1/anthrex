//! Milestone 9.1 decision 53: the all-or-nothing ref transaction a merge into the
//! highest stage of a multi-stage run needs (its stage ref and `integration` move
//! together), and decision 48's create-only stage branch.
//!
//! Blocking; call only from `spawn_blocking`. Both are writes (decision 18's flags
//! through [`Git::write_raw`]/[`Git::write_input_raw`]) and belong behind the caller's
//! [`super::GitQueue::write`]. Every git command goes through `run_git` (AGENTS.md rule
//! 11: `--no-optional-locks`, a scrubbed environment). No ref is ever force-updated:
//! every update names the value it expects.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::merge::read;
use super::{Git, failure, os};

/// One `git update-ref --no-deref --stdin` transaction (`start`, one `update <ref> <new>
/// <old>` per entry, `prepare`, `commit`): every ref moves from its `old` to its `new`,
/// or none does. `Ok(false)` when a ref was not at its `old` (moved, or missing); every
/// ref is then untouched. `--no-deref`: a ref made a symbolic ref is replaced, never
/// followed onto the ref it names (as `cas_update`, M8a fix round 4). Each `old` must
/// be a commit id; a ref that does not exist never matches.
pub fn cas(
    git: &OsStr,
    root: &Path,
    updates: &[(String, String, String)],
    timeout: Duration,
) -> Result<bool, String> {
    let g = Git::new(git, timeout);
    let mut input = String::from("start\n");
    for (refname, new, old) in updates {
        // A line break in a name would be a second command; git refuses such a ref
        // name anyway, so this only keeps the transaction what it says it is.
        if [refname, new, old]
            .iter()
            .any(|s| s.contains(['\n', '\0']) || s.is_empty())
        {
            return Err(format!(
                "refusing a ref update with an invalid field: {refname}"
            ));
        }
        input.push_str(&format!("update {refname} {new} {old}\n"));
    }
    input.push_str("prepare\ncommit\n");
    let args = [os("update-ref"), os("--no-deref"), os("--stdin")];
    let output = g.write_input_raw(root, &args, input.as_bytes())?;
    if output.success {
        return Ok(true);
    }
    // git's refusal names the mismatch in words that vary by version; the refs
    // themselves say whether an `old` was the reason.
    for (refname, _, old) in updates {
        if read(g, root, refname)?.as_deref() != Some(old.as_str()) {
            return Ok(false);
        }
    }
    Err(failure(&args, &output))
}

/// Decision 48: `git update-ref --no-deref refs/heads/<branch> <from> ""`, created only
/// when it does not exist. `Ok(false)` when it already exists (wherever it points); it
/// is then untouched.
pub fn create_branch(
    git: &OsStr,
    root: &Path,
    branch: &str,
    from: &str,
    timeout: Duration,
) -> Result<bool, String> {
    let g = Git::new(git, timeout);
    let refname = format!("refs/heads/{branch}");
    let args = [
        os("update-ref"),
        os("--no-deref"),
        os(&refname),
        os(from),
        os(""),
    ];
    let output = g.write_raw(root, &args)?;
    if output.success {
        return Ok(true);
    }
    match read(g, root, &refname)? {
        Some(_) => Ok(false),
        None => Err(failure(&args, &output)),
    }
}

/// The branch the integration worktree stays on for a merge into `branch`: a stage
/// branch `anthrex/<run>/stage-<n>` has the alias `anthrex/<run>/integration`
/// (decision 53); any other branch is its own.
pub fn alias_of(branch: &str) -> String {
    match branch.rsplit_once('/') {
        Some((run, last))
            if run.starts_with("anthrex/")
                && last
                    .strip_prefix("stage-")
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            format!("{run}/integration")
        }
        _ => branch.to_string(),
    }
}

#[cfg(test)]
#[path = "refs_tx_tests.rs"]
mod tests;
