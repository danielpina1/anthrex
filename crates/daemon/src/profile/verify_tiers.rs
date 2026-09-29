//! Milestone 9.1 decision 12: verifying a proposal's tier commands, after M8b's three,
//! in the same fresh checkout and under the same confinement (`verify.rs`):
//! `build_check`, the module graph (it must be known), `module_test` and
//! `module_tests` for one real module, and `toolchain_id`. Blocking: called from
//! [`super::verify::run_commands`], which runs on `spawn_blocking`.
//!
//! A graph's JSON is one long line, longer than the output tail an engine command
//! keeps, so the graph command's stdout goes to a file in the command's own `TMPDIR`
//! (the confinement's, or this process's when unconfined), never into the checkout.

use std::fs::OpenOptions;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use proto::{ModuleNames, ProfileVerification, RepoProfile};

use super::verify::record;
use crate::launch::shell_quote;
use crate::run::confine::ConfineSpec;
use crate::run::exec::{ShellOutcome, run_matching};
use crate::run::globs::path_module;
use crate::run::messages::summary;
use crate::run::tiers::command::{Placeholders, filter_expr, substitute};
use crate::run::tiers::graph::{from_cargo_metadata, from_command_json};
use crate::run::tiers::{
    GRAPH_TIMEOUT, GraphSource, ModuleGraph, Scope, TOOLCHAIN_TIMEOUT, TierProfile,
};

/// The command the built-in `cargo` graph runs (decision 9).
pub const CARGO_METADATA: &str = "cargo metadata --format-version 1 --no-deps --offline";
/// The most of a graph command's stdout that is read.
const GRAPH_BYTES_MAX: u64 = 16 * 1024 * 1024;

/// The graph command's bound: its own (decision 9), or verification's when shorter.
pub(super) fn graph_timeout(verify: Duration) -> Duration {
    GRAPH_TIMEOUT.min(verify)
}

/// One command in `dir` as `verify.rs` runs it: confined when `confine` is set, and
/// unrun (failed) when the checkout cannot be confined.
fn run(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    confine: Option<&ConfineSpec>,
) -> ShellOutcome {
    match confine.map(|spec| spec.for_checkout(dir)).transpose() {
        Ok(confinement) => run_matching(dir, command, env, timeout, None, confinement.as_ref()).0,
        Err(error) => ShellOutcome::refused(error),
    }
}

/// `command`'s stdout, read from a file in its `TMPDIR`, with its outcome.
fn capture(
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
    let file: PathBuf = tmp.join(format!("anthrex-graph-{}-{nanos}.json", std::process::id()));
    if let Err(error) = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&file)
    {
        let reason = format!("cannot create {}: {error}", file.display());
        return (ShellOutcome::refused(reason), None);
    }
    let shell = format!(
        "{{ {command}\n}} > {}",
        shell_quote(&file.to_string_lossy())
    );
    let (outcome, _) = run_matching(dir, &shell, env, timeout, None, confinement.as_ref());
    let mut text = String::new();
    let read =
        std::fs::File::open(&file).and_then(|f| f.take(GRAPH_BYTES_MAX).read_to_string(&mut text));
    let _ = std::fs::remove_file(&file);
    (outcome, read.ok().map(|_| text))
}

/// The command M8b's `check` verification runs: for a tiered profile, `check` with
/// `{filter:…}` empty and `{shard}`/`{shards}` as 1 of 1, so the whole suite runs;
/// otherwise `check` as written, as M8a runs it.
pub fn check_command(profile: &RepoProfile, check: &str) -> String {
    if !TierProfile::from_repo(profile).is_tiered() {
        return check.to_string();
    }
    let values = Placeholders {
        shard: Some((1, 1)),
        ..Placeholders::default()
    };
    substitute(check, &values)
}

/// The directories under `dir` that match a `modules` pattern, relative, sorted.
fn module_dirs(dir: &Path, modules: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for pattern in modules {
        let mut current = vec![String::new()];
        for component in pattern.trim_end_matches('/').split('/') {
            let literal = !component.contains(['*', '?', '[']);
            let matcher = globset::Glob::new(component).map(|g| g.compile_matcher());
            let mut next = Vec::new();
            for base in &current {
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
                    let matches = match &matcher {
                        _ if literal => name == component,
                        Ok(m) => plain && m.is_match(&name),
                        Err(_) => false,
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
    found
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
/// also by their own bounds).
pub fn run_commands(
    dir: &Path,
    profile: &RepoProfile,
    env: &[(String, String)],
    timeout: Duration,
    confine: Option<&ConfineSpec>,
    v: &mut ProfileVerification,
) {
    let tiers = TierProfile::from_repo(profile);
    let plain = |command: &str, timeout: Duration| {
        let outcome = run(dir, command, env, timeout, confine);
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
        let (mut outcome, stdout) = capture(dir, &command, env, graph_timeout(timeout), confine);
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
    let filter = filter_expr(&tiers, Scope::Gate, false);
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
                let values = Placeholders {
                    module: (!each).then(|| first.clone()),
                    modules: each.then(|| vec![first.clone()]),
                    filter: filter.clone(),
                    ..Placeholders::default()
                };
                let (outcome, ok) = plain(&substitute(template, &values), timeout);
                record(template, outcome, ok)
            }
        });
    }
    v.toolchain_id = profile.toolchain_id.as_ref().map(|command| {
        let (outcome, ok) = plain(command, TOOLCHAIN_TIMEOUT.min(timeout));
        record(command, outcome, ok)
    });
}
