//! Task M9.2.12: preflight in `run start --plan` (after M8a's git preflight, before the
//! run is built) and `run start --goal` (before triage), each resolving the mode once; a
//! refusal leaves nothing behind; a `local` run calls no host. `FakeHost` throughout,
//! and every agent and decider binary a stand-in or a path that does not exist.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{DeliveryMode, RunReply, RunRequest};

use super::super::super::host_ops::tests::{NoRoots, service_with};
use super::{Rig, git};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::{RunContext, RunService};
use crate::run::test_support::{PROFILE, plan_with, task_toml};

const LOGGED_OUT: &str =
    "gh is not logged in to github.com; run gh auth login, or use --delivery local";

fn plan() -> String {
    plan_with(PROFILE, &[task_toml("t1", "S", "[\"crates/a/**\"]", "")])
}

fn start(rig: &Rig, yes: bool, delivery: Option<DeliveryMode>) -> RunRequest {
    RunRequest::Start {
        plan_toml: plan(),
        dir: rig.work.clone(),
        yes,
        trust_project: false,
        unconfined_checks: true,
        delivery,
    }
}

/// The branches and private refs of every run in `dir`.
fn run_refs(dir: &Path) -> String {
    git(
        dir,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads/anthrex/",
            "refs/anthrex/",
        ],
    )
}

/// `s`'s answer to `req`, within a deadline: a start that wrongly reaches the (here
/// unspawned) engine fails the test instead of hanging it.
async fn ask(s: &RunService, req: RunRequest) -> RunReply {
    match tokio::time::timeout(Duration::from_secs(60), s.request(req)).await {
        Ok(reply) => reply,
        Err(_) => panic!("no answer within 60 s"),
    }
}

fn gh_calls(rig: &Rig, verb: &[&str]) -> usize {
    let calls = rig.ctl.calls();
    calls
        .iter()
        .filter(|c| c.starts_with(&verb.iter().map(|v| v.to_string()).collect::<Vec<_>>()))
        .count()
}

#[tokio::test]
async fn preflight_refusal_leaves_nothing_behind() {
    let rig = Rig::new(true);
    rig.ctl.create_repo("fake", "app", &rig.bare, "main");
    let data = rig.tmp.path().join("data");
    let s = service_with(rig.host(), &data);
    let reply = ask(&s, start(&rig, true, Some(DeliveryMode::Pr))).await;
    assert_eq!(reply, RunReply::refused(request::START, LOGGED_OUT));
    assert!(s.current().runs.is_empty(), "no run");
    let runs = data.join("runs");
    assert!(
        !runs.exists() || std::fs::read_dir(&runs).unwrap().next().is_none(),
        "no run directory"
    );
    assert_eq!(run_refs(&rig.work), "", "no branch, no private ref");
    assert_eq!(
        git(&rig.bare, &["for-each-ref", "--format=%(refname)"]),
        "refs/heads/main",
        "nothing on the remote"
    );
    // It stopped at the failing check: nothing past `gh auth status` was asked.
    assert_eq!(gh_calls(&rig, &["auth", "status"]), 1);
    assert_eq!(gh_calls(&rig, &["repo", "view"]), 0);
}

/// `run start --plan --delivery pr` that passes preflight freezes the mode, the
/// repository and the seal into the run, watching from the start.
#[tokio::test]
async fn a_pr_run_freezes_its_delivery_at_start() {
    let rig = Rig::ready(true);
    let s = service_with(rig.host(), &rig.tmp.path().join("data"));
    let plan = crate::run::plan::parse_plan(&plan()).unwrap();
    let flags = (false, false, true);
    let asked = super::DeliveryStart::Resolve(Some(DeliveryMode::Pr));
    let shape = super::super::super::build::Shape::PlanFile;
    let built = s.build_delivered(plan, rig.work.clone(), flags, shape, asked);
    let run: crate::run::model::Run = match built.await {
        Ok(run) => run,
        Err(error) => panic!("{}", error.text()),
    };
    let d = &run.delivery;
    assert_eq!(d.mode, DeliveryMode::Pr);
    assert_eq!(
        d.repo.as_ref().map(|r| r.full()),
        Some("fake/app".to_string())
    );
    assert_eq!(
        d.remote_seal,
        Some(rig.host().remote_seal(&rig.work, "origin").unwrap())
    );
    assert!(d.watching);
    assert_eq!(d.poll_base_secs, d.limits.poll_secs);
    assert_eq!(gh_calls(&rig, &["auth", "status"]), 1, "one preflight");
}

