//! The end-to-end harness of milestone 8a (brief, "Shared test helpers"): one scratch
//! repository, one isolated daemon with `fake-agent` as both runtimes, and raw socket
//! clients for the run requests; `anthrex run` itself is driven through [`RunHarness::anthrex`].
//!
//! Every daemon has its own `ANTHREX_SOCKET` and `ANTHREX_DATA_DIR` under `/tmp` and is
//! stopped when the harness is dropped (AGENTS.md hard rule 1).

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{
    ClientKind, ClientMsg, DaemonMsg, RunInfo, RunReply, RunRequest, RunsSnapshot, WindowInfo,
};
use serde_json::Value;
use tokio::net::UnixStream;

use super::run_daemon::{DAEMON_EXIT_WAIT, DAEMON_START_WAIT, DaemonProcess};
use super::{ANTHREX, RunningCommand, fake_agent_bin, runtime};

/// One task path (brief, "End-to-end tests"): 40 engine git calls at 5 s, 4 check or
/// proof runs at 10 s and 20 s of scripted waits, 260 s, rounded up. A scenario of `k`
/// task paths waits `k * RUN_WAIT` (`docs/timing-budgets.md`).
pub const RUN_WAIT: Duration = Duration::from_secs(300);

/// How long one raw request may take, `run accept` and `run discard` aside: `run
/// start`'s legal worst case is its preflight's git calls at the harness's 5 s
/// `git_timeout_secs` (six calls, 30 s) plus the id draw's and the settings scan's
/// (three more, 15 s), 45 s; every other request is one engine step. Recorded in
/// `docs/timing-budgets.md`.
const REQUEST_WAIT: Duration = Duration::from_secs(60);

/// How long `Finish` may take: its own reads (three git calls, 15 s), then the op.
/// Accept's merge runs under `ACCEPT_MERGE_TIMEOUT` (600 s, never shortened); its other
/// git calls for a one-task run (at most 8 checks, salvage and removal of 4 worktrees
/// at most 10 calls each, at most 4 for the branches: 52 calls at 5 s, 260 s) fit in
/// `RUN_WAIT`. The harness's tests accept one-task runs only.
const FINISH_WAIT: Duration = Duration::from_secs(600 + RUN_WAIT.as_secs());

/// The bound for `request`'s reply.
fn request_wait(request: &RunRequest) -> Duration {
    match request {
        RunRequest::Finish { .. } => FINISH_WAIT,
        _ => REQUEST_WAIT,
    }
}

pub struct RunHarness {
    pub dir: tempfile::TempDir,
    pub repo: PathBuf,
    /// `FAKE_AGENT_ARGS_FILE` and `FAKE_AGENT_STDIN_FILE`: `<name>.args`, `<name>.stdin`.
    pub io: PathBuf,
    env: Vec<(String, String)>,
    /// The daemon this harness started and must stop (`run_daemon.rs`).
    daemon: Mutex<Option<DaemonProcess>>,
}

/// Initialises a repository at `path` with one commit of `README` (and `files`).
pub fn init_repo(path: &Path, files: &[(&str, &str)]) {
    std::fs::create_dir_all(path).unwrap();
    git_in(path, &["init", "-q", "-b", "main"]);
    git_in(path, &["config", "user.name", "Test User"]);
    git_in(path, &["config", "user.email", "test@example.com"]);
    git_in(path, &["config", "commit.gpgsign", "false"]);
    std::fs::write(path.join("README"), "readme\n").unwrap();
    for (name, content) in files {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, content).unwrap();
    }
    git_in(path, &["add", "-A"]);
    git_in(path, &["commit", "-q", "-m", "initial"]);
}

