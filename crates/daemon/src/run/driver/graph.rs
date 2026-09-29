//! Milestone 9.1 decisions 9 and 11, the I/O half: a tier job's module graph (`cargo
//! metadata`, or the profile's graph command) with its cache, and the run's toolchain
//! id. **Blocking**: every function here runs git, a shell command or file I/O, so call
//! it only on the tier job's blocking thread (`spawn_blocking`), never under
//! `daemon::lock` or the engine lock (AGENTS.md rules 2 and 10).
//!
//! Commands run as profile verification runs them ([`capture`]): in the checkout,
//! confined when `confine` is set, their stdout read from a file in the command's own
//! `TMPDIR`, and their whole process group killed on timeout (`exec::run_matching`).
//! Git runs through [`Git`], so `--no-optional-locks` and a scrubbed environment hold
//! (AGENTS.md rule 11).
//!
//! The cache, `<repo_dir>/module-graph.json`, holds at most [`GRAPH_CACHE_MAX`] known
//! graphs, each under the FNV-1a of what the graph is read from: every tracked
//! `Cargo.toml` and `Cargo.lock` for `cargo` (path and content), the command text, the
//! `manifests` files and the module directories for a command. It is advisory: a hit
//! skips the command, and a missing or corrupt file is simply replaced.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use proto::ModuleNames;
use serde::{Deserialize, Serialize};

use crate::profile::store::{Fnv1a64, fingerprint, write_atomic};
use crate::profile::verify_tiers::{CARGO_METADATA, capture, module_dirs};
use crate::run::confine::ConfineSpec;
use crate::run::exec::ShellOutcome;
use crate::run::git::{Git, os};
use crate::run::tiers::graph::{from_cargo_metadata, from_command_json};
use crate::run::tiers::{
    GRAPH_TIMEOUT, GraphSource, GraphState, ModuleGraph, TOOLCHAIN_TIMEOUT, TierProfile,
};

/// The graph cache's file in the repository's data directory (decision 9).
pub(crate) const GRAPH_CACHE_FILE: &str = "module-graph.json";
/// The most graphs the cache keeps; the oldest goes first.
pub(crate) const GRAPH_CACHE_MAX: usize = 32;
/// The largest cache file that is read; a larger one is replaced.
const GRAPH_CACHE_BYTES_MAX: u64 = 64 * 1024 * 1024;
/// A toolchain id that could not be read: such a job neither reads nor writes the
/// result cache (decision 11).
pub(crate) const TOOLCHAIN_UNKNOWN: &str = "unknown";

/// How a command is run with its stdout captured: [`capture`] in the daemon, a stub
/// in the tests that check what is handed to it.
pub(crate) type Runner<'a> = &'a dyn Fn(
    &Path,
    &str,
    &[(String, String)],
    Duration,
    Option<&ConfineSpec>,
) -> (ShellOutcome, Option<String>);

/// One cache entry: a known graph under its key (16 hex digits).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CacheEntry {
    pub key: String,
    pub graph: ModuleGraph,
}

/// Decision 9: the module graph of the checkout `dir`, from the cache or by running
/// `cargo metadata` or the graph command there under `confine`, bounded by
/// [`GRAPH_TIMEOUT`]. `env` is the job's command environment; `manifests` the
/// profile's (a command graph's cache key). Any failure is [`GraphState::Unknown`]
/// with its reason. Blocking thread only.
#[allow(clippy::too_many_arguments)]
pub(crate) fn module_graph(
    git: &OsStr,
    dir: &Path,
    repo_dir: &Path,
    tiers: &TierProfile,
    modules: &[String],
    manifests: &[String],
    env: &[(String, String)],
    confine: Option<&ConfineSpec>,
) -> GraphState {
    module_graph_with(
        git,
        dir,
        repo_dir,
        tiers,
        (modules, manifests),
        env,
        confine,
        GRAPH_TIMEOUT,
        &capture,
    )
}

/// [`module_graph`] with its bound and its runner given (the tests shorten the one and
/// count the other).
#[allow(clippy::too_many_arguments)]
pub(super) fn module_graph_with(
    git: &OsStr,
    dir: &Path,
    repo_dir: &Path,
    tiers: &TierProfile,
    (modules, manifests): (&[String], &[String]),
    env: &[(String, String)],
    confine: Option<&ConfineSpec>,
    timeout: Duration,
    run: Runner<'_>,
) -> GraphState {
    let (command, label) = match &tiers.module_graph {
        GraphSource::None => return GraphState::Unknown("module_graph is none".to_string()),
        GraphSource::Cargo => (CARGO_METADATA.to_string(), "cargo metadata"),
        GraphSource::Command(command) => (command.clone(), "the graph command"),
    };
    // A command graph's modules are named by their directories (decision 10).
    let dirs: BTreeMap<String, String> = match (&tiers.module_graph, tiers.module_names) {
        (GraphSource::Command(_), ModuleNames::Dir) => module_dirs(dir, modules)
            .into_iter()
            .filter_map(|d| Some((d.rsplit('/').next()?.to_string(), d.clone())))
            .collect(),
        _ => BTreeMap::new(),
    };
    let key = cache_key(git, dir, &tiers.module_graph, manifests, &dirs, timeout);
    let cache = repo_dir.join(GRAPH_CACHE_FILE);
    if let Some(graph) = key.as_ref().and_then(|key| cache_get(&cache, key)) {
        return GraphState::Known(graph);
    }
    let (outcome, stdout) = run(dir, &command, env, timeout, confine);
    if !outcome.ok {
        return GraphState::Unknown(failure(label, &outcome));
    }
    let Some(json) = stdout else {
        return GraphState::Unknown(format!("{label}'s output could not be read"));
    };
    let parsed = match &tiers.module_graph {
        GraphSource::Cargo => from_cargo_metadata(&json),
        _ => from_command_json(&json, &dirs),
    };
    match parsed {
        Ok(graph) => {
            if let Some(key) = key {
                cache_put(&cache, &key, &graph);
            }
            GraphState::Known(graph)
        }
        Err(reason) => GraphState::Unknown(reason),
    }
}

