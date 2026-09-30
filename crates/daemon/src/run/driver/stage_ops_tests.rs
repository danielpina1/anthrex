//! Milestone 9.1 task M9.1.12: the merge candidate over a multi-stage run's refs
//! (decision 53), through `driver/merge.rs`'s candidate path, against a temporary git
//! repository of its own: the guard list, the all-or-nothing move of a stage ref and
//! `integration`, and the integration worktree left on `integration`.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{GitRoots, OpCtx, RunService, merge};
use crate::run::engine::{OpKind, OpResult};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

const STAGE_1: &str = "anthrex/r1/stage-1";
const STAGE_2: &str = "anthrex/r1/stage-2";
const INTEGRATION: &str = "anthrex/r1/integration";

/// `main` at a base commit, both stage branches and `integration` there, the
/// integration worktree on `integration`, and a task commit on its own branch.
struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    integration: PathBuf,
    base: String,
    task: String,
    service: Arc<RunService>,
    ctx: OpCtx,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        let root = top.join("repo");
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "user.name", "Stage Test"]);
        git(&root, &["config", "user.email", "stage@test"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        for branch in [STAGE_1, STAGE_2, INTEGRATION] {
            git(&root, &["branch", branch, &base]);
        }
        git(&root, &["checkout", "-q", "-b", "anthrex/r1/t1"]);
        std::fs::write(root.join("t1.txt"), "t1\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "t1"]);
        let task = git(&root, &["rev-parse", "HEAD"]);
        git(&root, &["checkout", "-q", "main"]);
        let integration = top.join("wt/runs/r1/integration");
        let at = integration.to_string_lossy().to_string();
        git(&root, &["worktree", "add", "-q", &at, INTEGRATION]);
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let service = RunService::for_manager(&manager, top.join("data"), Arc::new(NoRoots));
        let ctx = OpCtx {
            run_id: "r1".into(),
            project: root.clone(),
            data_dir: top.join("data/runs/r1"),
            git_timeout: Duration::from_secs(30),
            check_timeout: Duration::from_secs(30),
            confine: None,
        };
        Rig {
            _tmp: tmp,
            root,
            integration,
            base,
            task,
            service,
            ctx,
        }
    }

    fn head(&self, branch: &str) -> String {
        git(&self.root, &["rev-parse", &format!("refs/heads/{branch}")])
    }

    /// A candidate of the task into `branch`, every run ref guarded at the base.
    fn candidate(&self, branch: &str, also_integration: bool) -> OpKind {
        let guarded = [STAGE_1, STAGE_2, INTEGRATION]
            .map(|b| (b.to_string(), self.base.clone()))
            .to_vec();
        OpKind::MergeCandidate {
            root: self.root.clone(),
            integration: self.integration.clone(),
            run_branch: branch.into(),
            expected_run_head: self.base.clone(),
            base_branch: "main".into(),
            expected_base: self.base.clone(),
            task_head: self.task.clone(),
            message: "anthrex: merge t1: t1".into(),
            check: None,
            timeout_secs: 30,
            env: Vec::new(),
            guarded,
            also_integration,
        }
    }

    async fn run(&self, kind: OpKind) -> OpResult {
        merge::candidate(&self.service, &self.ctx, kind)
            .await
            .unwrap()
    }

    /// The integration worktree is on `integration`, with a clean tree.
    fn assert_on_integration(&self) {
        let on = git(&self.integration, &["symbolic-ref", "-q", "HEAD"]);
        assert_eq!(on, format!("refs/heads/{INTEGRATION}"));
        let status = git(&self.integration, &["status", "--porcelain"]);
        assert!(status.is_empty(), "{status}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_into_the_highest_stage_moves_it_and_integration_together() {
    let rig = Rig::new();
    let OpResult::Merged { commit } = rig.run(rig.candidate(STAGE_2, true)).await else {
        panic!("not merged")
    };
    assert_eq!(rig.head(STAGE_2), commit);
    assert_eq!(rig.head(INTEGRATION), commit);
    assert_eq!(rig.head(STAGE_1), rig.base);
    let parents = git(&rig.root, &["rev-list", "--parents", "-n", "1", &commit]);
    assert_eq!(parents, format!("{commit} {} {}", rig.base, rig.task));
    rig.assert_on_integration();
    assert!(rig.integration.join("t1.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_merge_into_a_lower_stage_moves_only_its_ref() {
    let rig = Rig::new();
    let OpResult::Merged { commit } = rig.run(rig.candidate(STAGE_1, false)).await else {
        panic!("not merged")
    };
    assert_eq!(rig.head(STAGE_1), commit);
    assert_eq!(rig.head(STAGE_2), rig.base);
    assert_eq!(rig.head(INTEGRATION), rig.base);
    rig.assert_on_integration();
    assert!(!rig.integration.join("t1.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn guard_refuses_when_any_guarded_ref_moved() {
    let rig = Rig::new();
    // Stage 2 moved, though the merge is into stage 1: the run halts, nothing moves.
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{STAGE_2}"), &rig.task],
    );
    let result = rig.run(rig.candidate(STAGE_1, false)).await;
    let reason = format!(
        "refs/heads/{STAGE_2} moved from {} to {}",
        &rig.base[..7],
        &rig.task[..7]
    );
    assert_eq!(result, OpResult::RefMoved { reason });
    assert_eq!(rig.head(STAGE_1), rig.base);
    assert_eq!(rig.head(INTEGRATION), rig.base);
}

#[tokio::test(flavor = "multi_thread")]
async fn create_stage_branch_is_created_once_and_refused_elsewhere() {
    let rig = Rig::new();
    let kind = |from: &str| OpKind::CreateStageBranch {
        root: rig.root.clone(),
        branch: "anthrex/r1/stage-3".into(),
        from: from.into(),
    };
    let made = super::create(&rig.service, &rig.ctx, kind(&rig.base)).await;
    assert_eq!(made, OpResult::StageCreated);
    assert_eq!(rig.head("anthrex/r1/stage-3"), rig.base);
    // A replay at the same commit is the same result; anywhere else, a moved ref.
    let again = super::create(&rig.service, &rig.ctx, kind(&rig.base)).await;
    assert_eq!(again, OpResult::StageCreated);
    let elsewhere = super::create(&rig.service, &rig.ctx, kind(&rig.task)).await;
    assert!(
        matches!(elsewhere, OpResult::RefMoved { .. }),
        "{elsewhere:?}"
    );
    assert_eq!(rig.head("anthrex/r1/stage-3"), rig.base);
}