#[tokio::test(flavor = "multi_thread")]
async fn local_mode_calls_no_host() {
    for asked in [None, Some(DeliveryMode::Local)] {
        let rig = Rig::ready(true);
        let data = rig.tmp.path().join("data");
        let s = service_with(rig.host(), &data);
        let handle = s.spawn(tokio_util::sync::CancellationToken::new());
        let reply = ask(&s, start(&rig, false, asked)).await;
        let RunReply::Started { run_id, .. } = reply else {
            panic!("{asked:?}: {reply:?}");
        };
        let delivery = crate::lock(&s.state).runs[&run_id].delivery.clone();
        assert_eq!(delivery.mode, DeliveryMode::Local);
        assert_eq!((delivery.repo, delivery.remote_seal), (None, None));
        assert!(!delivery.watching);
        // The run does its git work; still no host call.
        let approved = ask(
            &s,
            RunRequest::Approve {
                run_id: run_id.clone(),
            },
        )
        .await;
        assert!(matches!(approved, RunReply::Done { .. }), "{approved:?}");
        let branch = format!("refs/heads/anthrex/{run_id}/integration");
        let deadline = Instant::now() + Duration::from_secs(30);
        while !run_refs(&rig.work).contains(&branch) {
            assert!(Instant::now() < deadline, "the run branch was never made");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(
            !rig.ctl.dir().join("calls.jsonl").exists(),
            "{asked:?}: a local run called the host: {:?}",
            rig.ctl.calls()
        );
        s.stop().await;
        handle.abort();
    }
}

/// A decider stand-in that records each call in `marker` and fails (the triage falls
/// back): never a real agent. Fix round 1, m3: it is written under another name, closed
/// and renamed into place, then executed once (a guard makes that run exit at once),
/// retrying while a concurrent fork still holds a writable copy of its descriptor
/// (`ETXTBSY`), so the daemon's exec of it is never the first and never busy.
pub(in crate::run::driver) fn decider_stand_in(dir: &Path) -> (String, std::path::PathBuf) {
    let marker = dir.join("decider-called");
    let script = dir.join("decider.sh");
    let staged = dir.join("decider.sh.new");
    std::fs::write(
        &staged,
        format!(
            "#!/bin/sh\n[ -n \"$ANTHREX_TEST_WARM_UP\" ] && exit 0\necho called >> '{}'\ncat > /dev/null\nexit 1\n",
            marker.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&staged, &script).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut child = loop {
        match std::process::Command::new(&script)
            .env("ANTHREX_TEST_WARM_UP", "1")
            .stdin(std::process::Stdio::null())
            .spawn()
        {
            Ok(child) => break child,
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                assert!(Instant::now() < deadline, "the stand-in stayed busy: {e}");
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => panic!("the stand-in did not start: {e}"),
        }
    };
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "the stand-in's warm-up never exited"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success() && !marker.exists(), "warm-up: {status:?}");
    (script.display().to_string(), marker)
}

/// A spawned service over `rig` whose decider is the stand-in (its marker returned),
/// with a stored profile, so a goal reaches triage once preflight passes.
fn goal_service(
    rig: &Rig,
) -> (
    Arc<RunService>,
    tokio::task::JoinHandle<()>,
    std::path::PathBuf,
) {
    let tmp = rig.tmp.path();
    let (decider, marker) = decider_stand_in(tmp);
    let data = tmp.join("data");
    let socket = tmp.join("d.sock");
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    config.decider_bin = Some(decider);
    config.worktrees_root = tmp.join("worktrees");
    let (manager, _events) = WindowManager::new(config);
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        config::Orchestrator::default(),
        Arc::new(NoRoots),
    )
    .with_host(rig.host());
    let s = RunService::new(manager.clone(), ctx);
    let handle = s.spawn(tokio_util::sync::CancellationToken::new());
    crate::profile::service::wire(
        &manager,
        &s,
        &data,
        &socket,
        &config::Orchestrator::default(),
    );
    let pre =
        crate::run::git::preflight("git".as_ref(), &rig.work, Duration::from_secs(30)).unwrap();
    let repo_dir = crate::profile::repo_dir(&data, &pre.project);
    std::fs::create_dir_all(&repo_dir).unwrap();
    let meta = proto::ProfileMeta {
        confirmed_at: 1,
        report: None,
        verification: None,
        fingerprint: BTreeMap::new(),
        edited_keys: Vec::new(),
        project: Some(pre.project.clone()),
    };
    crate::profile::store::save(&repo_dir, &proto::RepoProfile::default(), &meta).unwrap();
    (s, handle, marker)
}

