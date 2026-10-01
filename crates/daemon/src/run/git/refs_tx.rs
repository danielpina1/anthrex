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

/// What a ref transaction did: every ref moved, or none did because `Moved` (the
/// first ref, in the order given, that was not at the value expected) differed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Swap {
    Done,
    Moved(String),
}

/// One `git update-ref --no-deref --stdin` transaction (`start`, a `verify <ref> <old>`
/// per entry of `verifies`, an `update <ref> <new> <old>` per entry of `updates`,
/// `prepare`, `commit`): every update lands and every verified ref is where it was
/// read, or nothing moves. `Swap::Moved` names the ref that differed (controller
/// ruling C-15, M-2), read after git refused. `--no-deref`: a ref made a symbolic ref is
/// replaced, never followed onto the ref it names (as `cas_update`, M8a fix round 4).
/// Each `old` must be a commit id; a ref that does not exist never matches.
pub fn cas(
    git: &OsStr,
    root: &Path,
    updates: &[(String, String, String)],
    verifies: &[(String, String)],
    timeout: Duration,
) -> Result<Swap, String> {
    let mut lines = Vec::new();
    for (refname, old) in verifies {
        lines.push(format!("verify {refname} {old}"));
    }
    for (refname, new, old) in updates {
        lines.push(format!("update {refname} {new} {old}"));
    }
    let expected: Vec<(&str, &str)> = verifies
        .iter()
        .map(|(r, o)| (r.as_str(), o.as_str()))
        .chain(updates.iter().map(|(r, _, o)| (r.as_str(), o.as_str())))
        .collect();
    transaction(Git::new(git, timeout), root, &lines, &expected)
}

/// Controller ruling C-15 (I-1): after a `--rebaseline` of a multi-stage run,
/// `integration` goes back to the alias of the highest stage. In one transaction, the
/// commit it was at is kept under `salvage` (created, never overwritten) and the
/// branch moves from `old` (the value read) to `new`. Never a force, never the base.
pub fn salvage_and_move(
    git: &OsStr,
    root: &Path,
    salvage: &str,
    branch: &str,
    (new, old): (&str, &str),
    verifies: &[(String, String)],
    timeout: Duration,
) -> Result<Swap, String> {
    let refname = format!("refs/heads/{branch}");
    let mut lines: Vec<String> = verifies
        .iter()
        .map(|(r, o)| format!("verify {r} {o}"))
        .collect();
    lines.push(format!("create {salvage} {old}"));
    lines.push(format!("update {refname} {new} {old}"));
    let mut expected: Vec<(&str, &str)> = verifies
        .iter()
        .map(|(r, o)| (r.as_str(), o.as_str()))
        .collect();
    expected.push((refname.as_str(), old));
    let g = Git::new(git, timeout);
    match transaction(g, root, &lines, &expected) {
        Ok(swap) => Ok(swap),
        // The branch is where it was read, so the salvage ref (never overwritten) was
        // the obstacle.
        Err(error) => match read(g, root, salvage)? {
            Some(_) => Ok(Swap::Moved(salvage.to_string())),
            None => Err(error),
        },
    }
}

/// Controller ruling C-16: `base`, or the first `<base>-<n>` from 2 that no ref holds,
/// so two salvages in one second never collide.
pub fn free_ref(git: &OsStr, root: &Path, base: &str, timeout: Duration) -> Result<String, String> {
    let g = Git::new(git, timeout);
    let mut name = base.to_string();
    for n in 2.. {
        if read(g, root, &name)?.is_none() {
            break;
        }
        name = format!("{base}-{n}");
    }
    Ok(name)
}

/// `lines` between `start` and `prepare`/`commit`, with `expected` the refs to read
/// back when git refuses.
fn transaction(
    g: Git<'_>,
    root: &Path,
    lines: &[String],
    expected: &[(&str, &str)],
) -> Result<Swap, String> {
    let mut input = String::from("start\n");
    for line in lines {
        // A line break would be a second command; git refuses such a ref name
        // anyway, so this only keeps the transaction what it says it is.
        if line.contains(['\n', '\0']) || line.split(' ').any(str::is_empty) {
            return Err(format!("refusing a ref transaction line: {line:?}"));
        }
        input.push_str(line);
        input.push('\n');
    }
    input.push_str("prepare\ncommit\n");
    let args = [os("update-ref"), os("--no-deref"), os("--stdin")];
    let output = g.write_input_raw(root, &args, input.as_bytes())?;
    if output.success {
        return Ok(Swap::Done);
    }
    // git's refusal names the mismatch in words that vary by version; the refs
    // themselves say which one was not where it was expected.
    for (refname, old) in expected {
        // A symbolic ref whose target is at `old` is never why git refused: `verify`
        // and `update --no-deref` both take it (controller ruling C-16 (5)).
        if read(g, root, refname)?.as_deref() != Some(*old) {
            return Ok(Swap::Moved(refname.to_string()));
        }
    }
    Err(failure(&args, &output))
}

/// Controller ruling C-15 (M-4): the ref `refname` names when it is a symbolic ref
/// (`git symbolic-ref -q`), else `None`. A run ref never is one; one that is was made
/// so by something else, and counts as moved.
pub fn symbolic(
    git: &OsStr,
    root: &Path,
    refname: &str,
    timeout: Duration,
) -> Result<Option<String>, String> {
    symbolic_in(Git::new(git, timeout), root, refname)
}

pub(crate) fn symbolic_in(
    g: Git<'_>,
    root: &Path,
    refname: &str,
) -> Result<Option<String>, String> {
    let output = g.read(root, &[os("symbolic-ref"), os("-q"), os(refname)])?;
    let target = output.stdout.trim();
    Ok((output.success && !target.is_empty()).then(|| target.to_string()))
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