/// Decision 11: the toolchain id of the checkout `dir`, the 16-hex FNV-1a of
/// `command`'s exit code and stdout, or [`TOOLCHAIN_UNKNOWN`] when it fails or runs
/// past [`TOOLCHAIN_TIMEOUT`]. Blocking thread only.
pub(crate) fn toolchain(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    confine: Option<&ConfineSpec>,
) -> String {
    toolchain_with(dir, command, env, confine, TOOLCHAIN_TIMEOUT, &capture)
}

/// [`toolchain`] with its bound and its runner given.
pub(super) fn toolchain_with(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    confine: Option<&ConfineSpec>,
    timeout: Duration,
    run: Runner<'_>,
) -> String {
    let (outcome, stdout) = run(dir, command, env, timeout, confine);
    match (outcome.ok, outcome.code, stdout) {
        (true, Some(code), Some(stdout)) => {
            let mut hash = Fnv1a64::new();
            hash.update(&code.to_le_bytes());
            hash.update(stdout.as_bytes());
            format!("{:016x}", hash.0)
        }
        _ => TOOLCHAIN_UNKNOWN.to_string(),
    }
}

/// Why a graph command gave no graph, from its outcome: `<label> timed out after
/// <n>s`, `<label> exited <code>` or `<label> did not run`, with its output's last
/// line when it printed one.
fn failure(label: &str, outcome: &ShellOutcome) -> String {
    let what = if outcome.timed_out {
        format!("{label} timed out after {}s", outcome.secs)
    } else if let Some(code) = outcome.code {
        format!("{label} exited {code}")
    } else {
        format!("{label} did not run")
    };
    match outcome.tail.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(line) => format!("{what}: {}", line.trim()),
        None => what,
    }
}

/// The cache key of the graph `source` gives in `dir`, or `None` when it cannot be
/// computed (then the graph is read and nothing is cached).
fn cache_key(
    git: &OsStr,
    dir: &Path,
    source: &GraphSource,
    manifests: &[String],
    dirs: &BTreeMap<String, String>,
    timeout: Duration,
) -> Option<String> {
    let mut hash = Fnv1a64::new();
    let mut field = |bytes: &[u8]| {
        hash.update(bytes);
        hash.update(&[0]);
    };
    match source {
        GraphSource::None => return None,
        GraphSource::Cargo => {
            field(b"cargo");
            let listed = Git::new(git, timeout)
                .ok(dir, &[os("ls-files"), os("-z"), os("--cached")])
                .ok()?;
            let mut paths: Vec<String> = listed
                .split('\0')
                .filter(|p| {
                    let name = p.rsplit('/').next().unwrap_or(p);
                    name == "Cargo.toml" || name == "Cargo.lock"
                })
                .map(str::to_string)
                .collect();
            paths.sort();
            paths.dedup();
            for (path, print) in fingerprint(dir, &paths) {
                field(path.as_bytes());
                field(print.as_bytes());
            }
        }
        GraphSource::Command(command) => {
            field(b"command");
            field(command.as_bytes());
            for (path, print) in fingerprint(dir, manifests) {
                field(path.as_bytes());
                field(print.as_bytes());
            }
            for (name, module_dir) in dirs {
                field(name.as_bytes());
                field(module_dir.as_bytes());
            }
        }
    }
    Some(format!("{:016x}", hash.0))
}

/// Every entry of the cache file, oldest first; none when it is missing or corrupt.
pub(super) fn cache_entries(cache: &Path) -> Vec<CacheEntry> {
    let mut text = String::new();
    let read = std::fs::File::open(cache)
        .and_then(|f| f.take(GRAPH_CACHE_BYTES_MAX).read_to_string(&mut text));
    match read {
        Ok(_) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// The cached graph under `key`.
pub(super) fn cache_get(cache: &Path, key: &str) -> Option<ModuleGraph> {
    cache_entries(cache)
        .into_iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.graph)
}

/// Stores `graph` under `key` as the newest entry, dropping the oldest past
/// [`GRAPH_CACHE_MAX`]. A failed write is logged and ignored: the cache is advisory.
pub(super) fn cache_put(cache: &Path, key: &str, graph: &ModuleGraph) {
    let mut entries = cache_entries(cache);
    entries.retain(|entry| entry.key != key);
    entries.push(CacheEntry {
        key: key.to_string(),
        graph: graph.clone(),
    });
    let excess = entries.len().saturating_sub(GRAPH_CACHE_MAX);
    entries.drain(..excess);
    let written = serde_json::to_vec(&entries)
        .map_err(std::io::Error::other)
        .and_then(|bytes| write_atomic(cache, &bytes));
    if let Err(error) = written {
        tracing::debug!(?error, cache = %cache.display(), "could not write the module graph cache");
    }
}

#[cfg(test)]
#[path = "graph_tests.rs"]
mod tests;