/// A `pr` goal on `rig`'s checkout, continuing `continue_from` when it is `Some`.
fn pr_goal(rig: &Rig, continue_from: Option<&str>) -> RunRequest {
    RunRequest::StartGoal {
        goal: "Add a feature to crates/a".into(),
        dir: rig.work.clone(),
        yes: true,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: None,
        delivery: Some(DeliveryMode::Pr),
        continue_from: continue_from.map(String::from),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn preflight_runs_before_triage_for_a_goal() {
    let rig = Rig::new(true);
    rig.ctl.create_repo("fake", "app", &rig.bare, "main");
    let (s, handle, marker) = goal_service(&rig);
    let goal = || pr_goal(&rig, None);

    // Refused: no decider was called, and nothing is left behind.
    let reply = ask(&s, goal()).await;
    assert_eq!(reply, RunReply::refused(request::START_GOAL, LOGGED_OUT));
    assert!(!marker.exists(), "triage ran before preflight");
    assert_eq!(run_refs(&rig.work), "");
    assert!(s.current().runs.is_empty());

    // Logged in, preflight passes and triage runs (its stand-in fails, so it falls
    // back); preflight ran once for the whole start, the fast or planned build after it
    // reusing its answer.
    rig.ctl.log_in("github.com");
    let reply = ask(&s, goal()).await;
    assert!(
        marker.exists(),
        "triage after a passing preflight: {reply:?}"
    );
    assert_eq!(
        gh_calls(&rig, &["auth", "status"]),
        2,
        "one preflight per start"
    );
    s.stop().await;
    handle.abort();
}

/// Milestone 9.3 (task M9.3.2 fix round 1, I1; task 6b): a goal continuing a run is
/// routed to `chain_goal.rs`, never started as a new run, and one naming a run that
/// does not exist is refused by the chain lookup, before preflight: no decider call, no
/// run, no ref, no host call. The same goal without `continue_from` reaches triage on
/// this rig. (Changed expectation: task 2's placeholder refusal `continuing an
/// orchestrator is not available yet` is gone.)
#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goal_of_an_unknown_run_is_refused_before_preflight() {
    let rig = Rig::new(true);
    rig.ctl.create_repo("fake", "app", &rig.bare, "main");
    rig.ctl.log_in("github.com");
    let (s, handle, marker) = goal_service(&rig);

    let reply = ask(&s, pr_goal(&rig, Some("run-x"))).await;
    assert_eq!(
        reply,
        RunReply::refused(request::START_GOAL, "unknown run run-x")
    );
    assert!(!marker.exists(), "a continued goal reached triage");
    assert!(s.current().runs.is_empty());
    assert_eq!(run_refs(&rig.work), "");
    assert_eq!(gh_calls(&rig, &["auth", "status"]), 0, "no preflight");

    let reply = ask(&s, pr_goal(&rig, None)).await;
    assert!(marker.exists(), "a new goal reaches triage: {reply:?}");
    s.stop().await;
    handle.abort();
}

/// Fix round 1, m7: `run deliver` and `run watch` reach the engine as themselves, each
/// answered under its own label.
#[tokio::test(flavor = "multi_thread")]
async fn deliver_and_watch_are_routed_to_the_engine() {
    use crate::run::test_support::run_ok;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let s = service_with(super::super::super::host_ops::tests::real_host(), &data);
    let handle = s.spawn(tokio_util::sync::CancellationToken::new());
    let (mut local, mut pr) = (run_ok(&plan()), run_ok(&plan()));
    pr.id = format!("{}p", pr.id);
    pr.delivery.mode = DeliveryMode::Pr;
    pr.delivery.watching = true;
    for run in [&mut local, &mut pr] {
        run.data_dir = data.join("runs").join(&run.id);
        crate::lock(&s.state)
            .runs
            .insert(run.id.clone(), run.clone());
    }
    let deliver = |run_id: &str| RunRequest::Deliver {
        run_id: run_id.into(),
        stage: 1,
    };
    let watch = |run_id: &str, on| RunRequest::Watch {
        run_id: run_id.into(),
        on,
    };
    let locally = format!(
        "run {} delivers locally; run deliver and run watch apply to pr mode",
        local.id
    );
    assert_eq!(
        ask(&s, deliver(&local.id)).await,
        RunReply::refused(request::DELIVER, &locally)
    );
    assert_eq!(
        ask(&s, watch(&local.id, false)).await,
        RunReply::refused(request::WATCH, &locally)
    );
    // In pr mode the two differ: watch --off stops the polling.
    let stopped = ask(&s, watch(&pr.id, false)).await;
    let RunReply::Done {
        request, message, ..
    } = &stopped
    else {
        panic!("{stopped:?}");
    };
    assert_eq!(request, request::WATCH);
    assert!(message.starts_with("stopped watching run "), "{message}");
    assert!(!crate::lock(&s.state).runs[&pr.id].delivery.watching);
    // `run deliver` alone needs a running run (watch does not).
    assert_eq!(
        ask(&s, deliver(&pr.id)).await,
        RunReply::refused(
            request::DELIVER,
            format!("run {} is awaiting_approval", pr.id)
        )
    );
    s.stop().await;
    handle.abort();
}
