//! Task M9.5.15, parts 2 and 3: a race lane's ops through the driver's executor
//! (`ops::run` → `lane_ops`), against a temporary repository with a real lane checkout
//! (its own repository in the run's data directory, as `PrepareWorktree` makes it).
//! The git functions alone are `daemon/tests/run_git_crown.rs`'s.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{GitRoots, OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

const T: Duration = Duration::from_secs(30);
const TASK_BRANCH: &str = "anthrex/r1/t1";
const LANE_BRANCH: &str = "anthrex/r1/t1.a";

/// git in `dir` with no configuration of the machine's and none of ours in the
/// environment; its stdout, trimmed.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=w",
            "-c",
            "user.email=w@w",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A repository at a base commit, lane a's checkout prepared from it, the service and
/// the op context of run `r1`.
struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    lane: PathBuf,
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
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        git(&root, &["add", "-A"]);
        git(&root, &["commit", "-q", "-m", "base"]);
        let base = git(&root, &["rev-parse", "HEAD"]);
        let data_dir = top.join("data/runs/r1");
        let lane = top.join("wt/runs/r1/t1.a");
        let repo = git::checkout_repo_dir(&data_dir, &lane);
        git::prepare_task_worktree(
            std::ffi::OsStr::new("git"),
            &root,
            LANE_BRANCH,
            &base,
            &lane,
            &repo,
            T,
        )
        .unwrap();
        let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        let (manager, _events) = WindowManager::new(config);
        let run_ctx = crate::run::driver::RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        );
        let service = RunService::new(manager, run_ctx);
        let ctx = OpCtx {
            run_id: "r1".into(),
            project: root.clone(),
            data_dir,
            git_timeout: T,
            check_timeout: T,
            confine: None,
        };
        Rig {
            _tmp: tmp,
            root,
            lane,
            service,
            ctx,
        }
    }

    /// The racer commits `file` in the lane checkout (detached `HEAD`, its objects in
    /// the checkout's own store); the engine imports it, as every gate does. Its head.
    fn racer_commits(&self, file: &str) -> String {
        std::fs::write(self.lane.join(file), format!("{file}\n")).unwrap();
        git(&self.lane, &["add", "-A"]);
        git(&self.lane, &["commit", "-q", "-m", file]);
        let head = git(&self.lane, &["rev-parse", "HEAD"]);
        assert_eq!(
            git::sync(std::ffi::OsStr::new("git"), &self.lane, T).unwrap(),
            head
        );
        head
    }

    fn branch(&self, name: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["rev-parse", "-q", "--verify", &format!("refs/heads/{name}")])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8(out.stdout).unwrap().trim().to_string())
    }

    fn run(&self, kind: OpKind) -> OpResult {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(super::super::run(&self.service, &self.ctx, 1, kind))
    }

    fn crown(&self, lane_head: &str) -> OpResult {
        self.run(OpKind::CrownRacer {
            root: self.root.clone(),
            task_branch: TASK_BRANCH.into(),
            lane_head: lane_head.into(),
            adopt: false,
            checkout: self.lane.clone(),
        })
    }

    fn own(&self) -> Option<String> {
        crate::worktree::pinned::own_ref(&self.lane)
    }
}

/// Review focus 6: after the crown every import moves the task branch, never the lane
/// branch, or the merge queue would merge a stale head.
#[test]
fn after_the_crown_imports_move_the_task_branch() {
    let rig = Rig::new();
    let first = rig.racer_commits("one.txt");
    assert_eq!(rig.branch(LANE_BRANCH), Some(first.clone()));
    assert_eq!(
        rig.branch(TASK_BRANCH),
        None,
        "no task branch before a crown"
    );

    assert_eq!(
        rig.crown(&first),
        OpResult::Crowned {
            head: first.clone()
        }
    );
    assert_eq!(rig.branch(TASK_BRANCH), Some(first.clone()));
    assert_eq!(rig.own(), Some(format!("refs/heads/{TASK_BRANCH}")));

    let second = rig.racer_commits("two.txt");
    assert_eq!(
        rig.branch(TASK_BRANCH),
        Some(second),
        "the task branch moved"
    );
    assert_eq!(
        rig.branch(LANE_BRANCH),
        Some(first),
        "the lane branch stays where the crown found it"
    );
}

#[test]
fn a_moved_branch_is_not_repinned() {
    let rig = Rig::new();
    let first = rig.racer_commits("one.txt");
    let base = git(&rig.root, &["rev-parse", "main"]);
    git(&rig.root, &["branch", TASK_BRANCH, &base]);

    let result = rig.crown(&first);
    assert!(
        matches!(result, OpResult::RefMoved { .. }),
        "a task branch at another commit: {result:?}"
    );
    assert_eq!(rig.own(), Some(format!("refs/heads/{LANE_BRANCH}")));
    assert_eq!(
        rig.branch(TASK_BRANCH),
        Some(base),
        "the moved branch is untouched"
    );
}

/// The lane checkout's own git directory, `tasks/t1.a/git`.
fn lane_git_dir(rig: &Rig) -> PathBuf {
    git::Repo::at(&git::checkout_repo_dir(&rig.ctx.data_dir, &rig.lane)).git_dir()
}

fn removal(rig: &Rig, keep_path: bool, clear_locks: bool) -> OpKind {
    OpKind::RemoveWorktree {
        root: rig.root.clone(),
        path: rig.lane.clone(),
        salvage_ref: "refs/anthrex/salvage/r1/t1/1".into(),
        keep_head: true,
        clear_locks,
        keep_path,
    }
}

