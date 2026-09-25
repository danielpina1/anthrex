//! M8a.22 fix round 1 (ruling T22-minors, m1): a run request answered on its own task
//! must not keep a disconnected client's connection open. The request itself carries on.

use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::{RunContext, RunService};
use daemon::server::{GitWiring, serve};
use proto::{ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, RunRequest, read_frame, write_frame};
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use tokio_util::sync::CancellationToken;

/// How long the stand-in `git` takes: far past the test's own bound, so a connection
/// held open by the request is told apart from one that closed.
const GIT_SLEEP_SECS: u64 = 20;
/// The connection must close well before the request's git call can return.
const CLOSE_WITHIN: Duration = Duration::from_secs(5);

/// A plan that parses, so the request reaches its preflight's git call.
const PLAN: &str = "goal = \"x\"\n\n[[task]]\nid = \"t1\"\ntitle = \"T\"\nsize = \"S\"\ntest_mode = \"check\"\ntest_mode_reason = \"r\"\nowns = [\"a.txt\"]\nbrief = \"b\"\nacceptance = [\"a\"]\n";

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
    let (manager, _events) =
        WindowManager::new(ManagerConfig::new(socket.clone(), "/bin/sh".into()));
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
