//! Milestone 9.3 task 6b: a next goal on a chain's orchestrator (decision 22) through a
//! real daemon socket, a temporary git checkout whose `origin` is a local bare
//! repository the fake GitHub knows (`delivery_tests.rs`' rig: no network, no `gh`),
//! a stored profile, and a decider stand-in that records each call. The chain's
//! window is a real PTY run window whose `claude` is a stand-in that sleeps; no agent
//! runs, and the test kills only that window. Also decision 23's adoption off every
//! lock, and decision 24's bounded history read.

use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::run_wire::request;
use proto::{AgentRole, DeliveryMode, Effort, RunPath, RunRef, RunReply, RunRequest, RunState};
use proto::{Runtime, WindowSpec};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{HANDOFF_LINES, Next, history_lines};
use crate::headless::McpTarget;
use crate::launch::LaunchGate;
use crate::launch::role::RoleLaunch;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::chain::{ChainState, rebuild};
use crate::run::driver::delivery::tests::start::decider_stand_in;
use crate::run::driver::delivery::tests::{Rig as Checkout, git};
use crate::run::driver::{RunContext, RunService};
use crate::run::model::Run;
use crate::run::orch::contract_rounds::handoff_prompt;
use crate::run::orch::test_support::{orchestrator, run_of};
use crate::server::{GitWiring, serve};

/// The previous run, accepted, and its chain.
pub(in crate::run::driver) const PREV: &str = "first-goal-3f9a";
pub(in crate::run::driver) const CHAIN: &str = "o-3f9a";
/// The rig's `git_timeout_secs` (the run harness's) and its host calls' cap.
pub(in crate::run::driver) const GIT_TIMEOUT_SECS: u64 = 5;
const HOST_CAP: Duration = Duration::from_secs(10);
/// How long one continued start may take (`docs/timing-budgets.md`): at most 24 git
/// calls at `GIT_TIMEOUT_SECS` (the roots' detection, `goal_ready`'s preflight, the
/// build's preflight, protected files, run refs, `.codex` tree and settings reads) and
/// the history read's own bound, 120 s; the delivery's preflight under `HOST_CAP`; one
/// engine step.
pub(in crate::run::driver) const ANSWER: Duration = Duration::from_secs(150);