/// Ruling RR-2: `keep_path` means the racer did not exit, so a lock in its checkout
/// may be a live process's. It is never cleared then, even with `clear_locks`.
#[test]
fn keep_path_salvages_and_leaves_the_checkout() {
    let rig = Rig::new();
    let head = rig.racer_commits("one.txt");
    let locks = ["index.lock", "HEAD.lock"].map(|name| lane_git_dir(&rig).join(name));
    for lock in &locks {
        std::fs::write(lock, "").unwrap();
    }

    let result = rig.run(removal(&rig, true, true));
    assert_eq!(
        result,
        OpResult::Removed {
            salvage_ref: Some("refs/anthrex/salvage/r1/t1/1".into()),
            cleared_locks: Vec::new(),
        }
    );
    let salvaged = git(&rig.root, &["rev-parse", "refs/anthrex/salvage/r1/t1/1"]);
    assert_eq!(salvaged, head, "a clean checkout is salvaged at its head");
    assert!(rig.lane.join("one.txt").is_file(), "the checkout is kept");
    for lock in &locks {
        assert!(
            lock.is_file(),
            "every lock in it is left: {}",
            lock.display()
        );
    }
}

/// Review m3: the locks are already gone when a later step fails, so the failure
/// names them.
#[test]
fn a_failed_removal_still_names_the_locks_it_cleared() {
    let rig = Rig::new();
    rig.racer_commits("one.txt");
    let base = git(&rig.root, &["rev-parse", "main"]);
    // Another salvage already holds this ref: the clean checkout's keep_head refuses.
    git(
        &rig.root,
        &["update-ref", "refs/anthrex/salvage/r1/t1/1", &base],
    );
    for name in ["index.lock", "HEAD.lock"] {
        std::fs::write(lane_git_dir(&rig).join(name), "").unwrap();
    }

    let result = rig.run(removal(&rig, false, true));
    let OpResult::Failed { message } = result else {
        panic!("the salvage cannot succeed: {result:?}");
    };
    assert!(message.contains("holds other work"), "{message}");
    for name in ["index.lock", "HEAD.lock"] {
        assert!(
            message.contains(&format!("removed a stale {name}")),
            "{message}"
        );
        assert!(!lane_git_dir(&rig).join(name).exists());
    }
    assert!(rig.lane.exists(), "nothing is removed without its salvage");
}

/// Review m2: the locks are cleared inside the repository's git write queue, never
/// beside a write that holds it.
#[test]
fn the_locks_are_cleared_inside_the_write_queue() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let rig = Rig::new();
    rig.racer_commits("one.txt");
    let locks = ["index.lock", "HEAD.lock"].map(|name| lane_git_dir(&rig).join(name));
    for lock in &locks {
        std::fs::write(lock, "").unwrap();
    }
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (held, release) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let (h, r) = (held.clone(), release.clone());
    let queue = rig.service.queue.clone();
    let project = rig.ctx.project.clone();
    let holder = rt.spawn(async move {
        queue
            .write(&project, move || {
                h.store(true, Ordering::SeqCst);
                let deadline = std::time::Instant::now() + Duration::from_secs(30);
                while !r.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(())
            })
            .await
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !held.load(Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "the holder never ran");
        std::thread::sleep(Duration::from_millis(5));
    }
    let (service, ctx) = (rig.service.clone(), rig.ctx.clone());
    let kind = removal(&rig, false, true);
    let removal = rt.spawn(async move { super::super::run(&service, &ctx, 1, kind).await });
    // A clearing outside the queue happens at once; give it the time to show.
    std::thread::sleep(Duration::from_millis(300));
    for lock in &locks {
        assert!(lock.is_file(), "cleared while another write held the queue");
    }
    release.store(true, Ordering::SeqCst);
    rt.block_on(holder).unwrap().unwrap();
    let result = rt.block_on(removal).unwrap();
    let OpResult::Removed { cleared_locks, .. } = result else {
        panic!("not removed: {result:?}");
    };
    assert_eq!(cleared_locks, vec!["index.lock", "HEAD.lock"]);
}

#[test]
fn a_lanes_removal_clears_its_stale_locks_first() {
    let rig = Rig::new();
    rig.racer_commits("one.txt");
    std::fs::write(rig.lane.join("one.txt"), "changed\n").unwrap();
    for name in ["index.lock", "HEAD.lock"] {
        std::fs::write(lane_git_dir(&rig).join(name), "").unwrap();
    }

    let result = rig.run(removal(&rig, false, true));
    let OpResult::Removed {
        salvage_ref,
        cleared_locks,
    } = result
    else {
        panic!("not removed: {result:?}");
    };
    assert_eq!(salvage_ref.as_deref(), Some("refs/anthrex/salvage/r1/t1/1"));
    assert_eq!(cleared_locks, vec!["index.lock", "HEAD.lock"]);
    assert!(!rig.lane.exists(), "the checkout is removed");
    let tree = git(&rig.root, &["show", "refs/anthrex/salvage/r1/t1/1:one.txt"]);
    assert_eq!(tree, "changed", "the salvage holds the dirty file");
}

/// Review m4: a racer's session works in its lane's stored checkout.
#[test]
fn a_racers_checkout_is_its_lanes_stored_one() {
    use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};
    use proto::{LaneState, RaceLane};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    let mut race = race_of(&run.tasks[0], [LaneState::Working, LaneState::Review]);
    race.lanes[1].checkout = "t1.lane-b".into();
    run.tasks[0].race = Some(race);
    assert_eq!(
        super::checkout_of(&run, "t1", Some(RaceLane::B)),
        "t1.lane-b"
    );
    assert_eq!(super::checkout_of(&run, "t1", None), "t1");
}
