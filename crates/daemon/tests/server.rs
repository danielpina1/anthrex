use daemon::manager::WindowManager;
use daemon::server::serve;
use proto::{
    ClientKind, ClientMsg, DaemonMsg, PROTO_VERSION, Runtime, Status, WindowSpec, read_frame,
    write_frame,
};
use std::path::PathBuf;
use std::time::Duration;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::sync::CancellationToken;

struct TestDaemon {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    shutdown: CancellationToken,
}

async fn start_daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let (manager, mut events) = WindowManager::new(socket.clone(), "/bin/sh".into());
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(listener, manager, shutdown.clone()));
    TestDaemon {
        _dir: dir,
        socket,
        shutdown,
    }
}

struct Client {
    rd: OwnedReadHalf,
    wr: OwnedWriteHalf,
}

impl Client {
    async fn connect(d: &TestDaemon, version: u32) -> (Self, DaemonMsg) {
        let stream = UnixStream::connect(&d.socket).await.unwrap();
        let (rd, wr) = stream.into_split();
        let mut c = Client { rd, wr };
        c.send(ClientMsg::Hello {
            proto_version: version,
            client: ClientKind::Cli,
        })
        .await;
        let first = c.recv().await;
        (c, first)
    }

    async fn send(&mut self, m: ClientMsg) {
        write_frame(&mut self.wr, &m).await.unwrap();
    }

    async fn recv(&mut self) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(5), read_frame(&mut self.rd))
            .await
            .expect("timed out")
            .unwrap()
            .expect("daemon closed")
    }

    async fn recv_until(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let m = self.recv().await;
                if pred(&m) {
                    return m;
                }
            }
        })
        .await
        .expect("timed out waiting for message")
    }
}

fn shell_spec(name: &str) -> WindowSpec {
    WindowSpec {
        name: Some(name.into()),
        runtime: Runtime::Shell,
        cwd: std::env::temp_dir(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    }
}

#[tokio::test]
async fn handshake_returns_welcome_with_empty_window_list() {
    let d = start_daemon().await;
    let (_c, welcome) = Client::connect(&d, PROTO_VERSION).await;
    match welcome {
        DaemonMsg::Welcome { windows, .. } => assert!(windows.is_empty()),
        other => panic!("expected Welcome, got {other:?}"),
    }
}

#[tokio::test]
async fn version_mismatch_is_rejected() {
    let d = start_daemon().await;
    let (_c, reply) = Client::connect(&d, PROTO_VERSION + 1).await;
    match reply {
        DaemonMsg::Error { request, message } => {
            assert_eq!(request, "hello");
            assert!(message.contains("anthrex daemon stop"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn create_subscribe_input_and_kill_flow() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;

    c.send(ClientMsg::CreateWindow {
        spec: shell_spec("w"),
        cols: 80,
        rows: 24,
    })
    .await;
    let created = c
        .recv_until(|m| matches!(m, DaemonMsg::Created { .. }))
        .await;
    let DaemonMsg::Created { window_id } = created else {
        unreachable!()
    };

    c.send(ClientMsg::Subscribe {
        window_id,
        cols: 100,
        rows: 30,
    })
    .await;
    let snap = c
        .recv_until(|m| matches!(m, DaemonMsg::Snapshot { .. }))
        .await;
    let DaemonMsg::Snapshot { cols, rows, .. } = snap else {
        unreachable!()
    };
    assert_eq!((cols, rows), (100, 30));

    c.send(ClientMsg::Input {
        window_id,
        bytes: b"echo srv-$((1+1))-ok\n".to_vec(),
    })
    .await;
    let mut seen = Vec::new();
    c.recv_until(|m| {
        if let DaemonMsg::Output { bytes, .. } = m {
            seen.extend_from_slice(bytes);
        }
        String::from_utf8_lossy(&seen).contains("srv-2-ok")
    })
    .await;

    c.send(ClientMsg::Kill { window_id }).await;
    let ack = c.recv_until(|m| matches!(m, DaemonMsg::Ack { .. })).await;
    assert_eq!(
        ack,
        DaemonMsg::Ack {
            request: "kill".into()
        }
    );
    c.recv_until(|m| match m {
        DaemonMsg::WindowsChanged { windows } => windows
            .iter()
            .any(|w| w.id == window_id && w.status == Status::Exited),
        _ => false,
    })
    .await;

    c.send(ClientMsg::Remove {
        window_id,
        remove_worktree: false,
        force: false,
    })
    .await;
    c.recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "remove"))
        .await;
}

#[tokio::test]
async fn errors_are_reported_per_request() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    c.send(ClientMsg::Subscribe {
        window_id: 42,
        cols: 80,
        rows: 24,
    })
    .await;
    let reply = c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await;
    let DaemonMsg::Error { request, message } = reply else {
        unreachable!()
    };
    assert_eq!(request, "subscribe");
    assert!(message.contains("42"));
    c.send(ClientMsg::Restart { window_id: 1 }).await;
    let DaemonMsg::Error { request, .. } =
        c.recv_until(|m| matches!(m, DaemonMsg::Error { .. })).await
    else {
        unreachable!()
    };
    assert_eq!(request, "restart");
}

#[tokio::test]
async fn shutdown_request_says_bye_and_stops_serving() {
    let d = start_daemon().await;
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    c.send(ClientMsg::Shutdown).await;
    let bye = c.recv_until(|m| matches!(m, DaemonMsg::Bye { .. })).await;
    assert!(matches!(bye, DaemonMsg::Bye { .. }));
    tokio::time::timeout(Duration::from_secs(2), d.shutdown.cancelled())
        .await
        .expect("token cancelled");
}
