//! Milestone 9.11 decision 36 (A13): `run.json` records its format. A run written by a
//! newer anthrex (a format above `RUN_FORMAT`) is skipped at load with a problem and its
//! directory is never written, so a downgrade cannot rewrite it without the fields it
//! does not know; a run from before the field (protocol 21) still loads.

// Each binary mounted its own copy before the merge (milestone 9.12, decision 13).
#[allow(clippy::duplicate_mod)]
#[path = "run_journal/fixture.rs"]
#[allow(dead_code)]
mod fixture;

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::{RunContext, RunService};
use daemon::run::journal::{RUN_FILE, RUN_FORMAT, RUN_TMP, load_all, save_run};
use daemon::server::GitWiring;
use fixture::{RUN_ID, plain_run};
use std::path::Path;

/// `run.json` of a run saved by this build, as JSON.
fn saved_json(data: &Path) -> serde_json::Value {
    let run = plain_run(data);
    save_run(&run).unwrap();
    let bytes = std::fs::read(run.data_dir.join(RUN_FILE)).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn run_file(data: &Path) -> std::path::PathBuf {
    data.join("runs").join(RUN_ID).join(RUN_FILE)
}

#[test]
fn a_saved_run_records_its_format() {
    let data = tempfile::tempdir().unwrap();
    assert_eq!(RUN_FORMAT, 2);
    let json = saved_json(data.path());
    assert_eq!(json["format"], serde_json::json!(2), "{json}");
}

/// Pinning (milestone 9.13, decision 36's bump to 2): a `run.json` a 9.11 or 9.12
/// daemon wrote, stamped format 1, still loads.
#[test]
fn a_format_1_run_loads() {
    let data = tempfile::tempdir().unwrap();
    let mut json = saved_json(data.path());
    json.as_object_mut()
        .unwrap()
        .insert("format".into(), serde_json::json!(1));
    std::fs::write(run_file(data.path()), serde_json::to_vec(&json).unwrap()).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0.id, RUN_ID);
}

/// Pinning: a protocol-21 `run.json` has no `format` and loads as format 0.
#[test]
fn a_run_without_a_format_loads() {
    let data = tempfile::tempdir().unwrap();
    let mut json = saved_json(data.path());
    json.as_object_mut().unwrap().remove("format").unwrap();
    std::fs::write(run_file(data.path()), serde_json::to_vec(&json).unwrap()).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0.id, RUN_ID);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_from_a_newer_anthrex_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let mut json = saved_json(&data);
    let object = json.as_object_mut().unwrap();
    object.insert("format".into(), serde_json::json!(3));
    object.insert(
        "a_field_from_the_future".into(),
        serde_json::json!({"kept": true}),
    );
    let file = run_file(&data);
    let bytes = serde_json::to_vec_pretty(&json).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    // A leftover temp file a load would otherwise remove: the directory is not written.
    let leftover = file.with_file_name(RUN_TMP);
    std::fs::write(&leftover, b"half a run").unwrap();

    let (runs, problems) = load_all(&data);
    assert!(runs.is_empty(), "the newer run was loaded");
    assert_eq!(
        problems,
        [format!(
            "run {RUN_ID} skipped: written by a newer anthrex (run format 3; this one reads up to 2); left as it is"
        )]
    );
    assert!(
        leftover.exists(),
        "load_all wrote to the newer run's directory"
    );
    assert_eq!(std::fs::read(&file).unwrap(), bytes);

    // A whole restore, as the daemon runs it at start.
    let (manager, _events) = WindowManager::new(ManagerConfig::for_tests(
        dir.path().join("d.sock"),
        "/bin/sh".into(),
    ));
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let service = RunService::new(manager, ctx);
    service.restore().await;
    assert_eq!(
        std::fs::read(&file).unwrap(),
        bytes,
        "the restore rewrote a newer run.json"
    );
}

/// Stream X review X-M2: a `format` that is present but not a whole number this build
/// can read (a string, a fraction, a negative, one past `u32`) is a format this build
/// does not know: the run is skipped and left as it is, never read as 0 and rewritten.
#[test]
fn a_run_whose_format_is_not_a_whole_number_is_left_alone() {
    for format in [
        serde_json::json!("2"),
        serde_json::json!(1.5),
        serde_json::json!(-1),
        serde_json::json!(4_294_967_296u64),
        serde_json::json!(null),
    ] {
        let data = tempfile::tempdir().unwrap();
        let mut json = saved_json(data.path());
        json.as_object_mut()
            .unwrap()
            .insert("format".into(), format.clone());
        let file = run_file(data.path());
        let bytes = serde_json::to_vec(&json).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        let (runs, problems) = load_all(data.path());
        assert!(runs.is_empty(), "{format}: the run was loaded");
        assert_eq!(
            problems,
            [format!(
                "run {RUN_ID} skipped: written by a newer anthrex (run format {format}; this one reads up to 2); left as it is"
            )],
            "{format}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "{format}");
    }
}
