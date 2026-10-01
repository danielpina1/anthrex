//! Milestone 9.0.6 decision 41 (preflight F12): `run accept` and `run discard` ask the
//! engine's own predicate under the engine lock before any git read or confirmation.

use std::sync::Arc;

use proto::run_wire::request;
use proto::{ActionKind, FinishAction, RunReply, RunState};

use super::RunService;
use super::context::RunContext;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::engine::actions::{self, ActionNode};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

/// A service whose git is a binary that does not exist, holding one running run.
fn with_running_run(data: &std::path::Path) -> (Arc<RunService>, String) {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let mut ctx = RunContext::new(
        data.to_path_buf(),
        manager.config(),
        config::Orchestrator::default(),
        Arc::new(NoRoots),
    );
    ctx.git = data.join("no-such-git").into_os_string();
    let s = RunService::new(manager, ctx);
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.state = RunState::Running;
    run.data_dir = data.join("runs").join(&run.id);
    let id = run.id.clone();
    crate::lock(&s.state).runs.insert(id.clone(), run);
    (s, id)
}

/// `check`'s refusal of `kind` on the run.
fn checked(s: &RunService, id: &str, kind: ActionKind) -> String {
    let state = crate::lock(&s.state);
    actions::check(&state.runs[id], &ActionNode::Run, &kind).unwrap_err()
}

#[tokio::test]
async fn accept_of_a_running_run_is_refused_before_any_git_read() {
    let data = tempfile::tempdir().unwrap();
    let (s, id) = with_running_run(data.path());
    let expected = checked(&s, &id, ActionKind::Accept);
    assert_eq!(
        expected,
        format!("run {id} is running; accept applies only to a complete run")
    );
    let reply = s.finish(id.clone(), FinishAction::Accept, None).await;
    assert_eq!(reply, RunReply::refused(request::FINISH, expected));
}

#[tokio::test]
async fn discard_of_a_running_run_is_refused_before_its_confirmation() {
    let data = tempfile::tempdir().unwrap();
    let (s, id) = with_running_run(data.path());
    let expected = checked(&s, &id, ActionKind::Discard);
    let reply = s.finish(id.clone(), FinishAction::Discard, None).await;
    assert_eq!(reply, RunReply::refused(request::FINISH, expected));
}
