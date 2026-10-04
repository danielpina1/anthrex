//! Milestone 9.5 rulings RP-2, T16-7 to T16-10: a paired task's implementer's
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
use super::{Git, LARGE_OUTPUT_BYTES, nul_fields, os};
use crate::run::tiers::{ClaimSignals, SIGNALS_MAX, Signal, SignalsSpec};

/// Ruling T16-9 (2): at most this many paths go on one command line as pathspecs;
/// past it a read runs over every path and is filtered afterwards.
pub const PATHSPEC_MAX: usize = 256;

/// The implementer's signals. No merge commit is ever a base (T16-7), and each read is
/// limited by a pathspec, never filtered afterwards (T16-8 (a)), while the test writer
/// changed at most [`PATHSPEC_MAX`] paths; past that, each read covers every path and is
/// filtered, and `unlimited` says how many paths there were (T16-9 (2)).
///
/// - The paths `start..red` touched (the test writer's) are read over `red..head`,
///   except a path the run head changed since `start` whose head blob is exactly the
///   clean 3-way merge of `start`, `red` and the run head there (T16-10: the run head
///   brought the rest). A conflict there, a head that differs from the merge, or a git
///   without `merge-tree --merge-base` keeps the path.
/// - Every other path is read with [`super::done_signals`]'s merge-base read
///   (`<merge-base of run_head and head>..head`), which no merge can move past the
///   implementer's own commits.
///
/// Deleted test files first, then one `DiffTooLarge`, then the rest, the writer's paths
/// ahead of the others within each rank (so the cap never pushes a writer-test loss
/// into `more`); at most [`SIGNALS_MAX`]. `base` is the merge base; `restore_from`
/// names each deleted file's restore base, `red` for a writer's path (T16-8 (b)).
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
    // The paths `range` changes among `set` (all of them when `set` is `None`).
    let names = |range: String, set: Option<&[String]>| -> Result<Vec<String>, String> {
        let within = |s: &[String]| s.len() <= PATHSPEC_MAX;
        let spec = match set {
            Some(s) if within(s) => literal(s),
            _ => Vec::new(),
        };
        let found = signal_paths(git, worktree, (&range, &spec), None, &attrs, timeout)?;
        Ok(match set {
            Some(s) if !within(s) => found.into_iter().filter(|p| s.contains(p)).collect(),
            _ => found,
        })
    };
    let writer = names(format!("{start}..{red}"), None)?;
    let limited = writer.len() <= PATHSPEC_MAX;
    let limits = DiffLimits {
        bytes: SIGNALS_DIFF_BYTES,
        timeout,
    };
    let read = |from: String, pathspec: &[String]| {
        read_signals(git, worktree, (from, head, pathspec), spec, limits, timeout)
    };
    let base = merge_base(git, worktree, (run_head, head), timeout)?;
    let mut own = match limited {
        true => {
            let except: Vec<String> = (writer.iter())
                .map(|p| format!(":(exclude,literal){p}"))
                .collect();
            read(base.clone(), &except)?
        }
        false => keep_on(read(base.clone(), &[])?, &writer, false),
    };
    let mut restore_from = BTreeMap::new();
    note_deleted(&own.list, &base, &mut restore_from);
    if writer.is_empty() {
        own.restore_from = restore_from;
        return Ok(own);
    }
    let changed = names(format!("{start}..{run_head}"), Some(&writer))?;
    let brought = match changed.is_empty() {
        true => Vec::new(),
        false => match clean_merge(git, worktree, (start, red, run_head), timeout)? {
            Some((tree, conflicted)) => {
                let differs = names(format!("{tree}..{head}"), Some(&changed))?;
                (changed.into_iter())
                    .filter(|p| !conflicted.contains(p) && !differs.contains(p))
                    .collect()
            }
            None => Vec::new(),
        },
    };
    let ours: Vec<String> = (writer.iter())
        .filter(|p| !brought.contains(p))
        .cloned()
        .collect();
    let of_red = match (ours.is_empty(), limited) {
        (true, _) => ClaimSignals::default(),
        (false, true) => read(red.to_string(), &literal(&ours))?,
        (false, false) => keep_on(read(red.to_string(), &[])?, &ours, true),
    };
    note_deleted(&of_red.list, red, &mut restore_from);
    let mut list: Vec<Signal> = of_red.list.into_iter().chain(own.list).collect();
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
        unlimited: match limited {
            true => 0,
            false => u32::try_from(writer.len()).unwrap_or(u32::MAX),
        },
    })
}

/// `paths` as literal pathspecs.
fn literal(paths: &[String]) -> Vec<String> {
    paths.iter().map(|p| format!(":(literal){p}")).collect()
}

/// `signals` with only those on `paths` (`inside`) or only those off them; a
/// `DiffTooLarge` stays either way. `more` may then count a signal the filter dropped
/// (the pre-T16-8 overcount, accepted past [`PATHSPEC_MAX`]).
fn keep_on(mut signals: ClaimSignals, paths: &[String], inside: bool) -> ClaimSignals {
    signals
        .list
        .retain(|s| path_of(s).is_none_or(|p| paths.iter().any(|w| w == p) == inside));
    signals
}

/// The path a signal is on (`DiffTooLarge` has none).
fn path_of(signal: &Signal) -> Option<&str> {
    match signal {
        Signal::DeletedTestFile { path }
        | Signal::SkipMarker { path, .. }
        | Signal::AssertionLoss { path, .. }
        | Signal::TestCodeRemoved { path, .. } => Some(path),
        Signal::DiffTooLarge => None,
    }
}

/// Ruling T16-10: `git merge-tree --write-tree --merge-base=<start> --name-only
/// --no-messages -z <red> <run_head>`, the clean 3-way merge's tree and its conflicted
/// paths. `None` when there is no such answer: a git before 2.40 (`--merge-base`; 2.38
/// is the run engine's minimum), any other failure, or more names than were kept.
/// `merge-tree` writes objects only (no index, no ref, no working tree).
fn clean_merge(
    git: &OsStr,
    worktree: &Path,
    (start, red, run_head): (&str, &str, &str),
    timeout: Duration,
) -> Result<Option<(String, Vec<String>)>, String> {
    let base = format!("--merge-base={start}");
    let args = [
        os("merge-tree"),
        os("--write-tree"),
        os(&base),
        os("--name-only"),
        os("--no-messages"),
        os("-z"),
        os(red),
        os(run_head),
    ];
    let g = Git::new(git, timeout);
    let (output, kept) = g.read_head(worktree, &args, LARGE_OUTPUT_BYTES)?;
    if kept.dropped() {
        return Ok(None);
    }
    let text = String::from_utf8_lossy(&kept.head);
    let mut fields = nul_fields(&text);
    let is_tree = |t: &str| t.len() >= 40 && t.bytes().all(|b| b.is_ascii_hexdigit());
    // Exit 0 is clean, exit 1 a conflict with its names after the tree; anything else
    // (an unknown option prints only to stderr) is no answer.
    Ok(match fields.next() {
        Some(tree) if is_tree(tree) => {
            let conflicted = match output.success {
                true => Vec::new(),
                false => fields.map(str::to_string).collect(),
            };
            Some((tree.to_string(), conflicted))
        }
        _ => None,
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