pub(in crate::run::driver) fn role(run_id: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: run_id.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: Some(CHAIN.into()),
            lane: None,
            agent_label: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

/// `claude` as a stand-in that sleeps, in `dir`.
pub(in crate::run::driver) fn sleeping_claude(dir: &Path) -> String {
    let claude = dir.join("claude");
    std::fs::write(&claude, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    claude.to_str().unwrap().into()
}

pub(in crate::run::driver) struct ChainRig {
    pub checkout: Checkout,
    _sock: tempfile::TempDir,
    pub socket: PathBuf,
    pub s: Arc<RunService>,
    pub manager: Arc<WindowManager>,
    shutdown: CancellationToken,
    marker: PathBuf,
    pub window: u32,
    project: PathBuf,
    root: PathBuf,
}

impl ChainRig {
    /// A daemon over a checkout with a stored profile, and [`PREV`] accepted as its
    /// chain's last run, its orchestrator's window open: an idle, unended chain.
    /// `prepare` changes the previous run first.
    pub(in crate::run::driver) async fn new(prepare: impl FnOnce(&mut Run)) -> ChainRig {
        ChainRig::with_context(prepare, |_| {}).await
    }

    /// [`ChainRig::new`] with the run context changed by `adjust` first.
    pub(in crate::run::driver) async fn with_context(
        prepare: impl FnOnce(&mut Run),
        adjust: impl FnOnce(&mut RunContext),
    ) -> ChainRig {
        let checkout = Checkout::ready(true);
        let tmp = checkout.tmp.path().to_path_buf();
        let sock = tempfile::Builder::new()
            .prefix("anthrex-chain-goal-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = sock.path().join("d.sock");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let (decider, marker) = decider_stand_in(&tmp);
        let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
        config.claude_bin = sleeping_claude(&tmp);
        config.decider_bin = Some(decider);
        config.worktrees_root = tmp.join("worktrees");
        config.launch_gate = LaunchGate::open_already();
        let (manager, _events) = WindowManager::new(config);
        let wiring = GitWiring::new(config::Git {
            enabled: false,
            ..config::Git::default()
        });
        let data = tmp.join("data");
        let mut ctx = context(&checkout, &manager, &wiring);
        adjust(&mut ctx);
        let s = RunService::new(manager.clone(), ctx);
        crate::profile::service::wire(
            &manager,
            &s,
            &data,
            &socket,
            &config::Orchestrator::default(),
        );
        let pre = crate::run::git::preflight("git".as_ref(), &checkout.work, ANSWER).unwrap();
        let repo_dir = crate::profile::repo_dir(&data, &pre.project);
        std::fs::create_dir_all(&repo_dir).unwrap();
        let meta = proto::ProfileMeta {
            confirmed_at: 1,
            report: None,
            verification: None,
            fingerprint: std::collections::BTreeMap::new(),
            edited_keys: Vec::new(),
            project: Some(pre.project.clone()),
        };
        crate::profile::store::save(&repo_dir, &proto::RepoProfile::default(), &meta).unwrap();

        let spec = WindowSpec {
            name: Some("3f9a/orchestrator".into()),
            runtime: Runtime::Claude,
            cwd: checkout.work.clone(),
            worktree_branch: None,
            model: None,
            initial_prompt: None,
        };
        let window = manager
            .create_run_window(spec, checkout.work.clone(), role(PREV))
            .await
            .expect("the stand-in's window")
            .id;
        let mut prev = run_of(1);
        prev.id = PREV.into();
        prev.project = pre.project.clone();
        prev.root = pre.root.clone();
        prev.repo_dir = repo_dir;
        prev.data_dir = data.join("runs").join(PREV);
        prev.state = RunState::Accepted;
        prev.chain = Some(CHAIN.into());
        let resolved = crate::run::orch::launch::resolve_orchestrator(
            None,
            &config::AgentConfig::default(),
            prev.limits.default_runtime,
            &prev.roster,
        )
        .unwrap();
        let mut record = orchestrator();
        record.route = resolved.route;
        record.window_id = Some(window);
        record.live = false;
        prev.orch.orchestrator = Some(record);
        prepare(&mut prev);
        {
            let mut state = crate::lock(&s.state);
            state.runs.insert(PREV.into(), prev);
            state.chains = rebuild(&state.runs);
            if let Some(chain) = state.chains.get_mut(CHAIN) {
                chain.ended = false;
            }
        }
        let shutdown = CancellationToken::new();
        s.spawn(shutdown.clone());
        tokio::spawn(serve(
            listener,
            manager.clone(),
            wiring,
            s.clone(),
            shutdown.clone(),
        ));
        ChainRig {
            checkout,
            _sock: sock,
            socket,
            s,
            manager,
            shutdown,
            marker,
            window,
            project: pre.project,
            root: pre.root,
        }
    }

    /// A goal continuing `after` from `dir`, as the TUI and `--continue` send it.
    pub(in crate::run::driver) async fn continued(&self, after: &str, dir: &Path) -> RunReply {
        let req = RunRequest::StartGoal {
            goal: "Add a logout button".into(),
            dir: dir.to_path_buf(),
            yes: false,
            trust_project: false,
            unconfined_checks: true,
            orchestrator: None,
            delivery: Some(DeliveryMode::Local),
            continue_from: Some(after.into()),
            design: None,
        };
        tokio::time::timeout(ANSWER, self.s.request(req))
            .await
            .expect("answered")
    }

    /// The runs of the chain's that are not [`PREV`].
    pub(in crate::run::driver) fn new_runs(&self) -> Vec<Run> {
        let state = crate::lock(&self.s.state);
        let runs = state.runs.values();
        runs.filter(|r| r.id != PREV && r.chain.as_deref() == Some(CHAIN))
            .cloned()
            .collect()
    }

    pub(in crate::run::driver) async fn stop(self) {
        self.shutdown.cancel();
        self.s.stop().await;
    }
}

/// A failing test still stops its daemon and kills the window it made.
impl Drop for ChainRig {
    fn drop(&mut self) {
        self.shutdown.cancel();
        let _ = self.manager.kill(self.window);
    }
}

/// The rig's run context over `checkout`'s data directory (a restarted daemon's too).
pub(in crate::run::driver) fn context(
    checkout: &Checkout,
    manager: &Arc<WindowManager>,
    wiring: &GitWiring,
) -> RunContext {
    // Milestone 9.6 ruling T3-2: the design flow is off unless a test opts in.
    let mut orch_config = config::Orchestrator {
        git_timeout_secs: GIT_TIMEOUT_SECS,
        ..config::Orchestrator::default()
    };
    orch_config.design.default = proto::DesignMode::Off;
    let data = checkout.tmp.path().join("data");
    let mut ctx = RunContext::new(data, manager.config(), orch_config, wiring.registry.clone())
        .with_host(checkout.host());
    ctx.host_cap = Some(HOST_CAP);
    ctx
}

pub(in crate::run::driver) fn started(reply: &RunReply) -> String {
    match reply {
        RunReply::Started {
            run_id,
            state: RunState::Planning,
            ..
        } => run_id.clone(),
        other => panic!("not started: {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_from_is_refused_with_each_text() {
    let rig = ChainRig::new(|_| {}).await;
    {
        let mut state = crate::lock(&rig.s.state);
        // An earlier run of the chain, and a run with no chain.
        let mut older = state.runs[PREV].clone();
        older.id = "older-goal-1b2c".into();
        older.created_at -= 1;
        state.runs.insert(older.id.clone(), older);
        let mut plain = state.runs[PREV].clone();
        plain.id = "plain-goal-5d6e".into();
        plain.chain = None;
        state.runs.insert(plain.id.clone(), plain);
        state.chains.get_mut(CHAIN).unwrap().runs = vec!["older-goal-1b2c".into(), PREV.into()];
    }
    let work = rig.checkout.work.clone();
    let refused = |message: String| RunReply::refused(request::START_GOAL, message);
    let runs = || crate::lock(&rig.s.state).runs.len();
    let before = runs();

    let reply = rig.continued("older-goal-1b2c", &work).await;
    assert_eq!(
        reply,
        refused("run 1b2c is not the last run of o-3f9a; continue from run 3f9a".into())
    );
    let reply = rig.continued("plain-goal-5d6e", &work).await;
    assert_eq!(
        reply,
        refused(
            "run 5d6e has no orchestrator to continue; start a new goal without --continue".into()
        )
    );
    crate::lock(&rig.s.state)
        .chains
        .get_mut(CHAIN)
        .unwrap()
        .state = ChainState::Active;
    let reply = rig.continued(PREV, &work).await;
    assert_eq!(
        reply,
        refused("run 3f9a is still going; finish it before starting another goal".into())
    );
    crate::lock(&rig.s.state)
        .chains
        .get_mut(CHAIN)
        .unwrap()
        .state = ChainState::Idle;

    // Another project.
    let other = rig.checkout.tmp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    git(&other, &["init", "-q", "-b", "main"]);
    git(&other, &["commit", "-q", "--allow-empty", "-m", "other"]);
    let reply = rig.continued(PREV, &other).await;
    let text = format!(
        "o-3f9a belongs to {}; start this goal there, or with a new orchestrator",
        rig.project.display()
    );
    assert_eq!(reply, refused(text));

    // A branch reserved for runs: `git::preflight`'s own text.
    git(&work, &["checkout", "-q", "-b", "anthrex/elsewhere"]);
    let reply = rig.continued(PREV, &work).await;
    let text = format!(
        "{} is on anthrex/elsewhere, a branch reserved for runs; check out your own branch first",
        rig.root.display()
    );
    assert_eq!(reply, refused(text));
    git(&work, &["checkout", "-q", "main"]);

    assert_eq!(runs(), before, "no run was started");
    assert!(!rig.marker.exists(), "no decider was called");
    rig.stop().await;
}

/// Decision 22, step 4, from the tool: the previous run's delivery mode, trust and
/// unconfined checks, never its "approve at once"; and decision 23: the new run adopts
/// the chain's window, renamed and rebound, live for it.
#[tokio::test(flavor = "multi_thread")]
async fn start_goal_inherits_delivery_trust_and_checks_but_not_approve_at_once() {
    let rig = ChainRig::new(|prev| {
        prev.delivery.mode = DeliveryMode::Pr;
        prev.trust_project = true;
        prev.limits.unconfined_checks = true;
        prev.orch.yes = true;
    })
    .await;
    let prev = crate::lock(&rig.s.state).runs[PREV].clone();
    assert_eq!(
        Next::inherited(&prev, "g".into()),
        Next {
            goal: "g".into(),
            dir: rig.project.clone(),
            trust_project: true,
            unconfined_checks: true,
            yes: false,
            delivery: Some(DeliveryMode::Pr),
            design: None,
        }
    );
    let opts = mcp::McpOptions {
        role: AgentRole::Orchestrator,
        run_id: PREV.into(),
        task_id: None,
        scout_id: None,
        epic: None,
        window_id: rig.window,
        socket: rig.socket.clone(),
        chain: Some(CHAIN.into()),
        lane: None,
    };
    let (ok, text) = tokio::time::timeout(
        ANSWER,
        mcp::forward(&opts, "start_goal", json!({"goal": "Add a logout button"})),
    )
    .await
    .expect("answered");
    assert!(ok, "{text}");
    let new = rig.new_runs();
    assert_eq!(new.len(), 1, "{text}");
    let run = &new[0];
    let answer: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        answer,
        json!({"message": format!("run {} started", run.id)})
    );
    assert_eq!(run.delivery.mode, DeliveryMode::Pr);
    assert!(run.trust_project);
    // Fix round 1 (m3): the inherited permission is consumed at the start
    // (`confine::start_refusal`, which passed); the built run records whether its
    // checks do run unconfined, which they do only where the sandbox is missing.
    let unconfined = run.limits.worker_sandbox && !crate::run::confine::available();
    assert_eq!(run.limits.unconfined_checks, unconfined);
    assert!(!run.orch.yes, "its plan stops at the gate");
    assert_eq!(run.state, RunState::Planning);

    // The window is adopted: renamed and rebound to the new run, live for it.
    let expected = RunRef {
        run_id: run.id.clone(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session: 1,
        lane: None,
    };
    let deadline = Instant::now() + ANSWER;
    while rig.manager.run_window_live(rig.window) != Some(expected.clone()) {
        assert!(Instant::now() < deadline, "the window was never adopted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let name = format!("{}/orchestrator", run.short());
    assert!(
        rig.manager
            .list()
            .iter()
            .any(|w| w.id == rig.window && w.name == name)
    );
    let chain = crate::lock(&rig.s.state).chains[CHAIN].clone();
    assert_eq!(chain.runs, [PREV.to_string(), run.id.clone()]);
    assert_eq!(chain.state, ChainState::Active);
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continued_goal_skips_triage() {
    let rig = ChainRig::new(|_| {}).await;
    let reply = rig.continued(PREV, &rig.checkout.work.clone()).await;
    let run_id = started(&reply);
    let run = crate::lock(&rig.s.state).runs[&run_id].clone();
    assert_eq!(run.triage, None);
    assert_eq!(run.path, Some(RunPath::Plan));
    assert_eq!(run.chain.as_deref(), Some(CHAIN));
    // The run title change: the continued goal's run is named (that decider falls back
    // here), but triage is never asked.
    let calls = std::fs::read_to_string(&rig.marker).unwrap_or_default();
    assert!(!calls.contains("triage v1"), "triage was called: {calls}");
    assert!(
        calls.contains("run_name v1"),
        "the run was not named: {calls}"
    );
    assert_eq!(run.title, "", "a fallback gives no title");
    rig.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_continued_run_is_based_on_the_checkout() {
    let rig = ChainRig::new(|_| {}).await;
    let work = rig.checkout.work.clone();
    // The accepted work, on the user's branch since the previous run.
    git(
        &work,
        &["commit", "-q", "--allow-empty", "-m", "accepted work"],
    );
    let head = git(&work, &["rev-parse", "HEAD"]);
    let run_id = started(&rig.continued(PREV, &work).await);
    let run = crate::lock(&rig.s.state).runs[&run_id].clone();
    assert_eq!(run.log[0].text, format!("based on main at {}", &head[..7]));
    assert_eq!(run.base_sha, head);
    rig.stop().await;
}

/// Decision 24: the chain's history lines for a fresh session's first prompt are read
/// within the run's `git_timeout_secs`; a history file that does not answer (a FIFO
/// whose writer sends nothing) gives `(history unavailable)` within that bound plus
/// slack (`docs/timing-budgets.md`). A readable file gives the chain's last ten lines
/// as written.
///
/// Task 6b fix rounds 3 and 4: no thread can wait for good. The writer opens the FIFO
/// only once the read has (a non-blocking open fails with `ENXIO` until a reader is
/// there), within a deadline; past it, it opens it `O_RDWR`, which never blocks, so a
/// read that comes late opens it too. It holds the FIFO, so the read's open completes
/// and its read waits, until the test has measured the read (its signal) or for `HOLD`
/// at most, and for `GRACE` after its own open at least (the final fix wave, task 6b
/// m1); then it unlinks the FIFO and closes it, which ends the abandoned read (EOF)
/// and leaves no file for a read that comes later still. Before fix round 3, the
/// writer opened and closed at once, after the bound, and on macOS the reader blocked
/// in `open` could miss that writer and wait for good (about one run in three).
#[tokio::test(flavor = "multi_thread")]
async fn the_handoff_history_read_is_bounded() {
    // The writer's hold when no signal comes (a read that were not bounded keeps the
    // test from measuring): past the bound plus the 5 s slack below, so such a read
    // would end (EOF) only after the slack, and fail.
    const HOLD: Duration = Duration::from_secs(9);
    // The writer's least hold after its own open: a read that starts after the bound
    // (the blocking pool can start its thread late) still opens against a writer.
    const GRACE: Duration = Duration::from_secs(1);
    let dir = tempfile::tempdir().unwrap();
    let mut run = run_of(1);
    run.limits.git_timeout_secs = 1;
    let bound = Duration::from_secs(run.limits.git_timeout_secs);
    let chain = vec![PREV.to_string(), "older-goal-1b2c".to_string()];

    let fifo = dir.path().join("history.jsonl");
    let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let held = fifo.clone();
    let (release, released) = std::sync::mpsc::channel::<()>();
    let writer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let (file, opened) = loop {
            let opened = std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&held);
            match opened {
                Ok(file) => break (file, true),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                // Fix round 4 (M1): never blocks, and releases a read that comes late.
                Err(_) => {
                    let mut rdwr = std::fs::OpenOptions::new();
                    rdwr.read(true).write(true);
                    break (rdwr.open(&held).expect("a FIFO opens O_RDWR"), false);
                }
            }
        };
        // The final fix wave (task 6b m1): at least `GRACE` after its own open, even when
        // the signal came first, so a read that reaches `open` late still finds the
        // writer there rather than an open-then-close it can miss.
        let open_at = Instant::now();
        let _ = released.recv_timeout(HOLD);
        std::thread::sleep(GRACE.saturating_sub(open_at.elapsed()));
        let _ = std::fs::remove_file(&held);
        drop(file);
        opened
    });
    let started = Instant::now();
    let read = history_lines(fifo.clone(), chain.clone(), bound);
    let lines = tokio::time::timeout(Duration::from_secs(30), read)
        .await
        .expect("the read is bounded");
    let took = started.elapsed();
    let _ = release.send(());
    let opened = tokio::task::spawn_blocking(move || writer.join().unwrap())
        .await
        .unwrap();
    assert!(opened, "the read never opened the history file");
    let prompt = handoff_prompt("first", (CHAIN, "3f9a", "accepted"), None, lines.as_deref());
    assert_eq!(lines, None);
    assert!(
        prompt.ends_with("```\n(history unavailable)\n```\n"),
        "{prompt}"
    );
    assert!(took >= bound, "{took:?}");
    assert!(took < bound + Duration::from_secs(5), "{took:?}");

    // A readable file: the chain's runs' lines only, the last ten, as written.
    let file = dir.path().join("readable.jsonl");
    let mut text = String::new();
    for n in 0..14 {
        let run_id = if n % 2 == 0 { PREV } else { "older-goal-1b2c" };
        text.push_str(&format!(
            "{{\"type\":\"task\",\"run_id\":\"{run_id}\",\"n\":{n}}}\n"
        ));
        text.push_str(&format!(
            "{{\"type\":\"task\",\"run_id\":\"other-9999\",\"n\":{n}}}\n"
        ));
    }
    text.push_str("not json\n");
    std::fs::write(&file, text).unwrap();
    let lines = history_lines(file, chain, bound).await.unwrap();
    let kept: Vec<&str> = lines.lines().collect();
    assert_eq!(kept.len(), HANDOFF_LINES);
    assert!(kept[0].contains("\"n\":4"), "{lines}");
    assert!(kept[9].contains("\"n\":13"), "{lines}");
    assert!(!lines.contains("other-9999"), "{lines}");
}
