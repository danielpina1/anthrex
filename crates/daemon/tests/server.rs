use daemon::manager::{ManagerConfig, WindowManager};
use daemon::server::serve;
use proto::{
    ClientKind, ClientMsg, DaemonMsg, HookSource, PROTO_VERSION, Runtime, Status, WindowSpec,
    read_frame, write_frame,
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
    manager: std::sync::Arc<WindowManager>,
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        self.shutdown.cancel();
        for window in self.manager.list() {
            let _ = self.manager.remove(window.id);
        }
    }
}

async fn start_daemon() -> TestDaemon {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let stub = dir.path().join("stub.sh");
    std::fs::write(&stub, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut config = ManagerConfig::new(socket.clone(), "/bin/sh".into());
    config.claude_bin = stub.to_str().unwrap().into();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });
    let shutdown = CancellationToken::new();
    tokio::spawn(serve(listener, manager.clone(), shutdown.clone()));
    TestDaemon {
        _dir: dir,
        socket,
        shutdown,
        manager,
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
async fn a_version_1_client_is_rejected() {
    let d = start_daemon().await;
    let (_c, reply) = Client::connect(&d, 1).await;
    match reply {
        DaemonMsg::Error { request, message } => {
            assert_eq!(request, "hello");
            assert!(message.contains("anthrex daemon stop"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_version_2_client_is_rejected() {
    let d = start_daemon().await;
    let (_c, reply) = Client::connect(&d, 2).await;
    match reply {
        DaemonMsg::Error { request, message } => {
            assert_eq!(request, "hello");
            assert!(message.contains("protocol version mismatch"));
            assert!(message.contains("anthrex daemon stop"));
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

fn git(dir: &std::path::Path, args: &[&std::ffi::OsStr]) {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn created_windows_carry_their_project_root() {
    use std::ffi::OsStr;
    let (_dir, sub, worktree, root) = tokio::task::spawn_blocking(|| {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let sub = repo.join("sub");
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(&sub).unwrap();
        git(&repo, &[OsStr::new("init")]);
        git(
            &repo,
            &[
                OsStr::new("commit"),
                OsStr::new("--allow-empty"),
                OsStr::new("-m"),
                OsStr::new("init"),
            ],
        );
        git(
            &repo,
            &[
                OsStr::new("worktree"),
                OsStr::new("add"),
                OsStr::new("-b"),
                OsStr::new("feature"),
                worktree.as_os_str(),
            ],
        );
        let root = repo.canonicalize().unwrap();
        (dir, sub, worktree, root)
    })
    .await
    .unwrap();
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    for (name, cwd) in [("main", &sub), ("worktree", &worktree)] {
        client
            .send(ClientMsg::CreateWindow {
                spec: WindowSpec {
                    cwd: cwd.clone(),
                    ..shell_spec(name)
                },
                cols: 80,
                rows: 24,
            })
            .await;
    }
    let DaemonMsg::WindowsChanged { windows } = client.recv_until(|message| {
        matches!(message, DaemonMsg::WindowsChanged { windows } if windows.len() == 2)
    }).await else { unreachable!() };
    for (name, cwd) in [("main", &sub), ("worktree", &worktree)] {
        let window = windows.iter().find(|window| window.name == name).unwrap();
        assert_eq!(&window.cwd, cwd);
        assert_eq!(window.project, root);
    }
}

#[tokio::test]
async fn a_window_outside_any_repository_is_its_own_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let d = start_daemon().await;
    let (mut client, _) = Client::connect(&d, PROTO_VERSION).await;
    client
        .send(ClientMsg::CreateWindow {
            spec: WindowSpec {
                cwd: dir.path().into(),
                ..shell_spec("plain")
            },
            cols: 80,
            rows: 24,
        })
        .await;
    let DaemonMsg::WindowsChanged { windows } = client.recv_until(|message| {
        matches!(message, DaemonMsg::WindowsChanged { windows } if windows.len() == 1)
    }).await else { unreachable!() };
    assert_eq!(windows[0].cwd, dir.path());
    assert_eq!(windows[0].project, root);
}

#[tokio::test]
async fn create_subscribe_input_and_kill_flow() {
    let started = std::time::Instant::now();
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
    assert!(started.elapsed() < Duration::from_millis(1500));
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

impl Client {
    async fn hook(&mut self, id: u32, name: &str) -> DaemonMsg {
        self.send(ClientMsg::HookEvent {
            window_id: id,
            source: HookSource::Claude,
            payload: serde_json::json!({"hook_event_name":name}),
        })
        .await;
        self.recv_until(|m| matches!(m, DaemonMsg::Ack { request } | DaemonMsg::Error { request, .. } if request == "hook")).await
    }

    async fn subscribe(&mut self, id: u32) {
        self.send(ClientMsg::Subscribe {
            window_id: id,
            cols: 80,
            rows: 24,
        })
        .await;
        self.recv_until(|m| matches!(m, DaemonMsg::Snapshot { window_id, .. } if *window_id == id))
            .await;
    }

    async fn unsubscribe(&mut self) {
        self.send(ClientMsg::Unsubscribe).await;
        self.recv_until(|m| matches!(m, DaemonMsg::Ack { request } if request == "unsubscribe"))
            .await;
    }
}

async fn claude_window(d: &TestDaemon, name: &str) -> u32 {
    let manager = d.manager.clone();
    let mut spec = shell_spec(name);
    spec.runtime = Runtime::Claude;
    tokio::task::spawn_blocking(move || {
        manager
            .create(spec, std::env::temp_dir(), None, 80, 24)
            .unwrap()
            .id
    })
    .await
    .unwrap()
}

async fn assert_completion(d: &TestDaemon, client: &mut Client, id: u32, expected: Status) {
    assert!(matches!(
        client.hook(id, "UserPromptSubmit").await,
        DaemonMsg::Ack { .. }
    ));
    assert!(matches!(
        client.hook(id, "Stop").await,
        DaemonMsg::Ack { .. }
    ));
    assert_eq!(
        d.manager.list().iter().find(|w| w.id == id).unwrap().status,
        expected
    );
}

#[tokio::test]
async fn hook_events_are_acknowledged() {
    let d = start_daemon().await;
    let manager = d.manager.clone();
    let id = tokio::task::spawn_blocking(move || {
        manager
            .create(shell_spec("hook-shell"), std::env::temp_dir(), None, 80, 24)
            .unwrap()
            .id
    })
    .await
    .unwrap();
    let (mut c, _) = Client::connect(&d, PROTO_VERSION).await;
    assert_eq!(
        c.hook(id, "Stop").await,
        DaemonMsg::Ack {
            request: "hook".into()
        }
    );
    assert_eq!(
        c.hook(99, "Stop").await,
        DaemonMsg::Error {
            request: "hook".into(),
            message: "no window with id 99".into()
        }
    );
}

#[tokio::test]
async fn a_subscription_marks_the_window_viewed_until_the_client_leaves() {
    let d = start_daemon().await;
    let id = claude_window(&d, "viewed").await;
    let (mut a, _) = Client::connect(&d, PROTO_VERSION).await;
    let (mut b, _) = Client::connect(&d, PROTO_VERSION).await;
    a.subscribe(id).await;
    assert_completion(&d, &mut b, id, Status::Idle).await;
    drop(a);
    wait_unviewed(&d, &mut b, id).await;
}

async fn wait_unviewed(d: &TestDaemon, client: &mut Client, id: u32) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            client.hook(id, "UserPromptSubmit").await;
            client.hook(id, "Stop").await;
            if d.manager.list().iter().find(|w| w.id == id).unwrap().status == Status::Done {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("disconnected client retained its viewer");
}

#[tokio::test]
async fn subscriptions_count_clients_and_release_on_switch_and_unsubscribe() {
    let d = start_daemon().await;
    let one = claude_window(&d, "one").await;
    let two = claude_window(&d, "two").await;
    let (mut a, _) = Client::connect(&d, PROTO_VERSION).await;
    let (mut b, _) = Client::connect(&d, PROTO_VERSION).await;
    let (mut observer, _) = Client::connect(&d, PROTO_VERSION).await;
    a.subscribe(one).await;
    a.subscribe(one).await;
    b.subscribe(one).await;
    a.unsubscribe().await;
    a.unsubscribe().await;
    assert_completion(&d, &mut observer, one, Status::Idle).await;
    b.subscribe(two).await;
    assert_completion(&d, &mut observer, one, Status::Done).await;
    assert_completion(&d, &mut observer, two, Status::Idle).await;
    b.unsubscribe().await;
    assert_completion(&d, &mut observer, two, Status::Done).await;
    a.subscribe(one).await;
    a.send(ClientMsg::Subscribe {
        window_id: 99,
        cols: 80,
        rows: 24,
    })
    .await;
    a.recv_until(|m| matches!(m, DaemonMsg::Error { request, .. } if request == "subscribe"))
        .await;
    assert_completion(&d, &mut observer, one, Status::Done).await;
}

#[tokio::test]
async fn malformed_frames_release_the_subscription_viewer() {
    use tokio::io::AsyncWriteExt;
    let d = start_daemon().await;
    let id = claude_window(&d, "malformed").await;
    let (mut a, _) = Client::connect(&d, PROTO_VERSION).await;
    let (mut observer, _) = Client::connect(&d, PROTO_VERSION).await;
    a.subscribe(id).await;
    assert_completion(&d, &mut observer, id, Status::Idle).await;
    a.wr.write_all(&[0, 0, 0, 1, 0xc1]).await.unwrap();
    wait_unviewed(&d, &mut observer, id).await;
}
