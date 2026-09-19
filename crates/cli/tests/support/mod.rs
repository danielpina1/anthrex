#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::time::{Duration, Instant};

use proto::{ClientKind, ClientMsg, DaemonMsg, Runtime, WindowInfo, WindowSpec};
use serde_json::Value;
use tempfile::NamedTempFile;
use tokio::net::UnixStream;

pub const ANTHREX: &str = env!("CARGO_BIN_EXE_anthrex");

pub fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ax-")
        .tempdir_in("/tmp")
        .unwrap()
}

pub fn fake_agent_bin() -> PathBuf {
    let path = Path::new(ANTHREX).with_file_name("fake-agent");
    assert!(
        path.is_file(),
        "fake-agent not built; run cargo build -p anthrex-fake-agent"
    );
    path
}

pub fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

pub fn isolated_command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(ANTHREX);
    command
        .args(args)
        .env("ANTHREX_SOCKET", dir.join("daemon.sock"))
        .env("ANTHREX_DATA_DIR", dir.join("data"));
    command
}

/// Capture output without making direct-child completion depend on inherited pipe handles.
pub struct RunningCommand {
    description: String,
    child: Child,
    stdout: NamedTempFile,
    stderr: NamedTempFile,
}

impl RunningCommand {
    pub fn start(command: &mut Command) -> Self {
        let description = format!(
            "{:?} {:?}",
            command.get_program(),
            command.get_args().take(2).collect::<Vec<_>>()
        );
        let stdout = NamedTempFile::new().unwrap();
        let stderr = NamedTempFile::new().unwrap();
        let child = command
            .stdin(Stdio::piped())
            .stdout(stdout.reopen().unwrap())
            .stderr(stderr.reopen().unwrap())
            .spawn()
            .unwrap();
        Self {
            description,
            child,
            stdout,
            stderr,
        }
    }

    pub fn stdin(&mut self) -> ChildStdin {
        self.child.stdin.take().unwrap()
    }

    pub fn input(&mut self, bytes: Vec<u8>) {
        let mut stdin = self.stdin();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&bytes);
        });
    }

    pub fn is_running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }

    pub fn finish(mut self, limit: Duration) -> Output {
        drop(self.child.stdin.take());
        let deadline = Instant::now() + limit;
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "child {} ({}) exceeded test deadline {limit:?}",
                self.child.id(),
                self.description
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut stdout = Vec::new();
        self.stdout
            .as_file_mut()
            .read_to_end(&mut stdout)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to read stdout: {error}; child {} ({}) exited with {status}",
                    self.child.id(),
                    self.description
                )
            });
        let mut stderr = Vec::new();
        self.stderr
            .as_file_mut()
            .read_to_end(&mut stderr)
            .unwrap_or_else(|error| {
                panic!(
                    "failed to read stderr: {error}; child {} ({}) exited with {status}",
                    self.child.id(),
                    self.description
                )
            });
        Output {
            status,
            stdout,
            stderr,
        }
    }
}

impl Drop for RunningCommand {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}

