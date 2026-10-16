//! Milestone 9.1 decision 12: verifying a proposal's tier commands, after M8b's three,
//! in the same fresh checkout and under the same confinement (`verify.rs`):
//! `build_check`, the module graph (it must be known), `module_test` and
//! `module_tests` for one real module, and `toolchain_id`. Blocking: called from
//! [`super::verify::run_commands`], which runs on `spawn_blocking`.
//!
//! A graph's JSON is one long line, longer than the output tail an engine command
//! keeps, so the graph command's stdout goes to a file in the command's own `TMPDIR`
//! (the confinement's, or this process's when unconfined), never into the checkout.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use proto::{ModuleNames, ProfileVerification, RepoProfile};

use super::proposal_tiers::module_command;
use super::verify::{Steps, record};
use crate::launch::shell_quote;
use crate::run::confine::ConfineSpec;
use crate::run::exec::{ShellOutcome, run_matching};
use crate::run::globs::path_module;
use crate::run::messages::summary;
use crate::run::tiers::graph::{from_cargo_metadata, from_command_json};
use crate::run::tiers::{GRAPH_TIMEOUT, GraphSource, ModuleGraph, TOOLCHAIN_TIMEOUT, TierProfile};

/// The command the built-in `cargo` graph runs (decision 9).
pub const CARGO_METADATA: &str = "cargo metadata --format-version 1 --no-deps --offline";
/// The most of a graph command's stdout that is read.
const GRAPH_BYTES_MAX: u64 = 16 * 1024 * 1024;

/// The graph command's bound: its own (decision 9), or verification's when shorter.
pub(super) fn graph_timeout(verify: Duration) -> Duration {
    GRAPH_TIMEOUT.min(verify)
}

/// `command`'s stdout, read from a file in its `TMPDIR`, with its outcome. Also the
/// runner of the tier executor's graph and toolchain commands (`run/driver/graph.rs`).
pub(crate) fn capture(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    confine: Option<&ConfineSpec>,
) -> (ShellOutcome, Option<String>) {
    let confinement = match confine.map(|spec| spec.for_checkout(dir)).transpose() {
        Ok(confinement) => confinement,
        Err(error) => return (ShellOutcome::refused(error), None),
    };
    let tmp = confinement
        .as_ref()
        .map_or_else(std::env::temp_dir, |c| c.tmp().to_path_buf());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let file = match graph_file(&tmp, nanos) {
        Ok((file, _)) => file,
        Err(error) => return (ShellOutcome::refused(error.to_string()), None),
    };
    let shell = format!(
        "{{ {command}\n}} > {}",
        shell_quote(&file.to_string_lossy())
    );
    let (outcome, _) = run_matching(dir, &shell, (env, &[]), timeout, None, confinement.as_ref());
    let mut text = String::new();
    let read =
        std::fs::File::open(&file).and_then(|f| f.take(GRAPH_BYTES_MAX).read_to_string(&mut text));
    let _ = std::fs::remove_file(&file);
    (outcome, read.ok().map(|_| text))
}

/// How many sequence numbers [`graph_file`] tries before it refuses (decision 37).
const GRAPH_FILE_TRIES: u32 = 16;
/// This process's capture sequence: two captures in one process at the same `nanos`
/// (the clock's resolution, or two threads at once) still get two names (C1, E1).
static GRAPH_SEQ: AtomicU64 = AtomicU64::new(0);

/// A new, empty capture file in `tmp`, created `create_new` with mode `0o600`, named
/// `anthrex-graph-<pid>-<nanos>-<seq>.json`. A name that exists already (a leftover, or
/// another process with a recycled pid) takes the next sequence number, up to
/// [`GRAPH_FILE_TRIES`] tries. The error names the last path tried.
fn graph_file(tmp: &Path, nanos: u128) -> io::Result<(PathBuf, File)> {
    let pid = std::process::id();
    let mut tries = 0;
    loop {
        let seq = GRAPH_SEQ.fetch_add(1, Ordering::Relaxed);
        let file = tmp.join(format!("anthrex-graph-{pid}-{nanos}-{seq}.json"));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&file)
        {
            Ok(handle) => return Ok((file, handle)),
            Err(error) => {
                tries += 1;
                if error.kind() != io::ErrorKind::AlreadyExists || tries >= GRAPH_FILE_TRIES {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("cannot create {}: {error}", file.display()),
                    ));
                }
            }
        }
    }
}

/// The directories under `dir` that match a `modules` pattern, relative, sorted.
pub(crate) fn module_dirs(dir: &Path, modules: &[String]) -> Vec<String> {
    walk(dir, modules, None).unwrap_or_default()
}

/// [`module_dirs`], or `None` once `deadline` has passed: milestone 9.18 decision 11,
/// for the lessons graph's profile fallback. The deadline is checked before each
/// `read_dir`, so one `read_dir` that hangs (a stalled network checkout) is still
/// unbounded; the graph request already runs on `spawn_blocking`, so it holds a
/// blocking thread, never a tokio worker.
pub(crate) fn module_dirs_until(
    dir: &Path,
    modules: &[String],
    deadline: Instant,
) -> Option<Vec<String>> {
    walk(dir, modules, Some(deadline))
}

