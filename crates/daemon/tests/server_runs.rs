//! M8a.22 fix round 1 (ruling T22-minors, m1): a run request answered on its own task
//! must not keep a disconnected client's connection open. The request itself carries on.

mod support;

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::{RunContext, RunService};
use daemon::server::{GitWiring, serve};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, RunRequest, read_frame, write_frame};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

/// How long the stand-in `git` takes: far past the test's own bound, so a connection
/// held open by the request is told apart from one that closed.
const GIT_SLEEP_SECS: u64 = 20;
/// The connection must close well before the request's git call can return.
const CLOSE_WITHIN: Duration = Duration::from_secs(5);

/// A plan that parses, so the request reaches its preflight's git call.
const PLAN: &str = "goal = \"x\"\n\n[[task]]\nid = \"t1\"\ntitle = \"T\"\nsize = \"S\"\ntest_mode = \"check\"\ntest_mode_reason = \"r\"\nowns = [\"a.txt\"]\nbrief = \"b\"\nacceptance = [\"a\"]\n";

/// A manager configuration whose agent programs are nonexistent paths: a run these
/// tests start never reaches a real `claude` or `codex` (M9.13: a planned run starts
/// its orchestrator).
fn pinned(socket: std::path::PathBuf) -> ManagerConfig {
    ManagerConfig::for_tests(socket, "/bin/sh".into())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disconnected_client_is_not_held_open_by_its_run_request() {
    let dir = tempfile::Builder::new()
        .prefix("ax-runs")
        .tempdir_in("/tmp")
        .unwrap();
    let slow_git = dir.path().join("git");
    std::fs::write(&slow_git, format!("#!/bin/sh\nsleep {GIT_SLEEP_SECS}\n")).unwrap();
    std::fs::set_permissions(&slow_git, std::fs::Permissions::from_mode(0o755)).unwrap();

    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (manager, _events) = WindowManager::new(pinned(socket.clone()));
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let orchestrator = config::Orchestrator {
        git_timeout_secs: 60,
        ..config::Orchestrator::default()
    };
    let mut ctx = RunContext::new(
        dir.path().join("data"),
        manager.config(),
        orchestrator,
        git.registry.clone(),
    );
    ctx.git = slow_git.into_os_string();
    let runs = RunService::new(manager.clone(), ctx);
    let shutdown = CancellationToken::new();
    runs.spawn(shutdown.clone());
    tokio::spawn(serve(listener, manager, git, runs, shutdown.clone()));

    let stream = UnixStream::connect(&socket).await.unwrap();
    let (mut rd, mut wr) = stream.into_split();
    let hello = ClientMsg::Hello {
        proto_version: PROTO_VERSION,
        client: ClientKind::Cli,
    };
    write_frame(&mut wr, &hello).await.unwrap();
    let welcome = read_frame::<_, DaemonMsg>(&mut rd).await.unwrap();
    assert!(
        matches!(welcome, Some(DaemonMsg::Welcome { .. })),
        "{welcome:?}"
    );
    let start = ClientMsg::Run(RunRequest::Start {
        plan_toml: PLAN.into(),
        dir: dir.path().to_path_buf(),
        yes: true,
        trust_project: false,
        // Final fix batch F1c round 2: Linux cannot confine checks.
        unconfined_checks: !cfg!(target_os = "macos"),
        delivery: None,
    });
    write_frame(&mut wr, &start).await.unwrap();
    wr.shutdown().await.unwrap();

    let started = Instant::now();
    let closed = tokio::time::timeout(CLOSE_WITHIN, async {
        loop {
            match read_frame::<_, DaemonMsg>(&mut rd).await {
                Ok(Some(DaemonMsg::Run(reply))) => panic!("the request answered: {reply:?}"),
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => return,
            }
        }
    })
    .await;
    assert!(
        closed.is_ok(),
        "the connection stayed open {:?} after the client left",
        started.elapsed()
    );
    shutdown.cancel();
}

/// Milestone 8b task 2 added four requests refused `not available yet`; since M8b.17
/// every one is answered by its own handler. `run stats` outside a repository is
/// refused with the reason. (`run promote` is an engine event: this service's loop is
/// not spawned, so it is left to `engine/tests/fast_path.rs`.)
#[tokio::test(flavor = "multi_thread")]
async fn every_milestone_8b_request_is_answered_by_its_task() {
    use proto::run_wire::{ProfileReply, ProfileRequest, request};
    let dir = tempfile::Builder::new()
        .prefix("ax-runs8b")
        .tempdir_in("/tmp")
        .unwrap();
    let (manager, _events) = WindowManager::new(pinned(dir.path().join("d.sock")));
    let git = GitWiring::new(config::Git::default());
    let ctx = RunContext::new(
        dir.path().join("data"),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager, ctx);
    let here = dir.path().to_path_buf();
    // M8b.17 review, m8: each request's own answer, exactly (this service's profile
    // side is not wired, so the adaptation requests say so).
    let not_running = "the profile service is not running".to_string();
    assert_eq!(
        runs.request(RunRequest::StartGoal {
            goal: "g".into(),
            dir: here.clone(),
            yes: true,
            trust_project: false,
            unconfined_checks: false,
            orchestrator: None,
            delivery: None,
            continue_from: None,
        })
        .await,
        proto::RunReply::Refused {
            request: request::START_GOAL.to_string(),
            message: not_running.clone(),
            request_id: None,
        }
    );
    assert_eq!(
        runs.request(RunRequest::Profile(ProfileRequest::Status {
            dir: here.clone()
        }))
        .await,
        proto::RunReply::profile(ProfileReply::Refused {
            message: not_running
        })
    );
    assert_eq!(
        runs.request(RunRequest::Stats {
            dir: here.clone(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: false,
        })
        .await,
        proto::RunReply::Refused {
            request: request::STATS.to_string(),
            message: format!("not a git repository: {}", here.display()),
            request_id: None,
        }
    );
}

/// Milestone 9.5 task 2: until their tasks land, `McpReady` is answered `Done` and
/// changes nothing (task M9.5.5a), and `Stats`' `apply`, `dismiss` and `read_only` are
/// ignored (task M9.5.11).
#[tokio::test(flavor = "multi_thread")]
async fn mcp_ready_is_answered_and_new_stats_fields_are_ignored() {
    use proto::run_wire::request;
    let dir = tempfile::Builder::new()
        .prefix("ax-runs95")
        .tempdir_in("/tmp")
        .unwrap();
    let (manager, _events) = WindowManager::new(pinned(dir.path().join("d.sock")));
    let git = GitWiring::new(config::Git::default());
    let ctx = RunContext::new(
        dir.path().join("data"),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager, ctx);
    let before = runs.request(RunRequest::List).await;
    let ready = RunRequest::McpReady {
        run_id: "r-3f9a".into(),
        window_id: 4,
    };
    assert_eq!(
        runs.request(ready).await,
        proto::RunReply::done(request::MCP_READY, "")
    );
    assert_eq!(runs.request(RunRequest::List).await, before);
    let here = dir.path().to_path_buf();
    assert_eq!(
        runs.request(RunRequest::Stats {
            dir: here.clone(),
            apply: vec!["threshold-s".into()],
            dismiss: vec!["route-m".into()],
            read_only: true,
        })
        .await,
        proto::RunReply::Refused {
            request: request::STATS.to_string(),
            message: format!("not a git repository: {}", here.display()),
            request_id: None,
        }
    );
}

/// M9.2 review ruling 8, updated by M9.7: `ApproveHold` and `RejectHold` reach the
/// engine, which keeps approval holds, and are answered under their own labels.
#[tokio::test(flavor = "multi_thread")]
async fn approval_holds_are_answered_by_the_engine() {
    use proto::run_wire::request;
    let dir = tempfile::Builder::new()
        .prefix("ax-runs9h")
        .tempdir_in("/tmp")
        .unwrap();
    let (manager, _events) = WindowManager::new(pinned(dir.path().join("d.sock")));
    let git = GitWiring::new(config::Git::default());
    let ctx = RunContext::new(
        dir.path().join("data"),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager, ctx);
    let shutdown = CancellationToken::new();
    runs.spawn(shutdown.clone());
    let (run_id, hold) = ("r1".to_string(), "h1".to_string());
    for (req, label) in [
        (
            RunRequest::ApproveHold {
                run_id: run_id.clone(),
                hold: hold.clone(),
            },
            request::APPROVE,
        ),
        (RunRequest::RejectHold { run_id, hold }, request::REJECT),
    ] {
        let reply = tokio::time::timeout(Duration::from_secs(30), runs.request(req))
            .await
            .expect("the engine answers within 30 s");
        assert_eq!(reply, proto::RunReply::refused(label, "unknown run r1"));
    }
    shutdown.cancel();
}

/// A daemon on `dir/d.sock` with its run service, and the milestone-8b profile and
/// decider services too when `adaptation` is set (deciders off, so nothing is spawned).
/// Both agent programs are `dir/agent`, a stand-in that exits at once, so a planned
/// run's start finds its orchestrator's runtime installed (decision 26's start check)
/// and its window runs only the stand-in. Returns a connected, welcomed client.
async fn tagged_rig(
    dir: &std::path::Path,
    adaptation: bool,
    shutdown: &CancellationToken,
) -> (OwnedReadHalf, OwnedWriteHalf) {
    let agent = dir.join("agent");
    std::fs::write(&agent, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
    rig_on(
        dir,
        (adaptation, proto::DeciderMode::Off),
        Some(&agent),
        shutdown,
    )
    .await
}

/// [`rig_on`] with the pinned, nonexistent agent programs.
async fn rig_with(
    dir: &std::path::Path,
    mode: (bool, proto::DeciderMode),
    shutdown: &CancellationToken,
) -> (OwnedReadHalf, OwnedWriteHalf) {
    rig_on(dir, mode, None, shutdown).await
}

/// [`tagged_rig`] with the deciders in `mode`, and `agent` as both agent programs when
/// given. The decider program is pinned to a path that does not exist, so a call falls
/// back at once; the CLI caps are the shipped ones (a Codex session cannot exclude a
/// repository's `.codex` settings).
async fn rig_on(
    dir: &std::path::Path,
    (adaptation, mode): (bool, proto::DeciderMode),
    agent: Option<&std::path::Path>,
    shutdown: &CancellationToken,
) -> (OwnedReadHalf, OwnedWriteHalf) {
    let socket = dir.join("d.sock");
    let data = dir.join("data");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let mut config = pinned(socket.clone());
    if let Some(agent) = agent {
        config.claude_bin = agent.display().to_string();
        config.codex_bin = agent.display().to_string();
    }
    if mode != proto::DeciderMode::Off {
        config.cli_caps = daemon::headless::argv::CLI_CAPS;
    }
    let (manager, _events) = WindowManager::new(config);
    let git = GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let mut orchestrator = config::Orchestrator::default();
    orchestrator.deciders.mode = mode;
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        orchestrator.clone(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    if adaptation {
        daemon::profile::service::wire(&manager, &runs, &data, &socket, &orchestrator);
    }
    runs.spawn(shutdown.clone());
    tokio::spawn(serve(listener, manager, git, runs, shutdown.clone()));

    let stream = UnixStream::connect(&socket).await.unwrap();
    let (mut rd, mut wr) = stream.into_split();
    let hello = ClientMsg::Hello {
        proto_version: PROTO_VERSION,
        client: ClientKind::Cli,
    };
    write_frame(&mut wr, &hello).await.unwrap();
    let welcome = read_frame::<_, DaemonMsg>(&mut rd).await.unwrap();
    assert!(
        matches!(welcome, Some(DaemonMsg::Welcome { .. })),
        "{welcome:?}"
    );
    (rd, wr)
}

/// The next run reply, skipping broadcasts, within a deadline.
async fn next_run_reply(rd: &mut OwnedReadHalf) -> proto::RunReply {
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            match read_frame::<_, DaemonMsg>(rd).await {
                Ok(Some(DaemonMsg::Run(reply))) => return reply,
                Ok(Some(_)) => continue,
                other => panic!("the connection ended: {other:?}"),
            }
        }
    })
    .await
    .expect("the request is answered")
}

/// Milestone 9 task 2 (decision 2): a `ClientMsg::RunTagged` request is answered like
/// `ClientMsg::Run`, and its `Done` or `Refused` reply echoes the id; an untagged
/// request's reply carries none.
#[tokio::test(flavor = "multi_thread")]
async fn a_tagged_run_request_is_answered_with_its_id() {
    use proto::RunReply;
    let dir = tempfile::Builder::new()
        .prefix("ax-runs9")
        .tempdir_in("/tmp")
        .unwrap();
    let shutdown = CancellationToken::new();
    let (mut rd, mut wr) = tagged_rig(dir.path(), false, &shutdown).await;
    // `run stats` outside a repository is refused at once, whoever asks.
    let stats = RunRequest::Stats {
        dir: dir.path().to_path_buf(),
        apply: Vec::new(),
        dismiss: Vec::new(),
        read_only: false,
    };
    let tagged = ClientMsg::RunTagged {
        id: 7,
        request: stats.clone(),
    };
    for (msg, expected) in [(tagged, Some(7)), (ClientMsg::Run(stats), None)] {
        write_frame(&mut wr, &msg).await.unwrap();
        let reply = next_run_reply(&mut rd).await;
        let RunReply::Refused { request_id, .. } = reply else {
            panic!("refused: {reply:?}");
        };
        assert_eq!(request_id, expected);
    }
    shutdown.cancel();
}

/// M9.2 review ruling 1: a tagged request's success reply carries its id too. A goal
/// start in a repository with a stored profile and the deciders off takes triage's
/// fallback (the plan path); since M9.13 that path builds a planned run, so it is
/// answered `Triaged` with the run's id (its orchestrator's program is a stand-in that
/// exits at once).
#[tokio::test(flavor = "multi_thread")]
async fn a_tagged_goal_start_is_triaged_with_its_id() {
    use proto::RunReply;
    let repo = support::run_git::repo();
    let dir = tempfile::Builder::new()
        .prefix("ax-runs9g")
        .tempdir_in("/tmp")
        .unwrap();
    let meta = proto::ProfileMeta {
        confirmed_at: 1_700_000_000,
        report: None,
        verification: None,
        fingerprint: Default::default(),
        edited_keys: Vec::new(),
        project: None,
    };
    let repo_dir = daemon::profile::repo_dir(&dir.path().join("data"), &repo.root);
    daemon::profile::store::save(&repo_dir, &proto::RepoProfile::default(), &meta).unwrap();
    let shutdown = CancellationToken::new();
    let (mut rd, mut wr) = tagged_rig(dir.path(), true, &shutdown).await;
    let msg = ClientMsg::RunTagged {
        id: 11,
        request: RunRequest::StartGoal {
            goal: "rework storage".into(),
            dir: repo.root.clone(),
            yes: true,
            trust_project: false,
            unconfined_checks: true,
            orchestrator: None,
            delivery: None,
            continue_from: None,
        },
    };
    write_frame(&mut wr, &msg).await.unwrap();
    let reply = next_run_reply(&mut rd).await;
    let RunReply::Triaged {
        run_id, request_id, ..
    } = reply
    else {
        panic!("triaged: {reply:?}");
    };
    assert!(run_id.is_some(), "a planned run");
    assert_eq!(request_id, Some(11));
    shutdown.cancel();
}

/// Milestone 9 decision 43: pre-run triage's decider session leaves its record in the
/// repository's history (anthrex's data directory, never the repository) even when no
/// run is created. The decider program does not exist, so the call falls back; the
/// goal's Codex orchestrator is then refused after triage, because its program does
/// not exist either (decision 26's start check, M9.17 fix round).
#[tokio::test(flavor = "multi_thread")]
async fn pre_run_triage_writes_a_record_even_when_no_run_is_created() {
    use proto::{AgentRole, HistoryLine, RoleOutcome, RunReply};
    let repo = support::run_git::repo();
    support::run_git::commit_file(&repo.root, "src/lib.rs", "// lib\n", "lib");
    support::run_git::commit_file(&repo.root, ".codex/config.toml", "model = \"x\"\n", "codex");
    let dir = tempfile::Builder::new()
        .prefix("ax-runs9t")
        .tempdir_in("/tmp")
        .unwrap();
    let meta = proto::ProfileMeta {
        confirmed_at: 1_700_000_000,
        report: None,
        verification: None,
        fingerprint: Default::default(),
        edited_keys: Vec::new(),
        project: None,
    };
    let data = dir.path().join("data");
    let repo_dir = daemon::profile::repo_dir(&data, &repo.root);
    let profile = proto::RepoProfile {
        languages: vec!["rust".into()],
        ..proto::RepoProfile::default()
    };
    daemon::profile::store::save(&repo_dir, &profile, &meta).unwrap();
    let shutdown = CancellationToken::new();
    let mode = (true, proto::DeciderMode::Claude);
    let (mut rd, mut wr) = rig_with(dir.path(), mode, &shutdown).await;
    let goal = "rework storage";
    let msg = ClientMsg::Run(RunRequest::StartGoal {
        goal: goal.into(),
        dir: repo.root.clone(),
        yes: true,
        trust_project: false,
        unconfined_checks: true,
        orchestrator: Some(proto::OrchestratorChoice {
            runtime: proto::Runtime::Codex,
            model: None,
        }),
        delivery: None,
        continue_from: None,
    });
    write_frame(&mut wr, &msg).await.unwrap();
    let reply = next_run_reply(&mut rd).await;
    let RunReply::Refused { message, .. } = &reply else {
        panic!("refused: {reply:?}");
    };
    assert!(
        message.contains("the orchestrator's runtime codex is not installed"),
        "{message}"
    );
    shutdown.cancel();

    let path = repo_dir.join("history.jsonl");
    let (lines, problems) = daemon::run::history_io::read_history(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines.len(), 1, "{lines:#?}");
    let HistoryLine::RoleRoute(d) = &lines[0] else {
        panic!("a role_route line: {lines:#?}");
    };
    assert!(d.record_id.starts_with("triage/"), "{}", d.record_id);
    assert_eq!((d.run_id.as_deref(), d.task_id.as_deref()), (None, None));
    assert_eq!((d.role, d.trigger.as_str()), (AgentRole::Decider, "triage"));
    assert_eq!(d.source, "decider_config");
    assert_eq!(d.outcome, Some(RoleOutcome::Fallback));
    let reason = d.result.as_deref().unwrap_or_default();
    assert!(
        reason.starts_with("the decider could not start"),
        "{reason}"
    );
    assert_eq!(d.input.goal.as_deref(), Some(goal));
    assert_eq!(d.input.languages, vec!["rust".to_string()]);
    assert_eq!(d.input.question_kind.as_deref(), Some("triage"));
    assert_eq!(d.candidates[d.selected_index as usize].route, d.chosen);
    // Review I-2: the full ordered snapshot: the mode's runtime's roster entries at or
    // above `[orchestrator.deciders] strength`, lowest first; no peer runtime.
    let config = config::Orchestrator::default();
    let mut ladder: Vec<&proto::ModelEntry> = config
        .models
        .iter()
        .filter(|e| e.runtime == proto::Runtime::Claude && e.strength >= config.deciders.strength)
        .collect();
    ladder.sort_by_key(|e| e.strength);
    assert!(ladder.len() > 1, "the default roster has a ladder");
    let models: Vec<&str> = d
        .candidates
        .iter()
        .map(|c| c.route.model.as_str())
        .collect();
    let want: Vec<&str> = ladder.iter().map(|e| e.model.as_str()).collect();
    assert_eq!(models, want, "{:#?}", d.candidates);
    for (i, c) in d.candidates.iter().enumerate() {
        let reason = c.skipped_reason.as_deref();
        match i.cmp(&(d.selected_index as usize)) {
            std::cmp::Ordering::Equal => assert_eq!(reason, None),
            std::cmp::Ordering::Greater => {
                assert_eq!(reason, Some("an earlier candidate was taken"))
            }
            std::cmp::Ordering::Less => assert_eq!(reason, Some("not in the configured list")),
        }
    }
    // Nothing reached the repository.
    let status = support::run_git::out(&repo.root, &["status", "--porcelain", "--ignored"]);
    assert!(status.trim().is_empty(), "{status}");
}
