//! M8b.13: the driver reads a size check's evidence from the report files anthrex
//! stored, and from nowhere else (M8b decision 19).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::DeciderSource;
use serde_json::json;

use super::super::{OpCtx, RunService};
use super::read_evidence;
use crate::decider::{DeciderRequest, Evidence, SizeCheckInput};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::engine::OpResult;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

const RUN: &str = "g-0001";

/// A stored report `id` with `summary`, one file, a module and an interface.
fn write_report(dir: &Path, id: &str, summary: &str) {
    let report = json!({
        "id": id, "kind": "area", "run_id": null, "question": "q", "summary": summary,
        "files": [{"path": format!("src/{id}.rs"), "why": "w"}],
        "modules": [format!("mod-{id}")], "interfaces": [format!("fn {id}()")],
        "route": {"runtime": "claude", "model": "m", "strength": "fast", "effort": "low"},
        "window_id": 1, "started_at": 0, "finished_at": 0, "tool_calls": 0,
        "usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
    });
    let scouts = dir.join("scouts");
    std::fs::create_dir_all(&scouts).unwrap();
    std::fs::write(scouts.join(format!("{id}.json")), report.to_string()).unwrap();
}

/// A service over `data`, holding run `RUN` (its directory `data/runs/RUN`) whose
/// repository directory is `repo_dir` and whose onboarding report is `onboarding`.
fn service_with_run(
    data: &Path,
    repo_dir: &Path,
    onboarding: Option<&str>,
) -> (Arc<RunService>, OpCtx) {
    let config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let service = RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots));
    let mut run = super::tests::run();
    run.id = RUN.into();
    run.data_dir = data.join("runs").join(RUN);
    run.repo_dir = repo_dir.to_path_buf();
    run.onboarding_report = onboarding.map(str::to_string);
    let ctx = OpCtx::of(&run);
    crate::lock(&service.state).runs.insert(RUN.into(), run);
    (service, ctx)
}

fn size_check(refs: &[&str]) -> DeciderRequest {
    DeciderRequest::SizeCheck(SizeCheckInput {
        tasks: Vec::new(),
        evidence_refs: refs.iter().map(|r| r.to_string()).collect(),
        evidence: Vec::new(),
        modules: Vec::new(),
        hub: Vec::new(),
    })
}

#[tokio::test]
async fn the_driver_resolves_evidence_from_report_files() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let repo_dir = data.join("repos").join("r-00000001");
    write_report(&data.join("runs").join(RUN), "api-1", "the api");
    write_report(&repo_dir, "onboarding-9", "a Rust workspace");
    let (service, ctx) = service_with_run(&data, &repo_dir, Some("onboarding-9"));
    let mut request = size_check(&["api-1", "onboarding"]);
    service.with_evidence(&ctx, &mut request).await.unwrap();
    let DeciderRequest::SizeCheck(input) = &request else {
        unreachable!()
    };
    let evidence = |id: &str, summary: &str| Evidence {
        id: id.into(),
        summary: summary.into(),
        files: vec![format!("src/{id}.rs")],
        modules: vec![format!("mod-{id}")],
        interfaces: vec![format!("fn {id}()")],
    };
    assert_eq!(
        input.evidence,
        vec![
            evidence("api-1", "the api"),
            evidence("onboarding-9", "a Rust workspace")
        ]
    );
    // Another kind of request is left as it is.
    let mut other = DeciderRequest::BlockedReason(crate::decider::BlockedReasonInput {
        task_id: "t1".into(),
        title: "T".into(),
        reason: "r".into(),
    });
    let before = other.clone();
    service.with_evidence(&ctx, &mut other).await.unwrap();
    assert_eq!(other, before);
}

/// A plan's `scout_refs` never become a path outside the stored reports: a ref that is
/// not a scout id, the alias with no onboarding report, and a report reached through a
/// link are all skipped; with nothing read, the size check is its fallback.
#[tokio::test]
async fn evidence_is_read_only_from_stored_reports() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let run_dir = data.join("runs").join(RUN);
    let repo_dir = data.join("repos").join("r-00000001");
    // A report outside the scouts directory, one a traversal would reach, and a link
    // to it under the scouts directory.
    write_report(&data, "secret", "not evidence");
    std::fs::create_dir_all(run_dir.join("scouts")).unwrap();
    std::os::unix::fs::symlink(
        data.join("scouts").join("secret.json"),
        run_dir.join("scouts").join("linked.json"),
    )
    .unwrap();
    // The alias's file exists under the run, but the repository has no onboarding report.
    write_report(&run_dir, "onboarding", "a run scout named like the alias");
    let refs: Vec<String> = ["../../../scouts/secret", "linked", "onboarding"]
        .iter()
        .map(|r| r.to_string())
        .collect();
    let error = read_evidence(&refs, &run_dir, &repo_dir, None).unwrap_err();
    assert!(
        error.starts_with("no scout report could be read ("),
        "{error}"
    );
    assert!(
        error.contains("../../../scouts/secret: not a stored report"),
        "{error}"
    );
    assert!(error.contains("linked: not a regular file"), "{error}");
    assert!(error.contains("onboarding: not a stored report"), "{error}");
    // An onboarding id that is not a scout id is not followed either.
    let error = read_evidence(&refs[2..], &run_dir, &repo_dir, Some("../x")).unwrap_err();
    assert!(error.contains("onboarding: not a stored report"), "{error}");

    let (service, ctx) = service_with_run(&data, &repo_dir, None);
    let request = size_check(&["../../../scouts/secret", "linked"]);
    let OpResult::Decided(decision) = service.decide(&ctx, request).await else {
        panic!("not decided")
    };
    assert_eq!(decision.source, DeciderSource::Fallback);
    let reason = decision.fallback_reason.unwrap_or_default();
    assert!(
        reason.starts_with("the decider could not start: no scout report could be read ("),
        "{reason}"
    );
}
