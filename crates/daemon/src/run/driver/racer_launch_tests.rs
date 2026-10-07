//! Task M9.5.18: racers' and test writers' sessions through the driver. Their
//! `CreateWindow` sandbox (`ops::worker_git_dirs`) against a temporary repository with
//! real lane checkouts (each its own repository in the run's data directory, as
//! `PrepareWorktree` makes it), and a racer's tool call through a real daemon socket,
//! forwarded by `mcp::forward` exactly as `anthrex mcp` forwards it. No agent runs.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use proto::{AgentRole, LaneState, RaceLane};
use serde_json::json;

use crate::headless::HeadlessSpec;
use crate::headless::codex_sandbox::{CodexSandboxDialect, DialectChoice};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{EventKind, GitRoots, Msg, OpCtx, RunContext, RunService};
use crate::run::git;
use crate::run::model::Run;
use crate::run::role_launch::{task_tmp_dir, worker_git_roots};
use crate::run::role_launch_patterns::{racer_spec, test_writer_spec};
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

const T: Duration = Duration::from_secs(30);

/// git in `dir` with no configuration of the machine's and none of ours in the
/// environment; its stdout, trimmed.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=w", "-c", "user.email=w@w"])
        .args([
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

/// Run `r1` over a temporary repository: `t1` racing (lane a on Claude, lane b on
/// Codex, both `working`, sandboxed), with the checkouts `prepared` (`t1`, `t1.a`,
/// `t1.b`) made from the base commit, and the service that holds the run.
struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    data_dir: PathBuf,
    service: Arc<RunService>,
    ctx: OpCtx,
}

impl Rig {
    /// A rig whose Codex CLI speaks the profiles dialect.
    fn new(prepared: &[&str], change: impl FnOnce(&mut Run)) -> Rig {
        Rig::with_dialect(prepared, CodexSandboxDialect::Profiles, change)
    }

    /// A rig whose Codex CLI speaks `dialect`, fixed so no other test's probe of the
    /// process-wide recorded version can change it.
    fn with_dialect(
        prepared: &[&str],
        dialect: CodexSandboxDialect,
        change: impl FnOnce(&mut Run),
    ) -> Rig {
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

        let mut run = run_ok(&plan_with(
            PROFILE,
            &[task_toml("t1", "M", "[\"crates/a/**\"]", "")],
        ));
        run.id = "r1".into();
        run.wt_dir = top.join("wt");
        run.data_dir = data_dir.clone();
        run.git_common_dir = root.join(".git");
        run.limits.worker_sandbox = true;
        let race = race_of(&run.tasks[0], [LaneState::Working, LaneState::Working]);
        run.tasks[0].race = Some(race);
        run.tasks[0].worktree = run.task_path("t1");
        for checkout in prepared {
            let path = run.task_path(checkout);
            let branch = format!("anthrex/r1/{checkout}");
            let repo = git::checkout_repo_dir(&data_dir, &path);
            git::prepare_task_worktree(
                std::ffi::OsStr::new("git"),
                &root,
                &branch,
                &base,
                &path,
                &repo,
                T,
            )
            .unwrap();
        }
        change(&mut run);

        let mut config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
        config.cli_caps.codex_sandbox = DialectChoice::Fixed(dialect);
        let (manager, _events) = WindowManager::new(config);
        let run_ctx = RunContext::new(
            top.join("data"),
            manager.config(),
            config::Orchestrator::default(),
            Arc::new(NoRoots),
        );
        let service = RunService::new(manager, run_ctx);
        crate::lock(&service.state).runs.insert("r1".into(), run);
        let ctx = OpCtx {
            run_id: "r1".into(),
            project: root.clone(),
            data_dir: data_dir.clone(),
            git_timeout: T,
            check_timeout: T,
            confine: None,
        };
        Rig {
            _tmp: tmp,
            root,
            data_dir,
            service,
            ctx,
        }
    }

    fn run<R>(&self, read: impl FnOnce(&Run) -> R) -> R {
        read(&crate::lock(&self.service.state).runs["r1"])
    }

    /// Lane `lane`'s racer spec, built from the run's own race.
    fn racer(&self, lane: RaceLane) -> HeadlessSpec {
        self.run(|run| {
            let task = &run.tasks[0];
            let stored = task.race.as_ref().unwrap().lanes.iter();
            let stored = stored.clone().find(|l| l.lane == lane).unwrap().clone();
            racer_spec(run, task, &stored)
        })
    }

    /// `ops::worker_git_dirs` on `spec`, as `CreateWindow` runs it.
    fn complete(&self, spec: &mut HeadlessSpec) -> Result<(), String> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(super::worker_git_dirs(&self.service, &self.ctx, spec))
    }

    /// The repository (git directory, objects, temporary directory's owner) of the
    /// checkout named `checkout`.
    fn repo(&self, checkout: &str) -> PathBuf {
        git::checkout_repo_dir(&self.data_dir, Path::new(checkout))
    }

    fn git_dir(&self, checkout: &str) -> PathBuf {
        git::Repo::at(&self.repo(checkout)).git_dir()
    }
}

