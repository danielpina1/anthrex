//! `ModelService` over scripted probes (M9.8.6, MR §4.2).

use super::*;
use proto::{CatalogModel, CatalogSource, ModelCatalog, Runtime};
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Scripted results per runtime, with call counters and each `models` call's span.
struct CountingProbes {
    versions: Mutex<BTreeMap<Runtime, Result<String, ProbeError>>>,
    models: Mutex<BTreeMap<Runtime, Result<Vec<CatalogModel>, ProbeError>>>,
    version_calls: Mutex<BTreeMap<Runtime, u32>>,
    model_calls: Mutex<BTreeMap<Runtime, u32>>,
    spans: Mutex<Vec<(Runtime, Instant, Instant)>>,
    /// While `true`, `models` blocks (F27: until both `list` calls have entered).
    held: Mutex<bool>,
    released: Condvar,
}

fn model(id: &str) -> CatalogModel {
    CatalogModel {
        id: id.to_string(),
        label: id.to_string(),
        description: String::new(),
        efforts: vec!["low".into(), "high".into()],
        default_effort: Some("low".into()),
        is_default: false,
    }
}

impl CountingProbes {
    fn new() -> Arc<CountingProbes> {
        let ok = |v: &str| Ok(v.to_string());
        Arc::new(CountingProbes {
            versions: Mutex::new(BTreeMap::from([
                (Runtime::Claude, ok("2.1.290")),
                (Runtime::Codex, ok("0.160.1")),
            ])),
            models: Mutex::new(BTreeMap::from([
                (Runtime::Claude, Ok(vec![model("claude-opus-5-5")])),
                (Runtime::Codex, Ok(vec![model("gpt-6-sol")])),
            ])),
            version_calls: Mutex::default(),
            model_calls: Mutex::default(),
            spans: Mutex::default(),
            held: Mutex::new(false),
            released: Condvar::new(),
        })
    }

    fn set_version(&self, runtime: Runtime, version: Result<String, ProbeError>) {
        self.versions.lock().unwrap().insert(runtime, version);
    }

    fn set_models(&self, runtime: Runtime, models: Result<Vec<CatalogModel>, ProbeError>) {
        self.models.lock().unwrap().insert(runtime, models);
    }

    fn model_calls(&self, runtime: Runtime) -> u32 {
        self.model_calls
            .lock()
            .unwrap()
            .get(&runtime)
            .copied()
            .unwrap_or(0)
    }

    fn any_calls(&self) -> bool {
        !self.version_calls.lock().unwrap().is_empty()
            || !self.model_calls.lock().unwrap().is_empty()
    }

    fn hold(&self) {
        *self.held.lock().unwrap() = true;
    }

    fn release(&self) {
        *self.held.lock().unwrap() = false;
        self.released.notify_all();
    }
}

impl Probes for CountingProbes {
    fn version(&self, runtime: Runtime, _deadline: Instant) -> Result<String, ProbeError> {
        *self
            .version_calls
            .lock()
            .unwrap()
            .entry(runtime)
            .or_default() += 1;
        self.versions.lock().unwrap()[&runtime].clone()
    }

    fn models(
        &self,
        runtime: Runtime,
        _deadline: Instant,
    ) -> Result<(Vec<CatalogModel>, Vec<String>), ProbeError> {
        let start = Instant::now();
        *self.model_calls.lock().unwrap().entry(runtime).or_default() += 1;
        let mut held = self.held.lock().unwrap();
        while *held {
            held = self.released.wait(held).unwrap();
        }
        drop(held);
        let result = self.models.lock().unwrap()[&runtime].clone();
        self.spans
            .lock()
            .unwrap()
            .push((runtime, start, Instant::now()));
        result.map(|models| (models, vec!["{\"raw\":true}".to_string()]))
    }
}

fn service(dir: &std::path::Path, probes: &Arc<CountingProbes>) -> Arc<ModelService> {
    ModelService::new(dir.to_path_buf(), probes.clone())
}

fn write_cache(dir: &std::path::Path, version: &str, fetched_at: u64, id: &str) {
    let catalog = ModelCatalog {
        runtime: Runtime::Codex,
        cli_version: version.to_string(),
        fetched_at,
        source: CatalogSource::Live,
        models: vec![model(id)],
        problem: None,
    };
    let models = dir.join("models");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(
        models.join(format!("codex-{version}.json")),
        serde_json::to_string(&catalog).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn a_cache_for_the_same_version_is_used_without_probing() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    let first = service(dir.path(), &probes)
        .list(Some(Runtime::Codex), false)
        .await;
    assert_eq!(probes.model_calls(Runtime::Codex), 1);
    assert!(dir.path().join("models/codex-0.160.1.json").is_file());
    assert!(dir.path().join("models/codex-0.160.1.raw.jsonl").is_file());
    // A new service: only the file can answer.
    let again = service(dir.path(), &probes)
        .list(Some(Runtime::Codex), false)
        .await;
    assert_eq!(probes.model_calls(Runtime::Codex), 1);
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].fetched_at, first[0].fetched_at);
    assert_eq!(again[0].source, CatalogSource::Live);
    assert_eq!(again, first);
}

