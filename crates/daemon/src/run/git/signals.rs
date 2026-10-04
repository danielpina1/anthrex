//! Milestone 9.1 decision 40 and ruling C-20: the git reads behind a claim's
//! test-weakening signals. Blocking; call only from `spawn_blocking` (AGENTS.md rule
//! 2). Every call goes through [`Git`], so `--no-optional-locks`, the hooks-off read
//! flags and the scrubbed environment hold (AGENTS.md rules 10 and 11).
//!
//! The worker controls the checkout these reads run in, so none may be steered by it
//! (ruling C-20): each passes `-c core.attributesFile=/dev/null` and, when git has it
//! (2.40 and later), `--attr-source=<the empty tree>`, so no `.gitattributes`,
//! committed or untracked, can mark a file binary or give it a diff driver; each diff
//! passes `--text`, `--no-ext-diff`, `--no-textconv` and `--no-color`, and the grep
//! `--text`, `--no-textconv` and `--no-color`. `$GIT_DIR/info/attributes` is still read
//! without `--attr-source`; `--text` keeps even that from hiding a file's lines.

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use super::tiers::range_words;
use super::{DIFF_FLAGS, Git, PATCH_PREFIXES, failure, nul_fields, os};
use crate::run::tiers::weakening::{self, SignalInput};
use crate::run::tiers::{ClaimSignals, SIGNALS_MAX, Signal, SignalsSpec};
use crate::worktree::{WorktreeError, run_git_head_tail};

/// At most this much of decision 40's `-U0` diff is read; the rest is read and dropped
/// ([`Zero::Read`] says it was cut).
pub const SIGNALS_DIFF_BYTES: usize = 16 * 1024 * 1024;

/// At most this many paths per `git grep` of [`cfg_test_files`], so no argument list
/// grows past the system's limit.
const GREP_PATHS_MAX: usize = 256;

/// The global options ahead of every signal read (see the module doc).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoAttributes(Vec<String>);

impl NoAttributes {
    /// Reads the empty tree's id in `dir`'s object format, then probes whether this git
    /// takes `--attr-source`.
    pub fn probe(git: &OsStr, dir: &Path, timeout: Duration) -> Result<Self, String> {
        let g = Git::new(git, timeout);
        let empty = g.ok(
            dir,
            &[os("hash-object"), os("-t"), os("tree"), os("/dev/null")],
        )?;
        let mut flags = vec![
            "-c".to_string(),
            "core.attributesFile=/dev/null".to_string(),
        ];
        let source = format!("--attr-source={}", empty.trim());
        let probe = [os(&source), os("rev-parse"), os("--git-dir")];
        if g.read(dir, &probe)?.success {
            flags.push(source);
        }
        Ok(NoAttributes(flags))
    }

    fn args<'a>(&'a self, rest: &[&'a OsStr]) -> Vec<&'a OsStr> {
        let mut args: Vec<&OsStr> = self.0.iter().map(|f| os(f)).collect();
        args.extend_from_slice(rest);
        args
    }
}

/// What a `-U0` read gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Zero {
    /// The diff, or its first [`SIGNALS_DIFF_BYTES`] when `cut`.
    Read { text: String, cut: bool },
    /// It did not finish within its timeout.
    TimedOut,
}

/// `git diff -U0 --no-renames --text <range>` in `dir` (with [`DIFF_FLAGS`] and the
/// `a/`/`b/` prefixes), keeping at most `bytes`.
pub fn unified_zero(
    git: &OsStr,
    dir: &Path,
    range: &str,
    attrs: &NoAttributes,
    bytes: usize,
    timeout: Duration,
) -> Result<Zero, String> {
    let g = Git::new(git, timeout);
    let mut rest = vec![os("diff")];
    rest.extend(DIFF_FLAGS.map(os));
    rest.extend(PATCH_PREFIXES.map(os));
    rest.extend([os("--text"), os("-U0"), os("--no-renames")]);
    rest.extend(range_words(range)?.into_iter().map(os));
    rest.push(os("--"));
    let args = Git::unhooked(&attrs.args(&rest));
    let ran = run_git_head_tail(g.program, dir, &args, Instant::now() + timeout, bytes, 0);
    let (output, kept) = match ran {
        Ok(ran) => ran,
        Err(WorktreeError::TimedOut { .. }) => return Ok(Zero::TimedOut),
        Err(error) => return Err(error.to_string()),
    };
    if !output.success {
        return Err(failure(&args, &output));
    }
    Ok(Zero::Read {
        text: String::from_utf8_lossy(&kept.head).into_owned(),
        cut: kept.dropped(),
    })
}

