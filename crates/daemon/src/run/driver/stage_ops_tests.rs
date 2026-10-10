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
        Rig::with_wrapper(|_| None)
    }

    /// A rig whose service runs git through the script `wrapper(top)` returns, written
    /// to `<top>/git-wrapper.sh` before the service exists.
    fn with_wrapper(wrapper: impl FnOnce(&Path) -> Option<String>) -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let top = tmp.path().canonicalize().unwrap();
        if let Some(script) = wrapper(&top) {
            testexec::write_executable(top.join("git-wrapper.sh"), script);
        }
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
        let mut run_ctx = crate::run::driver::RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        );
        // Controller ruling C-16 (2): a wrapper around git, when the test wrote one.
        let wrapper = top.join("git-wrapper.sh");
        if wrapper.exists() {
            run_ctx.git = wrapper.into_os_string();
        }
        let service = RunService::new(manager, run_ctx);
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
            tier: None,
        }
    }

    async fn run(&self, kind: OpKind) -> OpResult {
        merge::candidate(&self.service, &self.ctx, 1, kind)
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
    let OpResult::Merged { commit, .. } = rig.run(rig.candidate(STAGE_2, true)).await else {
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
    let OpResult::Merged { commit, .. } = rig.run(rig.candidate(STAGE_1, false)).await else {
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

impl Rig {
    fn refs(&self) -> super::RunRefs {
        super::RunRefs {
            run_id: "r1".into(),
            root: self.root.clone(),
            project: self.root.clone(),
            base_branch: "main".into(),
            run_branch: INTEGRATION.into(),
            stages: vec![(1, STAGE_1.into()), (2, STAGE_2.into())],
            timeout: Duration::from_secs(30),
            may_move: true,
            now: 1_700_000_000,
        }
    }

    fn salvage_refs(&self) -> Vec<(String, String)> {
        let text = git(
            &self.root,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/anthrex/salvage/r1/",
            ],
        );
        text.lines()
            .filter_map(|l| l.split_once(' '))
            .map(|(r, o)| (r.to_string(), o.to_string()))
            .collect()
    }
}

/// Controller ruling C-15 (I-1), the review's scenario: `integration` moved by hand to
/// X, then `run resume --rebaseline`. It goes back to the highest stage's head, X is
/// salvaged, the guard expects that head, and the next merge proceeds.
#[tokio::test(flavor = "multi_thread")]
async fn rebaseline_puts_a_moved_integration_back_and_salvages_it() {
    let rig = Rig::new();
    let x = rig.task.clone();
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{INTEGRATION}"), &x],
    );
    let read = super::rebaseline(&rig.service, rig.refs()).await.unwrap();
    assert_eq!(read.head, rig.base);
    assert_eq!(
        read.stages,
        vec![(1, rig.base.clone()), (2, rig.base.clone())]
    );
    let (old, salvage) = read.salvaged.clone().expect("salvaged");
    assert_eq!(old, x);
    assert!(
        salvage.starts_with("refs/anthrex/salvage/r1/_integration-"),
        "{salvage}"
    );
    assert_eq!(rig.salvage_refs(), vec![(salvage, x.clone())]);
    assert_eq!(rig.head(INTEGRATION), rig.base);
    // The next merge, guarded at the rebaselined heads, lands.
    let OpResult::Merged { commit, .. } = rig.run(rig.candidate(STAGE_2, true)).await else {
        panic!("not merged")
    };
    assert_eq!(rig.head(INTEGRATION), commit);
}

/// A user moved the highest `stage-<n>`: its head is adopted, and `integration` follows
/// it, the commit it was at salvaged.
#[tokio::test(flavor = "multi_thread")]
async fn rebaseline_adopts_a_moved_highest_stage() {
    let rig = Rig::new();
    let y = rig.task.clone();
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{STAGE_2}"), &y],
    );
    let read = super::rebaseline(&rig.service, rig.refs()).await.unwrap();
    assert_eq!(read.stages, vec![(1, rig.base.clone()), (2, y.clone())]);
    assert_eq!(read.head, y);
    assert_eq!(
        read.salvaged.as_ref().map(|(o, _)| o.clone()),
        Some(rig.base.clone())
    );
    assert_eq!(rig.head(INTEGRATION), y);
    assert_eq!(rig.head(STAGE_2), y);
    assert_eq!(rig.head("main"), rig.base, "the base never moves");
    // Nothing to put back: no second salvage.
    let again = super::rebaseline(&rig.service, rig.refs()).await.unwrap();
    assert_eq!(again.salvaged, None);
    assert_eq!(rig.salvage_refs().len(), 1);
}

/// Controller ruling C-15 (M-4): a symbolic ref where a stage branch should be is a
/// moved ref, even when it resolves to `from`.
#[tokio::test(flavor = "multi_thread")]
async fn create_stage_branch_refuses_a_symbolic_ref() {
    let rig = Rig::new();
    let branch = "anthrex/r1/stage-3";
    let refname = format!("refs/heads/{branch}");
    git(&rig.root, &["symbolic-ref", &refname, "refs/heads/main"]);
    let kind = OpKind::CreateStageBranch {
        root: rig.root.clone(),
        branch: branch.into(),
        from: rig.base.clone(),
    };
    let result = super::create(&rig.service, &rig.ctx, kind).await;
    assert_eq!(
        result,
        OpResult::RefMoved {
            reason: format!("{refname} is a symbolic ref to refs/heads/main")
        }
    );
}