/// Fix round 1: a cache file that does not parse is no cache; the probe runs and its
/// catalog replaces the file.
#[tokio::test]
async fn a_garbage_cache_file_is_treated_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    let models = dir.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    let path = models.join("codex-0.160.1.json");
    std::fs::write(&path, "not json {").unwrap();
    let probes = CountingProbes::new();
    let catalogs = service(dir.path(), &probes)
        .list(Some(Runtime::Codex), false)
        .await;
    assert_eq!(probes.model_calls(Runtime::Codex), 1);
    assert_eq!(catalogs[0].source, CatalogSource::Live);
    assert_eq!(catalogs[0].models, vec![model("gpt-6-sol")]);
    let written: ModelCatalog =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written, catalogs[0]);
}

#[tokio::test]
async fn a_new_version_probes_again() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    let svc = service(dir.path(), &probes);
    svc.list(Some(Runtime::Codex), false).await;
    probes.set_version(Runtime::Codex, Ok("0.161.0".into()));
    let catalogs = svc.list(Some(Runtime::Codex), false).await;
    assert_eq!(probes.model_calls(Runtime::Codex), 2);
    assert_eq!(catalogs[0].cli_version, "0.161.0");
    assert!(dir.path().join("models/codex-0.161.0.json").is_file());
}

#[tokio::test]
async fn refresh_probes_even_with_a_matching_cache() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    let svc = service(dir.path(), &probes);
    svc.list(Some(Runtime::Codex), false).await;
    let catalogs = svc.list(Some(Runtime::Codex), true).await;
    assert_eq!(probes.model_calls(Runtime::Codex), 2);
    assert_eq!(catalogs[0].source, CatalogSource::Live);
}

#[tokio::test]
async fn a_failed_probe_serves_the_newest_cache_as_cached() {
    let dir = tempfile::tempdir().unwrap();
    write_cache(dir.path(), "0.150.0", 10, "gpt-old");
    write_cache(dir.path(), "0.151.0", 20, "gpt-newer");
    let probes = CountingProbes::new();
    probes.set_models(Runtime::Codex, Err(ProbeError::Timeout));
    let catalogs = service(dir.path(), &probes)
        .list(Some(Runtime::Codex), false)
        .await;
    assert_eq!(catalogs.len(), 1);
    let codex = &catalogs[0];
    assert_eq!(codex.models, vec![model("gpt-newer")]);
    assert_eq!(codex.fetched_at, 20);
    assert_eq!(codex.source, CatalogSource::Cached);
    assert_eq!(
        codex.problem.as_deref(),
        Some("codex did not answer within 10 s")
    );
}

#[tokio::test]
async fn no_cache_and_a_failed_probe_serve_the_builtin_list() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    probes.set_models(
        Runtime::Codex,
        Err(ProbeError::Failed("codex's reply had no models".into())),
    );
    let catalogs = service(dir.path(), &probes)
        .list(Some(Runtime::Codex), false)
        .await;
    let codex = &catalogs[0];
    assert_eq!(codex.source, CatalogSource::Builtin);
    assert_eq!(codex.models[0].id, "default");
    assert_eq!(
        codex.problem.as_deref(),
        Some("codex's reply had no models")
    );
}

#[tokio::test]
async fn a_missing_cli_is_builtin_and_says_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    probes.set_version(Runtime::Claude, Err(ProbeError::Missing));
    let catalogs = service(dir.path(), &probes)
        .list(Some(Runtime::Claude), false)
        .await;
    let claude = &catalogs[0];
    assert_eq!(claude.source, CatalogSource::Builtin);
    assert_eq!(claude.problem.as_deref(), Some("claude not found"));
    assert_eq!(
        claude
            .models
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>(),
        ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5-5"]
    );
    assert_eq!(probes.model_calls(Runtime::Claude), 0);
}

/// Review focus 5, ruling F27: a caller that finds a refresh in flight waits for it and
/// reads memory; it never probes the same runtime a second time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_concurrent_lists_probe_once() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    probes.hold();
    let svc = service(dir.path(), &probes);
    let first = svc.list(None, true);
    let second = svc.list(None, true);
    // Both calls have entered; let the one probe per runtime start, then finish.
    let deadline = Instant::now() + Duration::from_secs(10);
    while probes.model_calls(Runtime::Claude) + probes.model_calls(Runtime::Codex) < 2 {
        assert!(Instant::now() < deadline, "the probes never started");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    probes.release();
    let (a, b) = tokio::join!(first, second);
    assert_eq!(a.len(), 2);
    assert_eq!(a, b, "the waiter reads what the first probe found");
    assert_eq!(probes.model_calls(Runtime::Claude), 1);
    assert_eq!(probes.model_calls(Runtime::Codex), 1);
    let spans = probes.spans.lock().unwrap().clone();
    for (i, (runtime, start, end)) in spans.iter().enumerate() {
        for (other, s2, e2) in &spans[i + 1..] {
            assert!(
                runtime != other || end <= s2 || e2 <= start,
                "two {runtime:?} probes overlapped"
            );
        }
    }
}

#[tokio::test]
async fn current_never_probes() {
    let dir = tempfile::tempdir().unwrap();
    let probes = CountingProbes::new();
    let svc = service(dir.path(), &probes);
    assert!(svc.current().is_empty());
    assert!(!probes.any_calls());
    write_cache(dir.path(), "0.150.0", 10, "gpt-old");
    write_cache(dir.path(), "0.151.0", 20, "gpt-newer");
    svc.load_disk().await;
    let current = svc.current();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0].models, vec![model("gpt-newer")]);
    assert_eq!(current[0].source, CatalogSource::Cached);
    assert_eq!(current[0].problem, None);
    assert!(!probes.any_calls());
}
