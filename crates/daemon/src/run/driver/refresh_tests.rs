//! Milestone 9 task M9.13a, decision 42e: a refresh's clean-tree check at acceptance
//! (`driver/refresh.rs`), for `run edit` and the orchestrator's `edit_plan`, through a
//! real daemon socket (`orch_read_rig.rs`) and a real task worktree. The refused call
//! never reaches the engine.

use std::path::Path;
use std::process::Command;

use proto::{PlanEdit, RunReply, RunRequest};
use serde_json::json;

use super::read_rig::Rig;
use crate::run::model::Run;
use crate::run::orch::RefreshState;
use crate::run::orch::test_support::task_mut;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// `t0`'s real task worktree, one worker commit past its start, in a repository at
/// `<dir>/repo`.
fn with_worktree(run: &mut Run, dir: &Path) {
    let root = dir.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(&root, &["add", "a.txt"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let start = git(&root, &["rev-parse", "HEAD"]);
    let path = dir.join(format!("wt/runs/{}/t0", run.id));
    let branch = format!("anthrex/{}/t0", run.id);
    let git_bin = std::ffi::OsStr::new("git");
    let timeout = std::time::Duration::from_secs(30);
    crate::run::git::prepare_worktree(git_bin, &root, &branch, &start, &path, timeout).unwrap();
    std::fs::write(path.join("b.txt"), "task\n").unwrap();
    git(&path, &["add", "b.txt"]);
    git(&path, &["commit", "-q", "-m", "task work"]);
    run.root = root.clone();
    run.project = root;
    let t0 = task_mut(run, "t0");
    t0.worktree = path;
    t0.branch = branch;
    t0.start_commit = Some(start);
}

fn refresh_t0(rig: &Rig) -> RunRequest {
    RunRequest::Edit {
        run_id: rig.run_id.clone(),
        edits: vec![PlanEdit::Refresh {
            task_id: "t0".into(),
        }],
        submit: false,
    }
}

const UNCOMMITTED: &str =
    "task t0 has uncommitted changes; send it a message asking it to commit first";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_with_uncommitted_changes_is_refused() {
    let rig = Rig::new(with_worktree).await;
    let worktree = rig.run(|run| run.task("t0").unwrap().worktree.clone());
    std::fs::write(worktree.join("b.txt"), "edited, not committed\n").unwrap();
    let edits_before = rig.run(|run| run.plan_edits.len());

    let reply = rig.request(refresh_t0(&rig)).await;
    let RunReply::Refused { message, .. } = reply else {
        panic!("refused: {reply:?}");
    };
    assert_eq!(message, UNCOMMITTED);
    // The orchestrator's `edit_plan` is refused the same way.
    let call = json!({"edits": [{"op": "refresh", "task_id": "t0"}]});
    let (ok, value) = rig.orch("edit_plan", call).await;
    assert!(!ok, "{value}");
    assert_eq!(value["error"], UNCOMMITTED);
    rig.run(|run| {
        assert_eq!(
            run.plan_edits.len(),
            edits_before,
            "the engine never saw it"
        );
        assert_eq!(run.task("t0").unwrap().orch.refresh, None);
    });

    // Committed, the same request is accepted: the refresh is due, or already in
    // flight, since the rig's worker turn is closed.
    git(&worktree, &["commit", "-q", "-am", "more work"]);
    let reply = rig.request(refresh_t0(&rig)).await;
    assert!(matches!(reply, RunReply::Done { .. }), "{reply:?}");
    let refresh = rig.run(|run| run.task("t0").unwrap().orch.refresh.clone());
    assert!(
        matches!(refresh, Some(RefreshState::Due | RefreshState::InFlight(_))),
        "{refresh:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untracked_file_is_not_an_uncommitted_change() {
    let rig = Rig::new(with_worktree).await;
    let worktree = rig.run(|run| run.task("t0").unwrap().worktree.clone());
    std::fs::write(worktree.join("scratch.txt"), "notes\n").unwrap();
    let reply = rig.request(refresh_t0(&rig)).await;
    assert!(matches!(reply, RunReply::Done { .. }), "{reply:?}");
}
