//! Milestone 9.16 task M9.16.8: the hold in the driver, against a real run loop and a
//! scratch data directory (decisions 16, 17, 19 and 20, rulings C9, C10 and C11). The
//! refusals use the chain rig (`chain_goal_tests.rs`): a real socket, a real checkout
//! with a stored profile, and a decider stand-in that records each call.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::brakes::DispatchHold;
use proto::run_wire::request;
use proto::{DeliveryMode, RunReply, RunRequest};
use tokio_util::sync::CancellationToken;

use super::RunService;
use super::brakes_io::{BRAKES_FILE, CORRUPT_BY};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::run::driver::orch::chain_goal::tests::{ANSWER, CHAIN, ChainRig, PREV};
use crate::run::engine::brakes::HOLD_TEXT;

/// One engine step and one `brakes.json` write (`BRAKES_IO_WAIT`, 5 s), with room for
/// a loaded machine; a hang guard, not a cost.
const STEP: Duration = Duration::from_secs(30);

const ON: &str =
    "dispatch held daemon-wide; running sessions continue · anthrex hold off releases it";

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

/// A spawned service over `<dir>/data`, no agent anywhere.
fn service(dir: &Path) -> (Arc<RunService>, Arc<WindowManager>) {
    let mut config = ManagerConfig::for_tests(dir.join("d.sock"), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    config.worktrees_root = dir.join("worktrees");
    config.launch_gate = crate::launch::LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, dir.join("data"), Arc::new(NoRoots));
    (s, manager)
}

async fn hold(s: &RunService, on: bool) -> RunReply {
    tokio::time::timeout(STEP, s.request(RunRequest::Hold { on }))
        .await
        .expect("answered")
}

fn held(reply: &RunReply) -> (Option<DispatchHold>, String) {
    match reply {
        RunReply::Held { hold, message, .. } => (hold.clone(), message.clone()),
        other => panic!("not held: {other:?}"),
    }
}

fn on_file(data: &Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(data.join(BRAKES_FILE)).unwrap()).unwrap()
}

/// Decision 17: on, on again, off, off again; the file first, the engine after, and
/// the replies' texts. No run's state moves (decision 18).
#[tokio::test(flavor = "multi_thread")]
async fn hold_on_and_off_answer_and_persist() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (s, _manager) = service(dir.path());
    let mut run = crate::run::orch::test_support::run_of(1);
    run.state = proto::RunState::Running;
    let id = run.id.clone();
    crate::lock(&s.state).runs.insert(id.clone(), run);
    let handle = s.spawn(CancellationToken::new());

    let (first, message) = held(&hold(&s, true).await);
    let first = first.expect("held");
    assert_eq!(first.by, "user");
    assert_eq!(message, ON);
    assert_eq!(
        on_file(&data),
        serde_json::json!({"v": 1, "hold": {"since": first.since, "by": "user"}})
    );
    assert_eq!(crate::lock(&s.state).brakes.hold, Some(first.clone()));
    let snap = s.current();
    assert_eq!(snap.brakes.hold, Some(first.clone()));
    let attention = &snap.runs.iter().find(|r| r.run_id == id).unwrap().attention;
    assert!(attention.iter().any(|l| l == HOLD_TEXT), "{attention:?}");

    let (again, message) = held(&hold(&s, true).await);
    assert_eq!(again, Some(first.clone()), "the first since is kept");
    let since = common::text::hh_mm(first.since);
    assert_eq!(
        message,
        format!("dispatch was already held since {since} UTC")
    );

    let (off, message) = held(&hold(&s, false).await);
    assert_eq!((off, message.as_str()), (None, "dispatch released"));
    assert_eq!(on_file(&data), serde_json::json!({"v": 1, "hold": null}));
    let (off, message) = held(&hold(&s, false).await);
    assert_eq!((off, message.as_str()), (None, "dispatch is not held"));
    assert_eq!(crate::lock(&s.state).brakes.hold, None);
    assert_eq!(
        crate::lock(&s.state).runs[&id].state,
        proto::RunState::Running
    );
    s.stop().await;
    drop(handle);
}

/// Decision 17: a write that fails refuses with the error and changes nothing.
#[tokio::test(flavor = "multi_thread")]
async fn hold_write_failure_refuses_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let (s, _manager) = service(dir.path());
    let handle = s.spawn(CancellationToken::new());
    // The load first, so the read-only directory is the only change.
    s.brakes_ready().await;
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o555)).unwrap();
    // A process that may write anyway (root) cannot test this.
    if std::fs::write(data.join("probe"), b"").is_ok() {
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        s.stop().await;
        return;
    }
    let reply = hold(&s, true).await;
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
    let RunReply::Refused {
        request: action,
        message,
        ..
    } = &reply
    else {
        panic!("not refused: {reply:?}");
    };
    assert_eq!(action, request::HOLD);
    assert!(message.starts_with("the hold was not saved: "), "{message}");
    assert_eq!(crate::lock(&s.state).brakes.hold, None);
    assert!(!data.join(BRAKES_FILE).exists());
    s.stop().await;
    drop(handle);
}

