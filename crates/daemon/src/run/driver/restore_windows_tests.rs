//! The Ubuntu CI hang of `e2e_crash_after_each_intent_kind_reconciles` (PR #75): a
//! restart never hands out a window id a restored run still names. The window table's
//! state file is saved with a debounce (`state::SAVE_DEBOUNCE`, at most
//! `SAVE_MAX_DELAY`), so a daemon that dies within it restarts with no record of its
//! newest windows and would number the next one from where its file left off, while
//! the restored runs' rounds still name those ids. The engine routes a window's events
//! to the latest round naming it, so a fresh session's early turn end went to the
//! restored, ended round and was dropped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::{Runtime, WindowSpec};
use serde_json::json;

use super::named_windows;
use crate::launch::LaunchGate;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::RunService;
use crate::run::engine::OpResult;
use crate::run::model::{Outgoing, Run};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

/// A running run `r1` under `data` whose worker round names window `window`.
fn run_with_round(data: &Path, window: u32) -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = "r1".into();
    run.data_dir = data.join("runs").join("r1");
    run.state = proto::RunState::Running;
    let mut round = crate::run::orch::test_support::round(1, 0, Default::default());
    round.window_id = Some(window);
    round.pid = None;
    run.tasks[0].rounds.push(round);
    run
}

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_never_hands_out_a_window_id_a_restored_run_names() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    // The crashed daemon's state file never saved its window: the table starts empty.
    let (manager, _events) = WindowManager::new(config);
    let data = dir.path().join("data");
    let run = run_with_round(&data, 1);
    crate::run::journal::save_run(&run).unwrap();
    let s = RunService::for_manager(&manager, data, Arc::new(NoRoots));
    s.restore().await;
    let named = crate::lock(&s.state).runs["r1"].tasks[0].rounds[0].window_id;
    assert_eq!(named, Some(1), "the restored round names window 1");

    let spec = WindowSpec {
        name: Some("after-restart".into()),
        runtime: Runtime::Shell,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, dir.path().to_path_buf(), None, 80, 24)
        .await
        .expect("a shell window");
    // The window this test started, and nothing else.
    let _ = manager.kill(info.id);
    assert_eq!(info.id, 2, "a new window took an id the restored run names");
}

#[test]
fn every_window_id_a_run_or_a_replayed_result_names_counts() {
    let data = tempfile::tempdir().unwrap();
    let mut run = run_with_round(data.path(), 3);
    assert_eq!(named_windows(std::slice::from_ref(&run), &[]), 3);
    run.outbox.push(Outgoing {
        id: 1,
        window_id: 6,
        task_id: "t1".into(),
        text: "hi".into(),
        queued_at: 0,
        delivered_at: None,
    });
    assert_eq!(named_windows(std::slice::from_ref(&run), &[]), 6);
    // A launch whose `done` line the journal replays names its window too.
    let replay = vec![(
        "r1".to_string(),
        9,
        OpResult::Window {
            window_id: 8,
            pid: None,
        },
    )];
    assert_eq!(named_windows(&[run], &replay), 8);
}

#[test]
fn a_window_id_is_found_at_any_depth_under_either_name() {
    let value = json!({
        "orch": {"orchestrator": {"window_id": 4}, "handoffs": [{"from_window": 11}]},
        "tasks": [{"rounds": [{"window_id": null}, {"window_id": 2}]}],
        "max_windows": 99,
        "windows_created": 50,
    });
    assert_eq!(super::highest_window(&value), 11);
    assert_eq!(
        super::highest_window(&json!({"window_id": 1u64 << 40})),
        u32::MAX
    );
}