/// Every writable root of `spec`, on whichever runtime it runs.
fn granted(spec: &HeadlessSpec) -> Vec<PathBuf> {
    let mut roots = spec.codex_writable_roots.clone();
    if let Some(sandbox) = &spec.claude_sandbox {
        roots.extend(sandbox.writable_roots.iter().cloned());
    }
    roots
}

/// Whether `path` is one of `roots` or under one: granted.
fn covers(roots: &[PathBuf], path: &Path) -> bool {
    roots.iter().any(|root| path.starts_with(root))
}

/// Whether any of `roots` is `dir` or under it.
fn reaches(roots: &[PathBuf], dir: &Path) -> bool {
    roots.iter().any(|root| root.starts_with(dir))
}

/// Task 15's addendum: a racer's sandbox is completed like a worker's, on its own
/// lane's checkout: its objects, its temporary directory and the commit's files in its
/// own git directory (exactly a worker's grant there, task 18 review m1), and nothing
/// of the other lane (its temporary directory included), of the task's own checkout or
/// of the user's repository.
#[test]
fn a_racers_sandbox_covers_its_own_lanes_git_dir_only() {
    let rig = Rig::new(&["t1", "t1.a", "t1.b"], |_| {});
    let common = rig.root.join(".git").canonicalize().unwrap();
    for (lane, own, other) in [(RaceLane::A, "t1.a", "t1.b"), (RaceLane::B, "t1.b", "t1.a")] {
        let mut spec = rig.racer(lane);
        let denied_before = spec
            .claude_sandbox
            .as_ref()
            .map(|s| s.deny_write.clone())
            .unwrap_or_default();
        rig.complete(&mut spec).unwrap();
        let roots = granted(&spec);
        assert!(
            covers(&roots, &rig.git_dir(own).join("index")),
            "{lane:?}: {roots:?}"
        );
        assert!(
            roots.contains(&rig.repo(own).join("git/objects")),
            "{lane:?}: {roots:?}"
        );
        // Task 18 review m1: exactly a worker's grant, computed for the lane's checkout,
        // in its runtime's shape (lane a is Claude's, lane b Codex's on the profiles
        // dialect: both the host's).
        let shape =
            super::grant_shape(spec.claude_sandbox.is_some(), CodexSandboxDialect::Profiles);
        assert_eq!(shape, git::GrantShape::host(), "{lane:?}");
        let (git_common, own_path) =
            rig.run(|run| (run.git_common_dir.clone(), run.task_path(own)));
        let roots_of_own = worker_git_roots(&rig.data_dir, own);
        let worker = git::worker_git_grant(&git_common, &own_path, &roots_of_own, shape).unwrap();
        let mut expected = worker.writable.clone();
        let mut racer = roots.clone();
        expected.sort();
        racer.sort();
        assert_eq!(racer, expected, "{lane:?}: a worker's grant on {own}");
        if let Some(sandbox) = &spec.claude_sandbox {
            let mut denied = denied_before.clone();
            denied.extend(worker.deny.iter().cloned());
            assert_eq!(sandbox.deny_write, denied, "{lane:?}");
        } else {
            // Ruling R3: the checkout's `.git` (the rig's checkouts have none of the
            // protected agent-config paths), then the grant's denials, read-only.
            let mut read_only = vec![own_path.join(".git")];
            read_only.extend(worker.deny.iter().cloned());
            assert!(own_path.join(".git").exists(), "{lane:?}");
            assert_eq!(spec.codex_read_only, read_only, "{lane:?}");
        }
        let (other_path, task_path) = rig.run(|run| (run.task_path(other), run.task_path("t1")));
        for forbidden in [
            rig.repo(other),
            // The other lane's temporary directory is outside its repository.
            task_tmp_dir(&rig.data_dir, other),
            other_path,
            rig.repo("t1"),
            task_path,
            common.clone(),
            rig.root.clone(),
        ] {
            assert!(
                !reaches(&roots, &forbidden),
                "{lane:?} reaches {}: {roots:?}",
                forbidden.display()
            );
        }
    }
}

