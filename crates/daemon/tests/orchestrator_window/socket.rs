//! Milestone 9 decisions 11, 11a and 12 over a real socket: what a client may do to the
//! orchestrator's window while its run is live and after, a placeholder headless window,
//! a client's input time, and `StopFailure`.

use super::support::{Client, TestDaemon, start_daemon_configured};
use super::*;
use daemon::manager::orchestrator_refusal;
use proto::{ClientMsg, DaemonMsg, PROTO_VERSION};

/// Decision 11a: a headless record whose `run` does not parse comes back as an exited
/// headless window nothing drives: `Subscribe`, `Input` and `Restart` are refused;
/// `Kill` and `Remove` work.
#[tokio::test]
async fn unparseable_headless_record_restores_as_an_exited_headless_window() {
    let d = start_daemon_configured(false, |_| {}).await;
    let dir = tempfile::tempdir().unwrap();
    let id = 41;
    d.manager.restore(daemon::state::StateFile {
        version: daemon::state::STATE_VERSION,
        next_id: 42,
        windows: vec![WindowRecord {
            id,
            name: "broken".into(),
            runtime: Runtime::Claude,
            cwd: dir.path().into(),
            project: None,
            worktree: None,
            managed: None,
            model: None,
            initial_prompt: None,
            session_id: Some("s-1".into()),
            created_at: 1,
            status: Status::Exited,
            run: Some(json!({"runtime": "not a runtime"})),
            kind: WindowKind::Headless,
        }],
        runs: Vec::new(),
    });
    let window = find(&d.manager, id);
    assert_eq!(window.kind, WindowKind::Headless);
    assert_eq!(window.status, Status::Exited);
    assert_eq!(window.run, None);
    assert_eq!(d.manager.child_pid(id).unwrap(), None);

    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    for (msg, request) in [
        (
            ClientMsg::Subscribe {
                window_id: id,
                cols: 80,
                rows: 24,
            },
            "subscribe",
        ),
        (
            ClientMsg::Input {
                window_id: id,
                bytes: b"echo hi\r".to_vec(),
            },
            "input",
        ),
        (ClientMsg::Restart { window_id: id }, "restart"),
    ] {
        client.send(msg).await;
        let reply = client
            .recv_until(|m| matches!(m, DaemonMsg::Error { .. } | DaemonMsg::Ack { .. }))
            .await;
        assert!(
            matches!(&reply, DaemonMsg::Error { request: r, .. } if r == request),
            "{request}: {reply:?}"
        );
    }
    client.send(ClientMsg::Kill { window_id: id }).await;
    let reply = client
        .recv_until(|m| matches!(m, DaemonMsg::Error { .. } | DaemonMsg::Ack { .. }))
        .await;
    assert_eq!(
        reply,
        DaemonMsg::Ack {
            request: "kill".into()
        }
    );
    client
        .send(ClientMsg::Remove {
            window_id: id,
            remove_worktree: false,
            force: false,
        })
        .await;
    let reply = client
        .recv_until(|m| matches!(m, DaemonMsg::Error { .. } | DaemonMsg::Ack { .. }))
        .await;
    assert_eq!(
        reply,
        DaemonMsg::Ack {
            request: "remove".into()
        }
    );
    assert!(!d.manager.list().iter().any(|w| w.id == id));
}

async fn reply(client: &mut Client) -> DaemonMsg {
    client
        .recv_until(|m| matches!(m, DaemonMsg::Error { .. } | DaemonMsg::Ack { .. }))
        .await
}

async fn socket_run_window(dir: &Path) -> (TestDaemon, u32) {
    let claude = stand_in(dir);
    let d =
        start_daemon_configured(false, |c| c.claude_bin = claude.to_str().unwrap().into()).await;
    let info = run_window(&d.manager, dir).await;
    (d, info.id)
}

