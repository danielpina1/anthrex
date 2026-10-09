//! One runtime's refresh (MR §4.2) and the per-version cache files it reads and writes:
//! `<data_dir>/models/<runtime>-<version>.json`, with the raw reply beside it as
//! `<runtime>-<version>.raw.jsonl` (decision 26). Runs on a blocking thread only.

use super::{DISCOVERY_TIMEOUT, ProbeError, Probes, builtin};
use proto::{CatalogSource, ModelCatalog, Runtime};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Decision 26: at most this much of a raw reply is kept.
const RAW_MAX_BYTES: usize = 256 * 1024;

/// The catalog for `runtime`: the cache for the CLI's version (unless `refresh`), else
/// a live probe, else the newest cache, else the built-in list.
pub fn refresh_one(
    probes: &dyn Probes,
    dir: &Path,
    runtime: Runtime,
    refresh: bool,
) -> ModelCatalog {
    let deadline = Instant::now() + DISCOVERY_TIMEOUT;
    let now = crate::run::driver::unix_now();
    let version = match probes.version(runtime, deadline) {
        Ok(version) => version,
        Err(error) => return fallback(dir, runtime, "unknown".into(), now, &error),
    };
    if !refresh
        && version != "unknown"
        && let Some(mut cached) = read(&file(dir, runtime, &version))
    {
        cached.source = CatalogSource::Live;
        cached.problem = None;
        return cached;
    }
    match probes.models(runtime, deadline) {
        Ok((models, raw)) => {
            let catalog = ModelCatalog {
                runtime,
                cli_version: version,
                fetched_at: now,
                source: CatalogSource::Live,
                models,
                problem: None,
            };
            if let Err(e) = write(dir, &catalog, &raw) {
                tracing::warn!(error = %e, ?runtime, "could not cache the model list");
            }
            catalog
        }
        Err(error) => fallback(dir, runtime, version, now, &error),
    }
}

/// The problem a failed probe reports (decision 24).
pub fn problem(runtime: Runtime, error: &ProbeError) -> String {
    let name = runtime.label();
    match error {
        ProbeError::Missing => format!("{name} not found"),
        ProbeError::Timeout => format!(
            "{name} did not answer within {} s",
            DISCOVERY_TIMEOUT.as_secs()
        ),
        ProbeError::Cancelled => format!("{name}'s probe was cancelled"),
        ProbeError::Failed(message) => message.clone(),
    }
}

fn fallback(
    dir: &Path,
    runtime: Runtime,
    version: String,
    now: u64,
    error: &ProbeError,
) -> ModelCatalog {
    let problem = problem(runtime, error);
    if *error == ProbeError::Missing {
        return builtin::catalog(runtime, version, now, problem);
    }
    if let Some(mut cached) = newest(dir, runtime) {
        cached.source = CatalogSource::Cached;
        cached.problem = Some(problem);
        return cached;
    }
    tracing::warn!(?runtime, %problem, "model discovery failed; using the built-in list");
    builtin::catalog(runtime, version, now, problem)
}

fn models_dir(dir: &Path) -> PathBuf {
    dir.join("models")
}

fn file(dir: &Path, runtime: Runtime, version: &str) -> PathBuf {
    models_dir(dir).join(format!("{}-{version}.json", runtime.label()))
}

fn read(path: &Path) -> Option<ModelCatalog> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// The newest `<runtime>-*.json` by `fetched_at`, as stored (ruling F31).
pub fn newest(dir: &Path, runtime: Runtime) -> Option<ModelCatalog> {
    let prefix = format!("{}-", runtime.label());
    std::fs::read_dir(models_dir(dir))
        .ok()?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&prefix) && name.ends_with(".json")
        })
        .filter_map(|entry| read(&entry.path()))
        .filter(|catalog| catalog.runtime == runtime)
        .max_by_key(|catalog| catalog.fetched_at)
}

fn write(dir: &Path, catalog: &ModelCatalog, raw: &[String]) -> std::io::Result<()> {
    std::fs::create_dir_all(models_dir(dir))?;
    let path = file(dir, catalog.runtime, &catalog.cli_version);
    let json = serde_json::to_string_pretty(catalog).map_err(std::io::Error::other)?;
    atomic_write(&path, json.as_bytes())?;
    let mut kept = String::new();
    for line in raw {
        if kept.len() + line.len() + 1 > RAW_MAX_BYTES {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    atomic_write(&path.with_extension("raw.jsonl"), kept.as_bytes())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let staging = path.with_extension(format!("tmp.{}", std::process::id()));
    std::fs::write(&staging, bytes)?;
    std::fs::rename(&staging, path)
}