/// Ruling R3: under the legacy Codex dialect, which cannot keep a path read-only inside
/// a writable one, a Codex racer keeps the exact files and gets no read-only entry
/// (one would turn its whole session read-only), on every host.
#[test]
fn a_legacy_codex_racer_gets_the_files_grant_and_no_read_only_entries() {
    let rig = Rig::with_dialect(&["t1", "t1.a", "t1.b"], CodexSandboxDialect::Legacy, |_| {});
    let mut spec = rig.racer(RaceLane::B);
    assert!(spec.claude_sandbox.is_none(), "lane b is Codex's");
    rig.complete(&mut spec).unwrap();
    let (git_common, own_path) = rig.run(|run| (run.git_common_dir.clone(), run.task_path("t1.b")));
    let roots = worker_git_roots(&rig.data_dir, "t1.b");
    let files = git::worker_git_grant(&git_common, &own_path, &roots, git::GrantShape::Files);
    let (mut want, mut got) = (files.unwrap().writable, spec.codex_writable_roots.clone());
    want.sort();
    got.sort();
    assert_eq!(got, want);
    assert!(
        spec.codex_read_only.is_empty(),
        "{:?}",
        spec.codex_read_only
    );
}

/// Hardening: the protected agent-config paths that exist in a Codex worker's checkout
/// at launch (here `AGENTS.md` and `.codex/`, made on the real filesystem) are among
/// its read-only entries on the profiles dialect; absent ones are not.
#[test]
fn a_codex_racers_existing_protected_paths_are_read_only_entries() {
    let rig = Rig::new(&["t1", "t1.a", "t1.b"], |_| {});
    let own_path = rig.run(|run| run.task_path("t1.b"));
    std::fs::write(own_path.join("AGENTS.md"), "x\n").unwrap();
    std::fs::create_dir(own_path.join(".codex")).unwrap();
    let mut spec = rig.racer(RaceLane::B);
    assert!(spec.claude_sandbox.is_none(), "lane b is Codex's");
    rig.complete(&mut spec).unwrap();
    for present in ["AGENTS.md", ".codex"] {
        assert!(
            spec.codex_read_only.contains(&own_path.join(present)),
            "{present}: {:?}",
            spec.codex_read_only
        );
    }
    assert!(
        !spec.codex_read_only.contains(&own_path.join("CLAUDE.md")),
        "{:?}",
        spec.codex_read_only
    );
}

/// Hardening: every read-only entry spells the checkout as the grant does (its
/// canonical path), whatever spelling the session's working directory has (macOS
/// `/tmp` for `/private/tmp`, here a symbolic link made on the real filesystem).
#[test]
fn a_codex_racers_read_only_entries_use_the_canonical_checkout() {
    let rig = Rig::new(&["t1", "t1.a", "t1.b"], |_| {});
    let own_path = rig.run(|run| run.task_path("t1.b"));
    std::fs::write(own_path.join("AGENTS.md"), "x\n").unwrap();
    let link = rig.root.join("alias");
    std::os::unix::fs::symlink(&own_path, &link).unwrap();
    let mut spec = rig.racer(RaceLane::B);
    spec.cwd = link.clone();
    rig.complete(&mut spec).unwrap();
    let canonical = own_path.canonicalize().unwrap();
    assert!(
        spec.codex_read_only.contains(&canonical.join(".git")),
        "{:?}",
        spec.codex_read_only
    );
    assert!(
        spec.codex_read_only.contains(&canonical.join("AGENTS.md")),
        "{:?}",
        spec.codex_read_only
    );
    assert!(
        spec.codex_read_only.iter().all(|p| !p.starts_with(&link)),
        "{:?}",
        spec.codex_read_only
    );
}