/// `git <args>` in `dir`, isolated from the user's configuration; its trimmed stdout.
pub fn git_in(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

impl RunHarness {
    /// `orchestrator` is extra lines for the `[orchestrator]` table.
    pub fn new(orchestrator: &str) -> Self {
        Self::with_env(orchestrator, &[], true)
    }

    /// With extra daemon environment; `git_off` sets `ANTHREX_GIT=off`.
    pub fn with_env(orchestrator: &str, env: &[(&str, &str)], git_off: bool) -> Self {
        Self::with_repo(orchestrator, env, git_off, &[])
    }

    /// With extra files in the base commit.
    pub fn with_repo(
        orchestrator: &str,
        env: &[(&str, &str)],
        git_off: bool,
        files: &[(&str, &str)],
    ) -> Self {
        let harness = Self::unstarted(orchestrator, env, git_off, files);
        if let Err(error) = harness.start_daemon(DAEMON_START_WAIT) {
            panic!("the daemon did not start: {error}\n{}", harness.log_tail());
        }
        harness
    }

    /// A harness whose daemon start waited only `start_wait` (a slow start, simulated),
    /// and what that start came to. The harness owns the daemon either way.
    pub fn try_started(orchestrator: &str, start_wait: Duration) -> (Self, Result<(), String>) {
        let harness = Self::unstarted(orchestrator, &[], true, &[]);
        let started = harness.start_daemon(start_wait);
        (harness, started)
    }

    fn unstarted(
        orchestrator: &str,
        env: &[(&str, &str)],
        git_off: bool,
        files: &[(&str, &str)],
    ) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("ax-run")
            .tempdir_in("/tmp")
            .unwrap();
        let repo = dir.path().join("repo");
        init_repo(&repo, files);
        let io = dir.path().join("agent-io");
        std::fs::create_dir_all(&io).unwrap();
        let config = dir.path().join("config.toml");
        // Final fix batch F1c round 2: where the platform cannot confine checks (Linux
        // CI), the e2e runs allow it, as a user must; a test that says otherwise wins.
        let orchestrator =
            if cfg!(target_os = "macos") || orchestrator.contains("unconfined_checks") {
                orchestrator.to_string()
            } else {
                format!("unconfined_checks = true\n{orchestrator}")
            };
        std::fs::write(
            &config,
            format!(
                "[orchestrator]\ngit_timeout_secs = 5\n{orchestrator}\n\n[orchestrator.profile]\ncheck_timeout_secs = 10\n"
            ),
        )
        .unwrap();
        let fake = fake_agent_bin();
        let mut all: Vec<(String, String)> = vec![
            ("ANTHREX_SOCKET".into(), path(&dir.path().join("d.sock"))),
            ("ANTHREX_DATA_DIR".into(), path(&dir.path().join("data"))),
            ("ANTHREX_CONFIG".into(), path(&config)),
            ("ANTHREX_CLAUDE_BIN".into(), path(&fake)),
            ("ANTHREX_CODEX_BIN".into(), path(&fake)),
            ("FAKE_AGENT_ARGS_FILE".into(), path(&io)),
            ("FAKE_AGENT_STDIN_FILE".into(), path(&io)),
            ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
            ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
        ];
        if git_off {
            all.push(("ANTHREX_GIT".into(), "off".into()));
        }
        all.extend(env.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        RunHarness {
            dir,
            repo,
            io,
            env: all,
            daemon: Mutex::new(None),
        }
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("d.sock")
    }

    pub fn data(&self) -> PathBuf {
        self.dir.path().join("data")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(ANTHREX);
        // Decision 50's check reads the daemon's own `ANTHROPIC_API_KEY`: never the
        // developer's.
        command
            .args(args)
            .env_remove("ANTHREX_GIT")
            .env_remove("ANTHROPIC_API_KEY");
        for (key, value) in &self.env {
            command.env(key, value);
        }
        command
    }

    /// `anthrex <args>` against this harness's daemon.
    pub fn anthrex(&self, args: &[&str]) -> Output {
        RunningCommand::start(&mut self.command(args)).finish(REQUEST_WAIT)
    }

    /// `anthrex <args>` with `input` on its stdin (M8a.23's prompts), waiting at most
    /// [`FINISH_WAIT`]: `run accept` and `run discard` wait on their op, not on one
    /// engine step.
    pub fn anthrex_input(&self, args: &[&str], input: &str) -> Output {
        let mut running = RunningCommand::start(&mut self.command(args));
        running.input(input.as_bytes().to_vec());
        running.finish(FINISH_WAIT)
    }

    /// Starts `anthrex daemon start --foreground` as this harness's own child and waits
    /// at most `wait` for its socket. On `Err` the daemon is still owned, and stopped
    /// with the harness.
    fn start_daemon(&self, wait: Duration) -> Result<(), String> {
        let mut command = self.command(&["daemon", "start", "--foreground"]);
        let mut daemon = DaemonProcess::spawn(&mut command, &self.dir.path().join("daemon.out"));
        let up = daemon.wait_up(&self.socket(), wait);
        *self.daemon() = Some(daemon);
        up
    }

    /// The owned daemon's slot, recovering from a poisoned lock (N1): `Drop` must
    /// always reach the daemon.
    fn daemon(&self) -> std::sync::MutexGuard<'_, Option<DaemonProcess>> {
        self.daemon
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The pid of the daemon this harness started, while it is owned.
    pub fn daemon_pid(&self) -> Option<u32> {
        self.daemon().as_ref().map(DaemonProcess::pid)
    }

    /// Stops the owned daemon, waiting out a slow start first (`DaemonProcess::stop`),
    /// and removes a socket file a killed daemon left behind.
    fn stop_daemon(&self) {
        let taken = self.daemon().take();
        let Some(mut daemon) = taken else {
            return;
        };
        // Never panics (M8a.24 fix round 2, N2): a stop that times out falls through to
        // the kill.
        daemon.stop(&self.socket(), DAEMON_START_WAIT, || {
            let _ = RunningCommand::start(&mut self.command(&["daemon", "stop"]))
                .try_finish(REQUEST_WAIT);
        });
        if std::os::unix::net::UnixStream::connect(self.socket()).is_err() {
            let _ = std::fs::remove_file(self.socket());
        }
    }

    /// Stops the daemon and starts it again with the same environment plus `extra`.
    pub fn restart_daemon(&mut self, extra: &[(&str, &str)]) {
        self.stop_daemon();
        assert!(!self.socket().exists(), "the daemon did not stop");
        self.env
            .extend(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        if let Err(error) = self.start_daemon(DAEMON_START_WAIT) {
            panic!("the daemon did not restart: {error}\n{}", self.log_tail());
        }
    }

    /// Drops `key` from the daemon's environment from its next (re)start on.
    pub fn unset_env(&mut self, key: &str) {
        self.env.retain(|(k, _)| k != key);
    }

    /// The last 60 lines of `daemon.log`.
    pub fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(self.data().join("daemon.log")).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(60)..].join("\n")
    }

    /// Writes `<repo>/.git/fake-agent/<name>.jsonl`.
    pub fn script(&self, name: &str, steps: &[Value]) {
        script_in(&self.repo, name, steps);
    }

    /// The plan file, outside the repository.
    pub fn plan(&self, toml: &str) -> PathBuf {
        let path = self.dir.path().join("plan.toml");
        std::fs::write(&path, toml).unwrap();
        path
    }

    /// `git <args>` in the repository.
    pub fn git(&self, args: &[&str]) -> String {
        git_in(&self.repo, args)
    }

    /// One raw run request on a fresh connection, and its reply.
    pub fn request(&self, request: RunRequest) -> RunReply {
        let socket = self.socket();
        let wait = request_wait(&request);
        let rt = runtime();
        rt.block_on(async move {
            let mut stream = connect(&socket).await;
            proto::write_frame(&mut stream, &ClientMsg::Run(request))
                .await
                .unwrap();
            tokio::time::timeout(wait, async {
                loop {
                    match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
                        Ok(Some(DaemonMsg::Run(reply))) => return reply,
                        Ok(Some(DaemonMsg::Error { message, .. })) => panic!("error: {message}"),
                        Ok(Some(_)) => {}
                        other => panic!("connection ended: {other:?}"),
                    }
                }
            })
            .await
            .expect("run request timed out")
        })
    }

    /// Sends one raw run request whose daemon may die before answering (decision 48's
    /// crash injection); the reply, if one came.
    pub fn request_unanswered(&self, request: RunRequest) -> Option<RunReply> {
        let socket = self.socket();
        let wait = request_wait(&request);
        runtime().block_on(async move {
            let mut stream = connect(&socket).await;
            proto::write_frame(&mut stream, &ClientMsg::Run(request))
                .await
                .ok()?;
            tokio::time::timeout(wait, async {
                loop {
                    match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
                        Ok(Some(DaemonMsg::Run(reply))) => return Some(reply),
                        Ok(Some(_)) => {}
                        _ => return None,
                    }
                }
            })
            .await
            .expect("the daemon neither answered nor died")
        })
    }

    /// After the daemon died without its shutdown (decision 48's abort): waits until
    /// its socket refuses connections, then removes the socket file, so the next
    /// [`Self::restart_daemon`] starts a fresh one.
    pub fn forget_dead_daemon(&self) {
        let deadline = Instant::now() + REQUEST_WAIT;
        while std::os::unix::net::UnixStream::connect(self.socket()).is_ok() {
            assert!(Instant::now() < deadline, "the daemon did not die");
            std::thread::sleep(Duration::from_millis(50));
        }
        // M8a.24 fix round 2 (N1): taken out of the lock first, and killed before the
        // assertion, so a failure neither poisons the lock nor leaks the process.
        let taken = self.daemon().take();
        if let Some(mut daemon) = taken {
            let exited = daemon.wait_exit(DAEMON_EXIT_WAIT);
            daemon.kill();
            assert!(exited, "the dead daemon's process was still running");
        }
        let _ = std::fs::remove_file(self.socket());
    }

    /// `Start` in the harness repository; the run id.
    pub fn start(&self, plan_toml: &str, yes: bool) -> String {
        self.start_in(&self.repo, plan_toml, yes, false)
    }

    pub fn start_in(&self, dir: &Path, plan_toml: &str, yes: bool, trust: bool) -> String {
        match self.start_reply(dir, plan_toml, yes, trust) {
            RunReply::Started { run_id, .. } => run_id,
            other => panic!("start refused: {other:?}\n{}", self.log_tail()),
        }
    }

    pub fn start_reply(&self, dir: &Path, plan_toml: &str, yes: bool, trust: bool) -> RunReply {
        self.request(RunRequest::Start {
            plan_toml: plan_toml.to_string(),
            dir: dir.to_path_buf(),
            yes,
            trust_project: trust,
            unconfined_checks: false,
        })
    }

    pub fn snapshot(&self) -> RunsSnapshot {
        match self.request(RunRequest::List) {
            RunReply::Snapshot(snapshot) => snapshot,
            other => panic!("List answered {other:?}"),
        }
    }

    pub fn run(&self, id: &str) -> Option<RunInfo> {
        self.snapshot().runs.into_iter().find(|r| r.run_id == id)
    }

    /// Polls `List` until run `id` satisfies `pred`, for at most `wait`.
    pub fn wait_run(&self, id: &str, pred: impl Fn(&RunInfo) -> bool, wait: Duration) -> RunInfo {
        let deadline = Instant::now() + wait;
        let mut last = None;
        loop {
            if let Some(run) = self.run(id) {
                if pred(&run) {
                    return run;
                }
                last = Some(run);
            }
            if Instant::now() >= deadline {
                panic!(
                    "run {id} did not get there within {wait:?}; last snapshot:\n{}\n--- daemon.log:\n{}",
                    serde_json::to_string_pretty(&last).unwrap(),
                    self.log_tail()
                );
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    /// Every listed window.
    pub fn windows(&self) -> Vec<WindowInfo> {
        let socket = self.socket();
        runtime().block_on(async move {
            let mut stream = connect(&socket).await;
            proto::write_frame(&mut stream, &ClientMsg::ListWindows)
                .await
                .unwrap();
            loop {
                match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
                    Ok(Some(DaemonMsg::WindowsChanged { windows })) => return windows,
                    Ok(Some(_)) => {}
                    other => panic!("connection ended: {other:?}"),
                }
            }
        })
    }

    /// A raw client that sends `Subscribe` and records every message it receives.
    pub fn subscribe(&self) -> RunWatcher {
        RunWatcher::connect(&self.socket(), Some(ClientMsg::Run(RunRequest::Subscribe)))
    }

    /// A raw client that records every message it receives, sending `first` if any.
    pub fn watch(&self, first: Option<ClientMsg>) -> RunWatcher {
        RunWatcher::connect(&self.socket(), first)
    }

    /// The lines of `<io>/<name>.<ext>` (`args` or `stdin`).
    pub fn io_lines(&self, name: &str, ext: &str) -> Vec<String> {
        std::fs::read_to_string(self.io.join(format!("{name}.{ext}")))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

impl RunHarness {
    /// The turn message of each Codex process claimed as `<name>`: its argv's last
    /// argument (`FAKE_AGENT_ARGS_FILE`, one JSON array per process).
    pub fn codex_messages(&self, name: &str) -> Vec<String> {
        self.io_lines(name, "args")
            .iter()
            .filter_map(|l| serde_json::from_str::<Vec<String>>(l).ok())
            .filter(|argv| argv.first().is_some_and(|a| a == "exec"))
            .filter_map(|argv| argv.last().cloned())
            .collect()
    }
}

impl Drop for RunHarness {
    fn drop(&mut self) {
        self.stop_daemon();
    }
}

fn path(p: &Path) -> String {
    p.display().to_string()
}

/// Writes `<repo>/.git/fake-agent/<name>.jsonl`.
pub fn script_in(repo: &Path, name: &str, steps: &[Value]) {
    let dir = repo.join(".git").join("fake-agent");
    std::fs::create_dir_all(&dir).unwrap();
    let lines: Vec<String> = steps.iter().map(Value::to_string).collect();
    std::fs::write(dir.join(format!("{name}.jsonl")), lines.join("\n") + "\n").unwrap();
}

async fn connect(socket: &Path) -> UnixStream {
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
    match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
        Ok(Some(DaemonMsg::Welcome { .. })) => stream,
        other => panic!("no Welcome: {other:?}"),
    }
}

/// A connection read on its own thread; every message it receives is kept in order.
pub struct RunWatcher {
    messages: Arc<Mutex<VecDeque<DaemonMsg>>>,
    all: Arc<Mutex<Vec<DaemonMsg>>>,
    /// The unix time each message of `all` was received at.
    times: Arc<Mutex<Vec<f64>>>,
    sender: tokio::sync::mpsc::UnboundedSender<ClientMsg>,
    _thread: std::thread::JoinHandle<()>,
}

impl RunWatcher {
    fn connect(socket: &Path, first: Option<ClientMsg>) -> Self {
        let socket = socket.to_path_buf();
        let messages: Arc<Mutex<VecDeque<DaemonMsg>>> = Arc::default();
        let all: Arc<Mutex<Vec<DaemonMsg>>> = Arc::default();
        let times: Arc<Mutex<Vec<f64>>> = Arc::default();
        let (sender, mut outgoing) = tokio::sync::mpsc::unbounded_channel::<ClientMsg>();
        if let Some(first) = first {
            sender.send(first).unwrap();
        }
        let (queue, log, stamps) = (messages.clone(), all.clone(), times.clone());
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            runtime().block_on(async move {
                let stream = connect(&socket).await;
                let _ = ready_tx.send(());
                let (mut rd, mut wr) = stream.into_split();
                let writer = tokio::spawn(async move {
                    while let Some(msg) = outgoing.recv().await {
                        if proto::write_frame(&mut wr, &msg).await.is_err() {
                            break;
                        }
                    }
                });
                while let Ok(Some(msg)) = proto::read_frame::<_, DaemonMsg>(&mut rd).await {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs_f64();
                    stamps.lock().unwrap().push(now);
                    log.lock().unwrap().push(msg.clone());
                    queue.lock().unwrap().push_back(msg);
                }
                writer.abort();
            });
        });
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the watcher connected");
        RunWatcher {
            messages,
            all,
            times,
            sender,
            _thread: thread,
        }
    }

    pub fn send(&self, msg: ClientMsg) {
        self.sender.send(msg).unwrap();
    }

    /// Every message received so far, in order.
    pub fn received(&self) -> Vec<DaemonMsg> {
        self.all.lock().unwrap().clone()
    }

    /// Every message received so far, in order, with the unix time it arrived at.
    pub fn received_at(&self) -> Vec<(f64, DaemonMsg)> {
        let times = self.times.lock().unwrap().clone();
        times.into_iter().zip(self.received()).collect()
    }

    /// Every `RunsSnapshot` received so far, in order.
    pub fn snapshots(&self) -> Vec<RunsSnapshot> {
        self.received()
            .into_iter()
            .filter_map(|m| match m {
                DaemonMsg::Run(RunReply::Snapshot(s)) => Some(s),
                _ => None,
            })
            .collect()
    }

    /// Waits for a message matching `pred`, taking every message before it off the
    /// queue.
    pub fn wait_for(&self, wait: Duration, pred: impl Fn(&DaemonMsg) -> bool) -> DaemonMsg {
        let deadline = Instant::now() + wait;
        loop {
            while let Some(msg) = self.messages.lock().unwrap().pop_front() {
                if pred(&msg) {
                    return msg;
                }
            }
            assert!(
                Instant::now() < deadline,
                "no matching message within {wait:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