/// Ruling C10: requests racing each other leave the file and the engine agreeing, so
/// a restart never flips the hold.
#[tokio::test(flavor = "multi_thread")]
async fn racing_hold_requests_leave_file_and_engine_agreeing() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (s, _manager) = service(dir.path());
    let handle = s.spawn(CancellationToken::new());
    for round in 0..5 {
        let asks: Vec<_> = (0..8)
            .map(|i| {
                let s = s.clone();
                tokio::spawn(async move { hold(&s, (i + round) % 2 == 0).await })
            })
            .collect();
        for ask in asks {
            assert!(matches!(ask.await.unwrap(), RunReply::Held { .. }));
        }
        let engine = crate::lock(&s.state).brakes.hold.clone();
        let file = super::brakes_io::load(&data, super::brakes_io::BRAKES_IO_WAIT, 0).await;
        assert_eq!(file, engine, "round {round}");
    }
    s.stop().await;
    drop(handle);
}

/// Decision 16: a hold saved by an earlier daemon holds before the first step, and a
/// corrupt file holds too.
#[tokio::test(flavor = "multi_thread")]
async fn a_persisted_or_corrupt_hold_holds_from_the_start() {
    for (text, by) in [
        (r#"{"v":1,"hold":{"since":5,"by":"user"}}"#, "user"),
        ("{", CORRUPT_BY),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join(BRAKES_FILE), text).unwrap();
        let (s, _manager) = service(dir.path());
        let handle = s.spawn(CancellationToken::new());
        let iterate = RunRequest::Iterate {
            run: "nope".into(),
            goal: "more".into(),
            design: None,
        };
        let reply = tokio::time::timeout(STEP, s.request(iterate))
            .await
            .unwrap();
        assert_eq!(
            reply,
            RunReply::refused(request::ITERATE, HOLD_TEXT),
            "{text}"
        );
        let hold = crate::lock(&s.state).brakes.hold.clone().expect("held");
        assert_eq!(hold.by, by);
        s.stop().await;
        drop(handle);
    }
}

/// The plan file `Start` asks for.
const PLAN: &str = "[[task]]\nid = \"t1\"\ntitle = \"t\"\nbrief = \"b\"\nowns = [\"a.txt\"]\nacceptance = [\"a\"]\nsize = \"s\"\n";

/// Decision 19: under the hold, `StartGoal` (new and `--continue`), `Start`, `Promote`
/// and `Iterate` are refused with `HOLD_TEXT`, and no decider, scout or window starts.
#[tokio::test(flavor = "multi_thread")]
async fn hold_refuses_start_goal_with_text() {
    let rig = ChainRig::new(|_| {}).await;
    let work = rig.checkout.work.clone();
    assert!(matches!(hold(&rig.s, true).await, RunReply::Held { .. }));
    let windows = rig.manager.list().len();
    let runs = crate::lock(&rig.s.state).runs.len();
    let goal = |continue_from: Option<String>| RunRequest::StartGoal {
        goal: "Add a logout button".into(),
        dir: work.clone(),
        yes: true,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: Some(DeliveryMode::Local),
        continue_from,
        design: None,
    };
    let cases = [
        (goal(None), request::START_GOAL),
        (goal(Some(PREV.into())), request::START_GOAL),
        (
            RunRequest::Start {
                plan_toml: PLAN.into(),
                dir: work.clone(),
                yes: true,
                trust_project: false,
                unconfined_checks: true,
                delivery: None,
                design: None,
            },
            request::START,
        ),
        (
            RunRequest::Promote {
                run_id: PREV.into(),
                orchestrator: None,
            },
            request::PROMOTE,
        ),
        (
            RunRequest::Iterate {
                run: PREV.into(),
                goal: "more".into(),
                design: None,
            },
            request::ITERATE,
        ),
    ];
    for (req, label) in cases {
        let shown = format!("{req:?}");
        let reply = tokio::time::timeout(ANSWER, rig.s.request(req))
            .await
            .expect("answered");
        assert_eq!(reply, RunReply::refused(label, HOLD_TEXT), "{shown}");
    }
    assert!(!rig.marker.exists(), "a decider was called");
    assert_eq!(rig.manager.list().len(), windows, "a window started");
    assert_eq!(crate::lock(&rig.s.state).runs.len(), runs, "a run was made");
    rig.stop().await;
}

