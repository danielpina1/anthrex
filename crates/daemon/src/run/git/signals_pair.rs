//! Milestone 9.5 rulings RP-2, T16-7 and T16-8: a paired task's implementer's
//! test-weakening signals. Blocking; call only from `spawn_blocking` (AGENTS.md rule
//! 2). Every git call goes through [`super::Git`] (`--no-optional-locks`, the scrubbed
//! environment) and the reads of `signals.rs` (no attributes, no textconv).

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use super::signals::{
    DiffLimits, NoAttributes, SIGNALS_DIFF_BYTES, merge_base, read_signals, signal_paths,
};
use crate::run::tiers::{ClaimSignals, SIGNALS_MAX, Signal, SignalsSpec};

/// The implementer's signals. No merge commit is ever a base (T16-7), and each read is
/// limited by a pathspec, never filtered afterwards (T16-8 (a)), so each read's cap,
/// cut and `DiffTooLarge` are its own paths' and `more` is exact:
///
/// - the paths `start..red` touched (the test writer's) are read over `red..head`,
///   except a path the head holds exactly as the run head does when the run head
///   changed it since `start` (T16-8 (c): the run head brought that change);
/// - every other path is read with [`super::done_signals`]'s merge-base read
///   (`<merge-base of run_head and head>..head`), which no merge can move past the
///   implementer's own commits.
///
/// Deleted test files first, then one `DiffTooLarge`, then the rest; at most
/// [`SIGNALS_MAX`]. `base` is the merge base; `restore_from` names each deleted file's
/// restore base, `red` for a writer's path (T16-8 (b)).
pub fn pair_signals(
    git: &OsStr,
    worktree: &Path,
    (start, red, run_head, head): (&str, &str, &str, &str),
    spec: &SignalsSpec,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    if [start, red, run_head, head]
        .iter()
        .any(|r| r.starts_with('-'))
    {
        return Err(format!(
            "not a diff range: {start:?} {red:?} {run_head:?} {head:?}"
        ));
    }
    let attrs = NoAttributes::probe(git, worktree, timeout)?;
    let paths = |range: String, within: &[String]| {
        signal_paths(git, worktree, (&range, within), None, &attrs, timeout)
    };
    let writer = paths(format!("{start}..{red}"), &[])?;
    let literal: Vec<String> = writer.iter().map(|p| format!(":(literal){p}")).collect();
    let limits = DiffLimits {
        bytes: SIGNALS_DIFF_BYTES,
        timeout,
    };
    let base = merge_base(git, worktree, (run_head, head), timeout)?;
    let except: Vec<String> = (writer.iter())
        .map(|p| format!(":(exclude,literal){p}"))
        .collect();
    let mut own = read_signals(
        git,
        worktree,
        (base.clone(), head, &except),
        spec,
        limits,
        timeout,
    )?;
    let mut restore_from = BTreeMap::new();
    note_deleted(&own.list, &base, &mut restore_from);
    if writer.is_empty() {
        own.restore_from = restore_from;
        return Ok(own);
    }
    let differs = paths(format!("{run_head}..{head}"), &literal)?;
    let theirs = paths(format!("{start}..{run_head}"), &literal)?;
    let ours: Vec<String> = (writer.iter())
        .filter(|p| differs.contains(p) || !theirs.contains(p))
        .map(|p| format!(":(literal){p}"))
        .collect();
    let of_red = match ours.is_empty() {
        true => ClaimSignals::default(),
        false => read_signals(
            git,
            worktree,
            (red.to_string(), head, &ours),
            spec,
            limits,
            timeout,
        )?,
    };
    note_deleted(&of_red.list, red, &mut restore_from);
    let mut list: Vec<Signal> = own.list.into_iter().chain(of_red.list).collect();
    let first = |s: &Signal| match s {
        Signal::DeletedTestFile { .. } => 0,
        Signal::DiffTooLarge => 1,
        _ => 2,
    };
    list.sort_by_key(first);
    let mut seen = false;
    list.retain(|s| !matches!(s, Signal::DiffTooLarge) || !std::mem::replace(&mut seen, true));
    let over = list.len().saturating_sub(SIGNALS_MAX);
    list.truncate(SIGNALS_MAX);
    let more = (own.more.saturating_add(of_red.more))
        .saturating_add(u32::try_from(over).unwrap_or(u32::MAX));
    Ok(ClaimSignals {
        list,
        more,
        base,
        restore_from,
    })
}

/// Each deleted test file of `list` restores from `base`.
fn note_deleted(list: &[Signal], base: &str, into: &mut BTreeMap<String, String>) {
    for signal in list {
        if let Signal::DeletedTestFile { path } = signal {
            into.insert(path.clone(), base.to_string());
        }
    }
}