/// The paths `range` changes (`git diff --name-only -z --no-renames`), only those of
/// `filter` (`--diff-filter=<filter>`) when given, in git's order.
pub fn signal_paths(
    git: &OsStr,
    dir: &Path,
    range: &str,
    filter: Option<&str>,
    attrs: &NoAttributes,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    let g = Git::new(git, timeout);
    let filter = filter.map(|f| format!("--diff-filter={f}"));
    let mut rest = vec![os("diff")];
    rest.extend(DIFF_FLAGS.map(os));
    rest.extend([
        os("--text"),
        os("--name-only"),
        os("-z"),
        os("--no-renames"),
    ]);
    rest.extend(filter.as_deref().map(os));
    rest.extend(range_words(range)?.into_iter().map(os));
    rest.push(os("--"));
    let out = g.ok(dir, &attrs.args(&rest))?;
    Ok(nul_fields(&out).map(str::to_string).collect())
}

/// Which of `files` hold `#[cfg(test)]` at `rev`
/// (`git grep -l -z -F '#[cfg(test)]' <rev> -- <files>`, each path literal), in git's
/// order, read in groups of [`GREP_PATHS_MAX`].
pub fn cfg_test_files(
    git: &OsStr,
    dir: &Path,
    rev: &str,
    files: &[String],
    attrs: &NoAttributes,
    timeout: Duration,
) -> Result<Vec<String>, String> {
    if rev.starts_with('-') {
        return Err(format!("not a commit: {rev:?}"));
    }
    let g = Git::new(git, timeout);
    let prefix = format!("{rev}:");
    let mut found = Vec::new();
    for group in files.chunks(GREP_PATHS_MAX) {
        let literal: Vec<String> = group.iter().map(|f| format!(":(literal){f}")).collect();
        let mut rest = vec![
            os("grep"),
            os("--no-color"),
            os("--text"),
            os("--no-textconv"),
            os("-l"),
            os("-z"),
            os("-F"),
            os("--no-recurse-submodules"),
            os("-e"),
            os("#[cfg(test)]"),
            os(rev),
            os("--"),
        ];
        rest.extend(literal.iter().map(|f| os(f)));
        let args = attrs.args(&rest);
        let output = g.read(dir, &args)?;
        // `git grep` exits 1, with nothing on stderr, when nothing matches.
        if !output.success && !output.stderr.trim().is_empty() {
            return Err(failure(&args, &output));
        }
        found.extend(
            nul_fields(&output.stdout)
                .map(|name| name.strip_prefix(&prefix).unwrap_or(name).to_string()),
        );
    }
    Ok(found)
}

/// How much of the `-U0` diff is read, and for how long.
#[derive(Debug, Clone, Copy)]
pub struct DiffLimits {
    pub bytes: usize,
    pub timeout: Duration,
}

/// Decision 40 and ruling C-20: the claim's signals, read over
/// `<merge-base of run_head and head>..<head>` (decision 15's range; the base is
/// returned for the restore command of decision 41). A cut or timed-out `-U0` diff
/// takes its deleted files from `--diff-filter=DT` and adds `DiffTooLarge`.
pub fn done_signals(
    git: &OsStr,
    worktree: &Path,
    run_head: &str,
    head: &str,
    spec: &SignalsSpec,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    let limits = DiffLimits {
        bytes: SIGNALS_DIFF_BYTES,
        timeout,
    };
    done_signals_with(git, worktree, (run_head, head), spec, limits, timeout)
}

/// [`done_signals`] with the `-U0` read's limits given.
pub fn done_signals_with(
    git: &OsStr,
    worktree: &Path,
    (run_head, head): (&str, &str),
    spec: &SignalsSpec,
    limits: DiffLimits,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    if run_head.starts_with('-') || head.starts_with('-') {
        return Err(format!("not a diff range: {run_head:?} {head:?}"));
    }
    let g = Git::new(git, timeout);
    let base = g
        .ok(worktree, &[os("merge-base"), os(run_head), os(head)])?
        .trim()
        .to_string();
    read_signals(git, worktree, (base, head), spec, limits, timeout)
}