/// Controller ruling C-16 (2): the driver's swap verifies every guarded ref it does not
/// move. A git wrapper (the test's own script, acting only on the test's own repository)
/// moves `stage-2` just before the swap's `update-ref --stdin`, after both guards have
/// passed: the merge into stage 1 is refused, naming stage 2, and stage 1 stays.
#[tokio::test(flavor = "multi_thread")]
async fn the_swap_is_refused_when_a_guarded_ref_moves_after_the_guard() {
    let real = which_git();
    let rig = Rig::with_wrapper(|top| {
        let root = top.join("repo");
        let marker = top.join("moved");
        Some(format!(
            r#"#!/bin/sh
case " $* " in
  *" update-ref "*" --stdin "*)
    if [ ! -e '{marker}' ]; then
      : > '{marker}'
      task=$('{real}' -C '{root}' rev-parse refs/heads/anthrex/r1/t1)
      '{real}' -C '{root}' update-ref refs/heads/{STAGE_2} "$task"
    fi
    ;;
esac
exec '{real}' "$@"
"#,
            marker = marker.display(),
            root = root.display(),
        ))
    });
    let result = rig.run(rig.candidate(STAGE_1, false)).await;
    assert_eq!(
        result,
        OpResult::RefMoved {
            reason: format!("refs/heads/{STAGE_2} moved during the merge")
        }
    );
    assert_eq!(rig.head(STAGE_1), rig.base);
    assert_eq!(rig.head(STAGE_2), rig.task, "the wrapper did move it");
    assert_eq!(rig.head(INTEGRATION), rig.base);
}

/// The absolute path of the real git, for a wrapper script.
fn which_git() -> String {
    let out = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let path = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(path.starts_with('/'), "{path}");
    path
}

/// Controller ruling C-16 (4): two salvages in the same second take a free suffix.
#[tokio::test(flavor = "multi_thread")]
async fn two_salvages_in_one_second_do_not_collide() {
    let rig = Rig::new();
    let int = format!("refs/heads/{INTEGRATION}");
    git(&rig.root, &["update-ref", &int, &rig.task]);
    let first = super::rebaseline(&rig.service, rig.refs()).await.unwrap();
    git(&rig.root, &["update-ref", &int, &rig.task]);
    let second = super::rebaseline(&rig.service, rig.refs()).await.unwrap();
    let name = |r: &crate::run::engine::Rebaseline| r.salvaged.clone().unwrap().1;
    let base = "refs/anthrex/salvage/r1/_integration-1700000000";
    assert_eq!(name(&first), base);
    assert_eq!(name(&second), format!("{base}-2"));
    assert_eq!(rig.salvage_refs().len(), 2);
    assert_eq!(rig.head(INTEGRATION), rig.base);
}

/// Controller ruling C-16 (1): `run resume --rebaseline` of a run the engine will not
/// resume (here running) writes no ref: it is refused, `integration` stays where the
/// user moved it, and nothing is salvaged.
#[tokio::test(flavor = "multi_thread")]
async fn a_rebaseline_the_engine_refuses_writes_no_ref() {
    use crate::run::model::{StageLayout, StageRecord};
    use crate::run::test_support::{EXAMPLE_PLAN, run_ok};
    let rig = Rig::new();
    let x = rig.task.clone();
    git(
        &rig.root,
        &["update-ref", &format!("refs/heads/{INTEGRATION}"), &x],
    );
    let mut run = run_ok(EXAMPLE_PLAN);
    run.id = "r1".into();
    run.root = rig.root.clone();
    run.project = rig.root.clone();
    let top = rig.root.parent().unwrap().to_path_buf();
    run.data_dir = top.join("data/runs/r1");
    run.wt_dir = top.join("wt");
    run.base_sha = rig.base.clone();
    run.run_head = rig.base.clone();
    run.stage_layout = StageLayout::Multi;
    run.stages = [(1, STAGE_1), (2, STAGE_2)]
        .map(|(n, b)| StageRecord::new(n, b.into(), &rig.base, Default::default(), 0))
        .to_vec();
    run.state = proto::RunState::Running;
    // Nothing for the scheduler to start or complete: one task waits on the user, the
    // rest are cancelled.
    for task in &mut run.tasks {
        task.state = proto::TaskState::Cancelled;
    }
    run.tasks[0].state = proto::TaskState::Blocked;
    run.tasks[0].block = Some(proto::BlockInfo::new(proto::BlockReason::Human, "waiting"));
    crate::lock(&rig.service.state)
        .runs
        .insert("r1".into(), run);
    let handle = rig
        .service
        .spawn(tokio_util::sync::CancellationToken::new());
    let refused = rig.service.resume("r1".into(), true).await;
    let text = refused.expect_err("a running run is not resumed");
    assert!(text.contains("running"), "{text}");
    assert_eq!(rig.head(INTEGRATION), x);
    assert!(rig.salvage_refs().is_empty(), "{:?}", rig.salvage_refs());
    rig.service.stop().await;
    drop(handle);
}

// Task M9.1.17: the propagate, in a file of its own for AGENTS.md rule 8.
#[path = "stage_ops_propagate_tests.rs"]
mod propagate_tests;