/// Ruling C17: the one walk behind [`module_dirs`] (no deadline, so the tier path is
/// unchanged) and [`module_dirs_until`].
fn walk(dir: &Path, modules: &[String], deadline: Option<Instant>) -> Option<Vec<String>> {
    let mut found = Vec::new();
    for pattern in modules {
        let mut current = vec![String::new()];
        for component in pattern.trim_end_matches('/').split('/') {
            let literal = !component.contains(['*', '?', '[']);
            // Review round 2, minor 2: as in `globs::glob_module`, only a whole `*`
            // component is a wildcard; any other with a metacharacter matches nothing.
            let wildcard = component == "*";
            let mut next = Vec::new();
            for base in &current {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return None;
                }
                let Ok(entries) = std::fs::read_dir(dir.join(base)) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
                    // Review m1 and m3: a hidden name, one a shell would read as an
                    // option, and one with a glob character (which `path_module` never
                    // names, so no path would ever be in it) are no module.
                    let plain = !name.starts_with(['.', '-']) && !name.contains(['*', '?', '[']);
                    let matches = if literal {
                        name == component
                    } else {
                        wildcard && plain
                    };
                    if is_dir && matches {
                        next.push(if base.is_empty() {
                            name
                        } else {
                            format!("{base}/{name}")
                        });
                    }
                }
            }
            current = next;
        }
        found.extend(current);
    }
    found.sort();
    found.dedup();
    Some(found)
}

/// The module names decision 10 gives, sorted: from the graph with `cargo` names (so
/// `None` without one), from the checkout's directories with `dir` names.
fn module_names(
    dir: &Path,
    tiers: &TierProfile,
    modules: &[String],
    graph: Option<&ModuleGraph>,
) -> Option<Vec<String>> {
    let mut names: Vec<String> = match tiers.module_names {
        ModuleNames::Cargo => graph?
            .modules
            .iter()
            .filter(|(_, info)| {
                info.dir.as_ref().is_some_and(|d| {
                    path_module(&format!("{d}/Cargo.toml"), modules).as_ref() == Some(d)
                })
            })
            .map(|(name, _)| name.clone())
            .collect(),
        ModuleNames::Dir => module_dirs(dir, modules)
            .iter()
            .filter_map(|d| d.rsplit('/').next().map(str::to_string))
            .collect(),
    };
    names.retain(|name| !name.starts_with('-'));
    names.sort();
    names.dedup();
    Some(names)
}

/// Decision 12's tier checks, into `v`, in order. `env` is the environment M8b's
/// commands ran with; `timeout` bounds each command (the graph and the toolchain id
/// also by their own bounds). Each runs through `steps` (ruling C-27, I-2).
pub fn run_commands(
    dir: &Path,
    profile: &RepoProfile,
    (env, timeout, confine): (&[(String, String)], Duration, Option<&ConfineSpec>),
    steps: &Steps,
    v: &mut ProfileVerification,
) {
    let tiers = TierProfile::from_repo(profile);
    let plain = |command: &str, timeout: Duration| {
        let (outcome, _) = steps.run(dir, command, env, timeout, None, confine);
        let ok = outcome.ok;
        (outcome, ok)
    };
    v.build_check = profile.build_check.as_ref().map(|command| {
        let (outcome, ok) = plain(command, timeout);
        record(command, outcome, ok)
    });
    let mut graph = None;
    let graph_command = match &tiers.module_graph {
        GraphSource::None => None,
        GraphSource::Cargo => Some(CARGO_METADATA.to_string()),
        GraphSource::Command(command) => Some(command.clone()),
    };
    if let (Some(key), Some(command)) = (&profile.module_graph, graph_command) {
        let (mut outcome, stdout) =
            steps.capture(dir, &command, env, graph_timeout(timeout), confine);
        let parsed = match (&tiers.module_graph, stdout) {
            _ if !outcome.ok => Err(String::new()),
            (_, None) => Err("its output could not be read".to_string()),
            (GraphSource::Cargo, Some(json)) => from_cargo_metadata(&json),
            (_, Some(json)) => from_command_json(&json, &Default::default()),
        };
        let ok = match parsed {
            Ok(known) => {
                graph = Some(known);
                true
            }
            Err(reason) if reason.is_empty() => false,
            Err(reason) => {
                let note = format!("module graph unknown: {reason}");
                outcome.tail = match outcome.tail.as_str() {
                    "" => note,
                    tail => summary(&format!("{tail}\n{note}")),
                };
                false
            }
        };
        v.module_graph = Some(record(key, outcome, ok));
    }
    let names = module_names(dir, &tiers, &profile.modules, graph.as_ref());
    for (template, slot, each) in [
        (&profile.module_test, &mut v.module_test, false),
        (&profile.module_tests, &mut v.module_tests, true),
    ] {
        let (Some(template), Some(names)) = (template, &names) else {
            continue;
        };
        *slot = Some(match names.first() {
            None => record(
                template,
                ShellOutcome::refused(format!(
                    "no module directory matches modules ({})",
                    profile.modules.join(", ")
                )),
                false,
            ),
            Some(first) => {
                let command = module_command(profile, template, each, first);
                let (outcome, ok) = plain(&command, timeout);
                record(template, outcome, ok)
            }
        });
    }
    v.toolchain_id = profile.toolchain_id.as_ref().map(|command| {
        let (outcome, ok) = plain(command, TOOLCHAIN_TIMEOUT.min(timeout));
        record(command, outcome, ok)
    });
}

#[cfg(test)]
#[path = "verify_tiers_tests.rs"]
mod tests;