/// Controller ruling C-21 (2): a sync task's signals, read from `base` (its conflicted
/// tree) to `head`, so the lower stage's changes its merge carries are never its own.
pub fn done_signals_from(
    git: &OsStr,
    worktree: &Path,
    (base, head): (&str, &str),
    spec: &SignalsSpec,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    if base.starts_with('-') || head.starts_with('-') {
        return Err(format!("not a diff range: {base:?} {head:?}"));
    }
    let limits = DiffLimits {
        bytes: SIGNALS_DIFF_BYTES,
        timeout,
    };
    read_signals(
        git,
        worktree,
        (base.to_string(), head),
        spec,
        limits,
        timeout,
    )
}

/// The signals of `<base>..<head>` (`base` may be a tree).
fn read_signals(
    git: &OsStr,
    worktree: &Path,
    (base, head): (String, &str),
    spec: &SignalsSpec,
    limits: DiffLimits,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    let attrs = NoAttributes::probe(git, worktree, timeout)?;
    let range = format!("{base}..{head}");
    let zero = unified_zero(git, worktree, &range, &attrs, limits.bytes, limits.timeout)?;
    let rs: Vec<String> = signal_paths(git, worktree, &range, None, &attrs, timeout)?
        .into_iter()
        .filter(|p| p.ends_with(".rs"))
        .collect();
    let grep = |rev: &str| match rs.is_empty() {
        true => Ok(Vec::new()),
        false => cfg_test_files(git, worktree, rev, &rs, &attrs, timeout),
    };
    let (cfg_base, cfg_head) = (grep(&base)?, grep(head)?);
    let (diff, cut) = match zero {
        Zero::Read { text, cut } => (text, cut),
        Zero::TimedOut => (String::new(), true),
    };
    let deleted = match cut {
        true => Some(signal_paths(
            git,
            worktree,
            &range,
            Some("DT"),
            &attrs,
            timeout,
        )?),
        false => None,
    };
    let (list, more) = weakening::read(&SignalInput {
        diff: &diff,
        cut: deleted.as_deref(),
        cfg_base: &cfg_base,
        cfg_head: &cfg_head,
        test_paths: &spec.test_paths,
        skip_markers: &spec.skip_markers,
    });
    Ok(ClaimSignals {
        list,
        more: u32::try_from(more).unwrap_or(u32::MAX),
        base,
    })
}

/// Milestone 9.5 rulings RP-2 and T16-1: a paired task's implementer's signals. While no
/// merge has landed on the checkout's first-parent line after `red` (the engine's
/// refresh or merge-queue hand-back), they are read over `red..head`. Once one has, the
/// newest such merge is the base, so the run head's changes it carries are never the
/// implementer's; the paths `start..red` touched (the writer's test) are still read
/// over `red..head`, those paths only, in place of the merge-based read's. Deleted test
/// files and `DiffTooLarge` first; at most [`SIGNALS_MAX`], `more` counting both reads'
/// overflow (the merge-based read's may include a dropped signal on a red path).
pub fn pair_signals(
    git: &OsStr,
    worktree: &Path,
    (start, red, head): (&str, &str, &str),
    spec: &SignalsSpec,
    timeout: Duration,
) -> Result<ClaimSignals, String> {
    if [start, red, head].iter().any(|r| r.starts_with('-')) {
        return Err(format!("not a diff range: {start:?} {red:?} {head:?}"));
    }
    let g = Git::new(git, timeout);
    let range = format!("{red}..{head}");
    let merges = [os("rev-list"), os("--first-parent"), os("--merges")];
    let mut args = merges.to_vec();
    args.extend([os("-n"), os("1"), os(&range)]);
    let merge = g.ok(worktree, &args)?.trim().to_string();
    if merge.is_empty() {
        return done_signals_from(git, worktree, (red, head), spec, timeout);
    }
    let attrs = NoAttributes::probe(git, worktree, timeout)?;
    let red_paths = signal_paths(
        git,
        worktree,
        &format!("{start}..{red}"),
        None,
        &attrs,
        timeout,
    )?;
    let on_red = |s: &Signal| path_of(s).is_some_and(|p| red_paths.iter().any(|r| r == p));
    let since = done_signals_from(git, worktree, (&merge, head), spec, timeout)?;
    let of_red = done_signals_from(git, worktree, (red, head), spec, timeout)?;
    let mut list: Vec<Signal> = (since.list.into_iter().filter(|s| !on_red(s)))
        .chain(of_red.list.into_iter().filter(|s| on_red(s)))
        .collect();
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
    let more = (since.more.saturating_add(of_red.more))
        .saturating_add(u32::try_from(over).unwrap_or(u32::MAX));
    Ok(ClaimSignals {
        list,
        more,
        base: merge,
    })
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
