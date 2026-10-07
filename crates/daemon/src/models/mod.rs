//! Milestone 9.8 (MR §4): each CLI's models, discovered from its own handshake, cached
//! per CLI version, and served to clients by `ListModels`.
//!
//! Probes never run at daemon start (decision 20) and never on a tokio worker: each
//! runtime's refresh is one `spawn_blocking` closure under one [`DISCOVERY_TIMEOUT`],
//! its child owned by `ProbeChild`. The memory map's `crate::lock` is taken only to
//! clone or replace a catalog, never across a probe.

pub mod builtin;
pub mod cache;
pub mod claude_probe;
pub mod codex_probe;
pub mod lines;
pub mod version;

use proto::{CatalogModel, ModelCatalog, ModelRef, Runtime};
use std::collections::BTreeMap;
use std::future::Future;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Decision 22: one runtime's version and model probes together.
pub const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(10);
/// Decision 22: kept from the deadline for `ProbeChild`'s kill-and-reap.
pub const REAP_RESERVE: Duration = Duration::from_millis(100);
/// A reply line longer than this, or a reply past `REPLY_MAX_BYTES`, ends the probe.
pub const LINE_MAX_BYTES: usize = 1 << 20;
pub const REPLY_MAX_BYTES: usize = 4 << 20;
/// After a probe has its answer and closed the CLI's stdin, how long the CLI may take
/// to exit on its own before `ProbeChild` kills it (within the read deadline).
pub const EXIT_GRACE: Duration = Duration::from_millis(500);
/// How much of a CLI's refusal message a problem keeps.
pub(crate) const REFUSAL_MAX_CHARS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    Missing,
    Timeout,
    Cancelled,
    Failed(String),
}

/// What a runtime's probes do; real ones are `CliProbes`, tests use their own.
pub trait Probes: Send + Sync + 'static {
    fn version(&self, runtime: Runtime, deadline: Instant) -> Result<String, ProbeError>;
    /// The models and the raw reply lines read.
    fn models(
        &self,
        runtime: Runtime,
        deadline: Instant,
    ) -> Result<(Vec<CatalogModel>, Vec<String>), ProbeError>;
}

/// The real CLIs. `claude_auth` is `[orchestrator.claude] auth`, so the Claude probe
/// sees the same scrubbed environment a headless Claude session does (ruling F29).
pub struct CliProbes {
    pub claude_bin: String,
    pub codex_bin: String,
    pub claude_auth: config::ClaudeAuth,
    pub cancel: CancellationToken,
}

impl Probes for CliProbes {
    fn version(&self, runtime: Runtime, deadline: Instant) -> Result<String, ProbeError> {
        match runtime {
            Runtime::Codex => match crate::headless::codex_sandbox::recorded_version() {
                Some((a, b, c)) => Ok(format!("{a}.{b}.{c}")),
                None => version::cli_version(&self.codex_bin, deadline, &self.cancel),
            },
            Runtime::Claude => version::cli_version(&self.claude_bin, deadline, &self.cancel),
            Runtime::Shell => Err(ProbeError::Missing),
        }
    }

    fn models(
        &self,
        runtime: Runtime,
        deadline: Instant,
    ) -> Result<(Vec<CatalogModel>, Vec<String>), ProbeError> {
        // Ruling F28: the caller passes the scrub list; the probes apply it.
        match runtime {
            Runtime::Claude => {
                let scrub = crate::headless::credential_scrub_for(runtime, self.claude_auth);
                claude_probe::probe(&self.claude_bin, deadline, &self.cancel, &scrub)
            }
            Runtime::Codex => {
                let scrub =
                    crate::headless::credential_scrub_for(runtime, config::ClaudeAuth::Login);
                codex_probe::probe(&self.codex_bin, deadline, &self.cancel, &scrub)
            }
            Runtime::Shell => Err(ProbeError::Missing),
        }
    }
}

/// The command a model probe runs: `env_remove` removed and `headless::session_vars`
/// set, as a headless session's process gets them; its own process group; stdin and
/// stdout piped, stderr discarded.
pub(crate) fn probe_command(
    program: &str,
    args: &[&str],
    runtime: Runtime,
    env_remove: &[&str],
) -> Command {
    let mut command = Command::new(program);
    command.args(args);
    for name in env_remove {
        command.env_remove(name);
    }
    for (name, value) in crate::headless::session_vars(runtime, &[]) {
        command.env(name, value);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);
    command
}

/// Writes one request line to the CLI.
pub(crate) fn write_line(stdin: &mut ChildStdin, line: &str, cli: &str) -> Result<(), ProbeError> {
    stdin
        .write_all(format!("{line}\n").as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| ProbeError::Failed(format!("could not write to {cli}: {e}")))
}