/// A test writer's sandbox is completed on the task's own checkout, as a worker's.
#[test]
fn a_test_writers_sandbox_is_its_tasks_checkout() {
    let rig = Rig::new(&["t1"], |run| run.tasks[0].race = None);
    let mut spec = rig.run(|run| {
        let task = &run.tasks[0];
        test_writer_spec(run, task, &task.route)
    });
    assert_eq!(spec.run_ref.as_ref().unwrap().role, AgentRole::TestWriter);
    rig.complete(&mut spec).unwrap();
    let roots = granted(&spec);
    assert!(
        covers(&roots, &rig.git_dir("t1").join("index")),
        "{roots:?}"
    );
    assert!(
        roots.contains(&rig.repo("t1").join("git/objects")),
        "{roots:?}"
    );
}

/// Task 15's re-review N3: a lane the run has no stored `Lane` for must not fall back
/// to the task's own checkout. Its racer gets no sandbox roots at all: the launch is
/// refused, and nothing of the task's checkout is made for it. A racer that names no
/// lane is refused the same way.
#[test]
fn a_lane_with_no_stored_entry_is_refused_its_launch() {
    let rig = Rig::new(&["t1.b"], |_| {});
    let mut spec = rig.racer(RaceLane::B);
    let mut nameless = spec.clone();
    {
        let mut state = crate::lock(&rig.service.state);
        let race = state.runs.get_mut("r1").unwrap().tasks[0].race.as_mut();
        race.unwrap().lanes.retain(|l| l.lane == RaceLane::A);
    }
    let before = granted(&spec);
    let error = rig.complete(&mut spec).unwrap_err();
    assert!(
        error.contains("task t1 has no lane b") && error.contains("no sandbox"),
        "{error}"
    );
    assert_eq!(granted(&spec), before, "no root was resolved for it");
    assert!(
        !rig.repo("t1").join("git/objects").exists(),
        "nothing of the task's own checkout was made for lane b"
    );

    nameless.run_ref.as_mut().unwrap().lane = None;
    let error = rig.complete(&mut nameless).unwrap_err();
    assert!(error.contains("names no lane"), "{error}");
}

/// A racer's `task_done`, forwarded as `anthrex mcp --role racer --lane a` forwards it
/// over the daemon's socket, reaches the engine with its role and `lane: Some(A)`.
#[tokio::test(flavor = "multi_thread")]
async fn a_racers_tool_call_carries_its_lane() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-racer-call-")
        .tempdir_in("/tmp")
        .unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    let (manager, _events) = WindowManager::new(config);
    let wiring = crate::server::GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let ctx = RunContext::new(
        dir.path().join("data"),
        manager.config(),
        config::Orchestrator::default(),
        wiring.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    // The engine's queue, read here instead of by the engine loop.
    let mut events = crate::lock(&runs.rx).take().unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let server = tokio::spawn(crate::server::serve(
        listener,
        manager,
        wiring,
        runs.clone(),
        shutdown.clone(),
    ));
    let opts = mcp::McpOptions {
        role: AgentRole::Racer,
        run_id: "r1".into(),
        task_id: Some("t1".into()),
        scout_id: None,
        epic: None,
        chain: None,
        lane: Some(RaceLane::A),
        window_id: 902,
        socket,
    };
    let args = json!({"summary": "did it", "test": "a::works", "red": "abcdef1"});
    let call = tokio::spawn(async move { mcp::forward(&opts, "task_done", args).await });

    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("the call reaches the engine's queue")
        .expect("the queue is open");
    let Msg::Event(EventKind::Tool { call: seen, .. }) = event else {
        panic!("not a tool call");
    };
    assert_eq!(
        (
            seen.role,
            seen.task_id.as_deref(),
            seen.window_id,
            seen.lane
        ),
        (AgentRole::Racer, Some("t1"), 902, Some(RaceLane::A))
    );
    assert_eq!(seen.tool, "task_done");
    call.abort();
    shutdown.cancel();
    let _ = server.await;
}
