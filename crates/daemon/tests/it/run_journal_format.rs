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
use daemon::run::engine::OpKind;
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
    assert_eq!(RUN_FORMAT, 4);
    let json = saved_json(data.path());
    assert_eq!(json["format"], serde_json::json!(4), "{json}");
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

/// Pinning (milestone 9.14, pre-flight ruling C3's bump to 3): a `run.json` a 9.13
/// daemon wrote, stamped format 2, still loads.
#[test]
fn a_format_2_run_loads() {
    let data = tempfile::tempdir().unwrap();
    let mut json = saved_json(data.path());
    json.as_object_mut()
        .unwrap()
        .insert("format".into(), serde_json::json!(2));
    std::fs::write(run_file(data.path()), serde_json::to_vec(&json).unwrap()).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0.id, RUN_ID);
}

/// Pinning (milestone 9.15, pre-flight ruling C1's bump to 4): the `run.json` a 9.14
/// daemon wrote (task M9.15.1's capture, stamped format 3) still loads.
#[test]
fn a_format_3_run_loads() {
    let text = include_str!("../fixtures/run/m914-run.json");
    let json: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(json["format"], serde_json::json!(3));
    let id = json["id"].as_str().unwrap();
    let data = tempfile::tempdir().unwrap();
    let dir = data.path().join("runs").join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(RUN_FILE), text).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].0.id, id);
}

/// Milestone 9.15 (ruling C1): a task record's signatures and `owns` reach `run.json`
/// inside a pending `AppendHistory`, which a 9.14 daemon would drop: the run is saved as
/// format 4, and both survive a load.
#[test]
fn a_pending_task_record_keeps_its_signatures() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let task_line = include_str!("../../../proto/src/m914_history.jsonl")
        .lines()
        .find(|l| l.contains("\"type\":\"task\""))
        .expect("the fixture's task line");
    let proto::HistoryLine::Task(mut record) = serde_json::from_str(task_line).unwrap() else {
        panic!("{task_line}");
    };
    record.signatures = vec![proto::SigRef {
        gate: proto::SigGate::Tier2,
        fp: "0123456789abcdef".into(),
        headline: "error: it failed".into(),
    }];
    record.owns = vec!["crates/a/**".into()];
    let line = proto::HistoryLine::Task(record.clone());
    let kind = OpKind::AppendHistory {
        path: data.path().join("history.jsonl"),
        record_id: record.record_id.clone(),
        line: Box::new(line.clone()),
    };
    fixture::pend(&mut run, 4, Some("t1"), kind);
    save_run(&run).unwrap();
    let json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run_file(data.path())).unwrap()).unwrap();
    assert_eq!(json["format"], serde_json::json!(4));
    let saved = json["pending_ops"]
        .as_object()
        .unwrap()
        .values()
        .find_map(|op| op["kind"]["AppendHistory"]["line"].as_object())
        .expect("the pending AppendHistory");
    assert_eq!(saved["signatures"][0]["gate"], "tier2", "{saved:?}");
    assert_eq!(saved["owns"], serde_json::json!(["crates/a/**"]));
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    let pending: Vec<_> = runs[0].0.pending_ops.values().collect();
    let OpKind::AppendHistory { line: loaded, .. } = &pending[0].kind else {
        panic!("{pending:?}");
    };
    assert_eq!(**loaded, line);
}

/// Milestone 9.14 (M9.14.8): a 9.13 `run.json`, stamped format 2, whose pending
/// `CreateWindow` spec has no `admit` loads with `admit: None`.
#[test]
fn a_format_2_pending_create_window_loads_without_admit() {
    let data = tempfile::tempdir().unwrap();
    let mut run = plain_run(data.path());
    let mut kind = fixture::create_window(
        data.path(),
        fixture::run_ref("t1", proto::AgentRole::Worker, 1),
        None,
    );
    if let OpKind::CreateWindow { spec, .. } = &mut kind {
        spec.admit = Some(daemon::admit::AdmitHook {
            wait_secs: 300,
            slots: Some(2),
            prefixes: Vec::new(),
        });
    }
    fixture::pend(&mut run, 4, Some("t1"), kind);
    save_run(&run).unwrap();
    let file = run_file(data.path());
    let mut json: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    json["format"] = serde_json::json!(2);
    let spec = json["pending_ops"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .find_map(|op| op["kind"]["CreateWindow"]["spec"].as_object_mut())
        .expect("the pending CreateWindow");
    assert!(spec.remove("admit").is_some(), "the spec saves `admit`");
    std::fs::write(&file, serde_json::to_vec(&json).unwrap()).unwrap();
    let (runs, problems) = load_all(data.path());
    assert!(problems.is_empty(), "{problems:?}");
    let pending: Vec<_> = runs[0].0.pending_ops.values().collect();
    assert_eq!(pending.len(), 1);
    let OpKind::CreateWindow { spec, .. } = &pending[0].kind else {
        panic!("{pending:?}");
    };
    assert_eq!(spec.admit, None);
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
    object.insert("format".into(), serde_json::json!(5));
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
            "run {RUN_ID} skipped: written by a newer anthrex (run format 5; this one reads up to 4); left as it is"
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
                "run {RUN_ID} skipped: written by a newer anthrex (run format {format}; this one reads up to 4); left as it is"
            )],
            "{format}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "{format}");
    }
}