/// Decision 11: while its run is live, the orchestrator cannot be killed or removed by a
/// client; it can be viewed, typed into and restarted.
#[tokio::test]
async fn kill_and_remove_are_refused_while_the_run_is_live() {
    let dir = tempfile::tempdir().unwrap();
    let (d, id) = socket_run_window(dir.path()).await;
    d.manager.set_run_window_live(id, true);
    assert_eq!(d.manager.run_window_live(id), Some(role().run_ref));
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    let message = orchestrator_refusal(id, RUN);
    assert_eq!(
        message,
        format!(
            "window {id} is the orchestrator of run {RUN}; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart {id}"
        )
    );
    client.send(ClientMsg::Kill { window_id: id }).await;
    assert_eq!(
        reply(&mut client).await,
        DaemonMsg::Error {
            request: "kill".into(),
            message: message.clone()
        }
    );
    for remove_worktree in [false, true] {
        client
            .send(ClientMsg::Remove {
                window_id: id,
                remove_worktree,
                force: false,
            })
            .await;
        assert_eq!(
            reply(&mut client).await,
            DaemonMsg::Error {
                request: "remove".into(),
                message: message.clone()
            }
        );
    }
    assert!(d.manager.list().iter().any(|w| w.id == id));
    assert!(d.manager.child_pid(id).unwrap().is_some());
    client
        .send(ClientMsg::Subscribe {
            window_id: id,
            cols: 100,
            rows: 30,
        })
        .await;
    let snapshot = client
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. } | DaemonMsg::Error { .. }))
        .await;
    assert!(
        matches!(
            snapshot,
            DaemonMsg::Snapshot {
                cols: 100,
                rows: 30,
                ..
            }
        ),
        "{snapshot:?}"
    );
}

/// Decision 11: once the run is terminal the window is an ordinary PTY window again.
#[tokio::test]
async fn and_allowed_once_it_is_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let (d, id) = socket_run_window(dir.path()).await;
    d.manager.set_run_window_live(id, true);
    d.manager.set_run_window_live(id, false);
    assert_eq!(d.manager.run_window_live(id), None);
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client.send(ClientMsg::Kill { window_id: id }).await;
    assert_eq!(
        reply(&mut client).await,
        DaemonMsg::Ack {
            request: "kill".into()
        }
    );
    client
        .send(ClientMsg::Remove {
            window_id: id,
            remove_worktree: false,
            force: false,
        })
        .await;
    assert_eq!(
        reply(&mut client).await,
        DaemonMsg::Ack {
            request: "remove".into()
        }
    );
    assert!(!d.manager.list().iter().any(|w| w.id == id));
}

/// Decision 39's input: a client's `Input` is noted with its time; the engine's own
/// writes are not client input.
#[tokio::test]
async fn client_input_time_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let (d, id) = socket_run_window(dir.path()).await;
    assert_eq!(d.manager.last_client_input(id), None);
    d.manager.write_input(id, b"engine\r").unwrap();
    assert_eq!(d.manager.last_client_input(id), None, "not client input");
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    let before = Instant::now();
    client
        .send(ClientMsg::Input {
            window_id: id,
            bytes: b"hello".to_vec(),
        })
        .await;
    wait_until("the input time", || {
        d.manager.last_client_input(id).is_some()
    })
    .await;
    let at = d.manager.last_client_input(id).unwrap();
    assert!(at >= before && at <= Instant::now(), "{at:?}");
}

/// Decision 12: a turn that ends on an API error sends `StopFailure`, which ends the
/// turn: `Working`, then `Idle`. The hooks arrive over the socket as `anthrex hook`
/// sends them.
#[tokio::test]
async fn stop_failure_hook_makes_the_window_idle() {
    let dir = tempfile::tempdir().unwrap();
    let (d, id) = socket_run_window(dir.path()).await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    // Viewed, so a finished turn is `Idle`, not `Done`.
    client
        .send(ClientMsg::Subscribe {
            window_id: id,
            cols: 80,
            rows: 24,
        })
        .await;
    client
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. }))
        .await;
    for (event, status) in [
        ("UserPromptSubmit", Status::Working),
        ("StopFailure", Status::Idle),
    ] {
        client
            .send(ClientMsg::HookEvent {
                window_id: id,
                source: HookSource::Claude,
                payload: json!({"hook_event_name": event, "session_id": "s-1", "prompt": "go", "error": "model_not_found"}),
            })
            .await;
        let deadline = Instant::now() + DEADLINE;
        while find(&d.manager, id).status != status {
            assert!(
                Instant::now() < deadline,
                "{event}: still {:?}",
                find(&d.manager, id).status
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
