//! Task M9.11: the rig of the driver's read-path tests (decisions 15 to 18): a real
//! daemon socket and no agent. Each call is made by `mcp::forward`, the test MCP
//! client, exactly as `anthrex mcp` forwards one: a fresh connection, `Hello`, one
//! `RunRequest::Tool`. The run is put in the engine by hand, in `running` with one task
//! working in a window that does not exist and the other blocked, so the scheduler
//! starts nothing; no agent binary exists (Claude and Codex are paths that do not
//! exist).

use std::path::Path;
use std::time::Duration;

use proto::{AgentRole, BlockReason, ClientKind, ClientMsg, DaemonMsg, RunReply, RunRequest};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::launch::LaunchGate;
use crate::run::driver::*;
use crate::run::model::Run;
use crate::run::orch::test_support::{block, orchestrator, round, run_of, task_mut};
use crate::server::{GitWiring, serve};

/// The orchestrator's window, a live sub-planner's, and the working task's.
pub(super) const ORCH: u32 = 900;
pub(super) const PLANNER: u32 = 901;
pub(super) const WORKER: u32 = 902;
/// How long any one answer may take when nothing makes it wait.
pub(super) const ANSWER: Duration = Duration::from_secs(10);

pub(super) struct Rig {
    _dir: tempfile::TempDir,
    pub(super) runs: Arc<RunService>,
    socket: PathBuf,
    pub(super) run_id: String,
    shutdown: CancellationToken,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl Rig {
    /// A daemon on a socket under `/tmp` (a socket path's length is bounded), and one
    /// run changed by `prepare` before the engine sees it.
    pub(super) async fn new(prepare: impl FnOnce(&mut Run, &Path)) -> Rig {
        Rig::with(prepare, |_, _| {}).await
    }

    /// As [`Rig::new`], with the run service's context changed by `configure` first.
    pub(super) async fn with(
        prepare: impl FnOnce(&mut Run, &Path),
        configure: impl FnOnce(&Path, &mut RunContext),
    ) -> Rig {
        let dir = tempfile::Builder::new()
            .prefix("anthrex-orch-read-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("d.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
        config.claude_bin = "/nonexistent/anthrex-test/claude".into();
        config.codex_bin = "/nonexistent/anthrex-test/codex".into();
        config.worktrees_root = dir.path().join("worktrees");
        config.launch_gate = LaunchGate::open_already();
        let (manager, _events) = WindowManager::new(config);
        let git = GitWiring::new(config::Git {
            enabled: false,
            ..config::Git::default()
        });
        let data = dir.path().join("data");
        let mut ctx = RunContext::new(
            data.clone(),
            manager.config(),
            config::Orchestrator::default(),
            git.registry.clone(),
        );
        configure(dir.path(), &mut ctx);
        let runs = RunService::new(manager.clone(), ctx);

        let mut run = run_of(2);
        run.state = proto::RunState::Running;
        run.data_dir = data.clone();
        run.root = dir.path().join("repo");
        let mut record = orchestrator();
        record.window_id = Some(ORCH);
        // Window `ORCH` does not exist. A live record over a missing window is one the
        // driver reports exited (task M9.13, decision 13), which changes the digest; the
        // read path checks only the window id, so the record is left dormant.
        record.live = false;
        run.orch.orchestrator = Some(record);
        let now = unix_now();
        let t0 = task_mut(&mut run, "t0");
        t0.state = proto::TaskState::Working;
        let mut working = round(1, 0, Default::default());
        working.window_id = Some(WORKER);
        working.pid = None;
        (working.started_at, working.last_event) = (now, now);
        t0.rounds.push(working);
        block(
            task_mut(&mut run, "t1"),
            BlockReason::Question,
            "which endpoint?",
        );
        prepare(&mut run, dir.path());
        // As the reducer leaves a run it changed, so the first step bumps nothing.
        run.orch.digest_fp = crate::run::orch::digest::fingerprint(&run);
        let run_id = run.id.clone();
        crate::lock(&runs.state).runs.insert(run_id.clone(), run);

        let shutdown = CancellationToken::new();
        runs.spawn(shutdown.clone());
        tokio::spawn(serve(
            listener,
            manager,
            git,
            runs.clone(),
            shutdown.clone(),
        ));
        Rig {
            _dir: dir,
            runs,
            socket,
            run_id,
            shutdown,
        }
    }

    pub(super) fn opts(
        &self,
        role: AgentRole,
        window_id: u32,
        epic: Option<&str>,
    ) -> mcp::McpOptions {
        mcp::McpOptions {
            role,
            run_id: self.run_id.clone(),
            task_id: None,
            scout_id: None,
            epic: epic.map(String::from),
            window_id,
            socket: self.socket.clone(),
            chain: None,
        }
    }

    /// The orchestrator's `tool`, as `anthrex mcp` forwards it: `(ok, parsed text)`.
    pub(super) async fn orch(&self, tool: &str, args: Value) -> (bool, Value) {
        self.call(self.opts(AgentRole::Orchestrator, ORCH, None), tool, args)
            .await
    }

    pub(super) async fn call(
        &self,
        opts: mcp::McpOptions,
        tool: &str,
        args: Value,
    ) -> (bool, Value) {
        let (ok, text) = mcp::forward(&opts, tool, args).await;
        let value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        (ok, value)
    }

    pub(super) fn run<T>(&self, read: impl FnOnce(&Run) -> T) -> T {
        read(&crate::lock(&self.runs.state).runs[&self.run_id])
    }

    pub(super) fn digest_rev(&self) -> u64 {
        self.run(|run| run.orch.digest_rev)
    }

    /// One client request over the socket, as `anthrex run …` sends it; its reply.
    pub(super) async fn request(&self, request: RunRequest) -> RunReply {
        use proto::{PROTO_VERSION, read_frame, write_frame};
        let stream = tokio::net::UnixStream::connect(&self.socket).await.unwrap();
        let (mut rd, mut wr) = stream.into_split();
        let hello = ClientMsg::Hello {
            proto_version: PROTO_VERSION,
            client: ClientKind::Cli,
        };
        write_frame(&mut wr, &hello).await.unwrap();
        write_frame(&mut wr, &ClientMsg::Run(request))
            .await
            .unwrap();
        tokio::time::timeout(ANSWER, async {
            loop {
                match read_frame::<_, DaemonMsg>(&mut rd).await {
                    Ok(Some(DaemonMsg::Run(reply))) => return reply,
                    Ok(Some(_)) => continue,
                    other => panic!("the connection ended: {other:?}"),
                }
            }
        })
        .await
        .expect("the request is answered")
    }

    /// `run edit <run> cancel t1`: a change the digest shows.
    pub(super) async fn cancel_t1(&self) {
        let edit = serde_json::from_value(json!({"op": "cancel_task", "task_id": "t1"})).unwrap();
        let reply = self
            .request(RunRequest::Edit {
                run_id: self.run_id.clone(),
                edits: vec![edit],
                submit: false,
            })
            .await;
        assert!(
            matches!(reply, RunReply::Done { .. }),
            "the edit is applied: {reply:?}"
        );
    }
}