/// Review focus 3: an id `ModelRef::parse` refuses never enters a catalog.
pub(crate) fn catalog_id_ok(runtime: Runtime, id: &str) -> bool {
    ModelRef::parse(&format!("{}:{id}", runtime.label())).is_ok()
}

fn gate_index(runtime: Runtime) -> Option<usize> {
    match runtime {
        Runtime::Claude => Some(0),
        Runtime::Codex => Some(1),
        Runtime::Shell => None,
    }
}

/// The discovered catalogs (MR §4.2). See the module doc.
pub struct ModelService {
    data_dir: PathBuf,
    probes: Arc<dyn Probes>,
    memory: Mutex<BTreeMap<Runtime, ModelCatalog>>,
    /// Per runtime: held for the whole of one refresh (single flight).
    gates: [tokio::sync::Mutex<()>; 2],
    /// Per runtime: refreshes finished. A caller that sees this move between its call
    /// and its turn at the gate waited for a refresh it did not start, and reads memory.
    finished: [AtomicU64; 2],
}

impl ModelService {
    pub fn new(data_dir: PathBuf, probes: Arc<dyn Probes>) -> Arc<ModelService> {
        Arc::new(ModelService {
            data_dir,
            probes,
            memory: Mutex::new(BTreeMap::new()),
            gates: Default::default(),
            finished: Default::default(),
        })
    }

    /// Loads the newest cache of each runtime into memory as `Cached` (ruling F31), on
    /// `spawn_blocking`; spawns no process.
    pub async fn load_disk(&self) {
        let dir = self.data_dir.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            [Runtime::Claude, Runtime::Codex]
                .into_iter()
                .filter_map(|runtime| cache::newest(&dir, runtime))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();
        let mut memory = crate::lock(&self.memory);
        for mut catalog in loaded {
            catalog.source = proto::CatalogSource::Cached;
            catalog.problem = None;
            memory.insert(catalog.runtime, catalog);
        }
    }

    /// The catalogs in memory now, one per runtime that has one. Never probes.
    pub fn current(&self) -> Vec<ModelCatalog> {
        crate::lock(&self.memory).values().cloned().collect()
    }

    /// MR §4.2 for each runtime asked, the two at once, each single-flight. The calls
    /// are made (and the in-flight refreshes noted) when `list` is called, not when the
    /// future is first polled, so "entered" is well defined (ruling F27).
    pub fn list(
        self: &Arc<Self>,
        runtime: Option<Runtime>,
        refresh: bool,
    ) -> impl Future<Output = Vec<ModelCatalog>> + Send + 'static {
        let runtimes = match runtime {
            Some(runtime) => vec![runtime],
            None => vec![Runtime::Claude, Runtime::Codex],
        };
        let tasks: Vec<_> = runtimes
            .into_iter()
            .filter_map(|runtime| {
                let index = gate_index(runtime)?;
                let ticket = self.finished[index].load(Ordering::SeqCst);
                let service = self.clone();
                Some(tokio::spawn(async move {
                    service.list_one(runtime, index, ticket, refresh).await
                }))
            })
            .collect();
        async move {
            let mut catalogs = Vec::new();
            for task in tasks {
                if let Ok(catalog) = task.await {
                    catalogs.push(catalog);
                }
            }
            catalogs
        }
    }

    async fn list_one(
        self: Arc<Self>,
        runtime: Runtime,
        index: usize,
        ticket: u64,
        refresh: bool,
    ) -> ModelCatalog {
        let _gate = self.gates[index].lock().await;
        if self.finished[index].load(Ordering::SeqCst) != ticket {
            let remembered = crate::lock(&self.memory).get(&runtime).cloned();
            if let Some(catalog) = remembered {
                return catalog;
            }
        }
        let (probes, dir) = (self.probes.clone(), self.data_dir.clone());
        let catalog = tokio::task::spawn_blocking(move || {
            cache::refresh_one(&*probes, &dir, runtime, refresh)
        })
        .await
        .unwrap_or_else(|e| {
            let problem = format!("{} discovery failed: {e}", runtime.label());
            tracing::warn!(%problem, "model discovery failed; using the built-in list");
            builtin::catalog(
                runtime,
                "unknown".into(),
                crate::run::driver::unix_now(),
                problem,
            )
        });
        crate::lock(&self.memory).insert(runtime, catalog.clone());
        self.finished[index].fetch_add(1, Ordering::SeqCst);
        catalog
    }

    /// Decision 20: `list(None, false)` on a spawned task when nothing is in memory.
    pub fn refresh_in_background(self: &Arc<Self>) {
        if crate::lock(&self.memory).is_empty() {
            let list = self.list(None, false);
            tokio::spawn(list);
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_service;