/// Decision 19 and ruling C11: the orchestrator's `start_goal` tool answers the
/// refusal text and the chain is unchanged.
#[tokio::test(flavor = "multi_thread")]
async fn hold_refuses_the_orchestrator_start_goal_tool() {
    let rig = ChainRig::new(|_| {}).await;
    assert!(matches!(hold(&rig.s, true).await, RunReply::Held { .. }));
    let before = format!("{:?}", crate::lock(&rig.s.state).chains.get(CHAIN));
    let reply = rig.s.start_goal_tool(PREV, "next".into()).await;
    assert_eq!(reply, Err(HOLD_TEXT.to_string()));
    let after = format!("{:?}", crate::lock(&rig.s.state).chains.get(CHAIN));
    assert_eq!(before, after);
    assert!(rig.new_runs().is_empty());
    assert!(!rig.marker.exists(), "a decider was called");
    rig.stop().await;
}

/// The goal a test queues.
fn queued_goal(project: &Path, dir: &Path) -> crate::profile::queue::QueuedGoal {
    crate::profile::queue::QueuedGoal {
        id: crate::profile::queue::next_id(1),
        project: project.to_path_buf(),
        dir: dir.to_path_buf(),
        goal: "add a".into(),
        queued_at: 1,
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: None,
        design: None,
    }
}

/// A stored profile for `project`, as `profile use` leaves it.
fn store_profile(data: &Path, project: &Path) {
    let repo_dir = crate::profile::repo_dir(data, project);
    std::fs::create_dir_all(&repo_dir).unwrap();
    let meta = proto::ProfileMeta {
        confirmed_at: 1,
        report: None,
        verification: None,
        fingerprint: std::collections::BTreeMap::new(),
        edited_keys: Vec::new(),
        project: Some(project.to_path_buf()),
    };
    crate::profile::store::save(&repo_dir, &proto::RepoProfile::default(), &meta).unwrap();
}

/// Decision 20 and ruling C9, through the real run service: a goal queued before its
/// profile, drained under the hold, stays queued and nothing is dropped; `hold off`
/// drains it and its start runs past the hold (here to the installed check, since no
/// agent exists; the profile service's tests see the starter called).
#[tokio::test(flavor = "multi_thread")]
async fn queued_goal_stays_queued_under_hold_and_hold_off_drains_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("app");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    for args in [
        &["init", "-q", "-b", "main"][..],
        &["config", "user.name", "t"],
        &["config", "user.email", "t@t"],
        &["add", "-A"],
        &["commit", "-q", "-m", "base"],
    ] {
        crate::run::driver::delivery::tests::git(&root, args);
    }
    let (s, manager) = service(dir.path());
    let data = dir.path().join("data");
    let socket = dir.path().join("d.sock");
    let profiles = crate::profile::service::wire(
        &manager,
        &s,
        &data,
        &socket,
        &config::Orchestrator::default(),
    );
    let shutdown = CancellationToken::new();
    let handle = s.spawn(shutdown.clone());
    let pre = crate::run::git::preflight("git".as_ref(), &root, STEP).unwrap();
    let queued = profiles.queue_goal(queued_goal(&pre.project, &root)).await;
    assert!(matches!(queued, Ok(Some(_))), "{queued:?}");
    assert!(matches!(hold(&s, true).await, RunReply::Held { .. }));
    // The profile is stored and its queue drained, under the hold.
    store_profile(&data, &pre.project);
    // Decision 19: a queued goal's start itself is refused with the text the drain
    // keeps a goal for (ruling C9), whatever the drain's probe saw.
    let direct = s.start_queued_goal(queued_goal(&pre.project, &root)).await;
    assert_eq!(direct, Err(HOLD_TEXT.to_string()));
    profiles.drain_all();
    let deadline = Instant::now() + STEP;
    let status = loop {
        let (queued, dropped) = profiles.queued_for(&pre.project);
        // Listed again once the drain that found the hold has ended.
        if queued.len() == 1 {
            break (queued, dropped);
        }
        assert!(Instant::now() < deadline, "the drain did not end");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert!(status.1.is_empty(), "dropped: {:?}", status.1);

    let (off, _) = held(&hold(&s, false).await);
    assert_eq!(off, None);
    let deadline = Instant::now() + STEP;
    let dropped = loop {
        let (queued, dropped) = profiles.queued_for(&pre.project);
        if queued.is_empty() && !dropped.is_empty() {
            break dropped;
        }
        assert!(Instant::now() < deadline, "the held goal did not start");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0]
            .reason
            .starts_with("the orchestrator's runtime claude is not installed"),
        "{:?}",
        dropped[0].reason
    );
    shutdown.cancel();
    s.stop().await;
    drop(handle);
}