fn terminate(child: &mut Child) {
    if child.try_wait().ok().flatten().is_some() {
        return;
    }
    let _ = child.kill();
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub fn assert_silent_success(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}, stderr {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.is_empty(),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        output.stderr.is_empty(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub struct TestDaemon {
    dir: tempfile::TempDir,
    data: PathBuf,
    child: Child,
}

impl TestDaemon {
    pub fn start(script: &[Value]) -> Self {
        let dir = tempdir();
        let script_path = dir.path().join("script.jsonl");
        let lines = script
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&script_path, lines).unwrap();
        let log = std::fs::File::create(dir.path().join("daemon.log")).unwrap();
        let child = isolated_command(dir.path(), &["daemon", "start", "--foreground"])
            .env("ANTHREX_CLAUDE_BIN", fake_agent_bin())
            .env("ANTHREX_CODEX_BIN", fake_agent_bin())
            .env("FAKE_AGENT_SCRIPT", script_path)
            .env("FAKE_AGENT_ARGS_FILE", dir.path().join("data/args.json"))
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let data = dir.path().join("data");
        let mut daemon = Self { dir, data, child };
        let deadline = Instant::now() + Duration::from_secs(3);
        while !daemon.socket().exists() {
            assert!(
                daemon.child.try_wait().unwrap().is_none(),
                "daemon exited: {}",
                daemon.log()
            );
            assert!(
                Instant::now() < deadline,
                "daemon did not create socket: {}",
                daemon.log()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        daemon
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("daemon.log")).unwrap_or_default()
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("daemon.sock")
    }
    pub fn data_dir(&self) -> &Path {
        &self.data
    }
    pub fn client(&self) -> Client {
        Client::connect(&self.socket(), self.dir.path())
    }
    pub fn command(&self, args: &[&str]) -> Command {
        isolated_command(self.dir.path(), args)
    }
    pub fn anthrex(&self, args: &[&str]) -> Output {
        RunningCommand::start(&mut self.command(args)).finish(Duration::from_secs(3))
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let rt = runtime();
        let _ = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(1), async {
                let mut stream = UnixStream::connect(self.socket()).await?;
                proto::write_frame(
                    &mut stream,
                    &ClientMsg::Hello {
                        proto_version: proto::PROTO_VERSION,
                        client: ClientKind::Cli,
                    },
                )
                .await?;
                let _ = proto::read_frame::<_, DaemonMsg>(&mut stream).await?;
                proto::write_frame(&mut stream, &ClientMsg::Shutdown).await
            })
            .await
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        terminate(&mut self.child);
    }
}

pub struct Client {
    rt: tokio::runtime::Runtime,
    stream: UnixStream,
    cwd: PathBuf,
}

impl Client {
    fn connect(socket: &Path, cwd: &Path) -> Self {
        let rt = runtime();
        let stream = rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                let mut stream = UnixStream::connect(socket).await.unwrap();
                proto::write_frame(
                    &mut stream,
                    &ClientMsg::Hello {
                        proto_version: proto::PROTO_VERSION,
                        client: ClientKind::Cli,
                    },
                )
                .await
                .unwrap();
                assert!(matches!(
                    proto::read_frame::<_, DaemonMsg>(&mut stream)
                        .await
                        .unwrap(),
                    Some(DaemonMsg::Welcome { .. })
                ));
                stream
            })
            .await
            .expect("client handshake timeout")
        });
        Self {
            rt,
            stream,
            cwd: cwd.to_owned(),
        }
    }

    pub fn send(&mut self, message: ClientMsg) {
        self.rt.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                proto::write_frame(&mut self.stream, &message),
            )
            .await
            .expect("client write timeout")
            .unwrap();
        });
    }

    pub fn receive(&mut self, mut pred: impl FnMut(&DaemonMsg) -> bool) -> DaemonMsg {
        self.rt.block_on(async {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let message = proto::read_frame::<_, DaemonMsg>(&mut self.stream)
                        .await
                        .unwrap()
                        .expect("daemon disconnected");
                    assert!(!matches!(&message, DaemonMsg::Error { .. }), "{message:?}");
                    if pred(&message) {
                        return message;
                    }
                }
            })
            .await
            .expect("client reply timeout")
        })
    }

    pub fn create(&mut self, runtime: Runtime, name: &str) -> u32 {
        self.create_spec(WindowSpec {
            name: Some(name.into()),
            runtime,
            cwd: self.cwd.clone(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        })
    }

    pub fn create_spec(&mut self, spec: WindowSpec) -> u32 {
        self.send(ClientMsg::CreateWindow {
            spec,
            cols: 80,
            rows: 24,
        });
        match self.receive(|msg| matches!(msg, DaemonMsg::Created { .. })) {
            DaemonMsg::Created { window_id } => window_id,
            _ => unreachable!(),
        }
    }

    pub fn windows(&mut self) -> Vec<WindowInfo> {
        self.send(ClientMsg::ListWindows);
        match self.receive(|msg| matches!(msg, DaemonMsg::WindowsChanged { .. })) {
            DaemonMsg::WindowsChanged { windows } => windows,
            _ => unreachable!(),
        }
    }

    pub fn wait_window(
        &mut self,
        id: u32,
        what: &str,
        pred: impl Fn(&WindowInfo) -> bool,
    ) -> WindowInfo {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            let windows = self.windows();
            if let Some(window) = windows
                .iter()
                .find(|window| window.id == id && pred(window))
            {
                return window.clone();
            }
            assert!(Instant::now() < deadline, "waiting for {what}: {windows:?}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    pub fn subscribe(&mut self, id: u32) {
        self.send(ClientMsg::Subscribe {
            window_id: id,
            cols: 80,
            rows: 24,
        });
        self.receive(
            |msg| matches!(msg, DaemonMsg::Snapshot { window_id, .. } if *window_id == id),
        );
    }

    pub fn input(&mut self, id: u32, bytes: &[u8]) {
        self.send(ClientMsg::Input {
            window_id: id,
            bytes: bytes.to_vec(),
        });
    }

    pub fn remains(&mut self, id: u32, duration: Duration, pred: impl Fn(&WindowInfo) -> bool) {
        let deadline = Instant::now() + duration;
        while Instant::now() < deadline {
            let windows = self.windows();
            let window = windows
                .iter()
                .find(|window| window.id == id)
                .expect("window missing");
            assert!(pred(window), "unexpected window: {window:?}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
