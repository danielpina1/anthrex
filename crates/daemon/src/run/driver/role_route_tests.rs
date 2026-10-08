//! Milestone 9 task M9.13b review fixes (decision 43): the driver keeps a run scout's
//! and a run-bound decider's record in the engine, saved in `run.json`, before their
//! session starts (M-2, M-5 c), and records no decider whose daemon has the deciders
//! off (M-3). A real engine loop and `run.json` writer; no agent: Claude and Codex are
//! paths that do not exist, and the one decider is a `/bin/sh` stand-in this test
//! writes, which exits by itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{AgentRole, BlockReason, DeciderMode, RoleOutcome, ScoutKind};
use tokio_util::sync::CancellationToken;

use crate::decider::{CheckSummaryInput, DeciderRequest};
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::context::OpCtx;
use crate::run::driver::{RunContext, RunService};
use crate::run::engine::OpResult;
use crate::run::journal::RUN_FILE;
use crate::run::orch::RunScoutState;
use crate::run::orch::test_support::{block, run_of, scout, task_mut};
use crate::server::GitWiring;

const WAIT: Duration = Duration::from_secs(30);
/// How long `run.json` writes are stalled at most while a decider is dispatched.
const STALL: Duration = Duration::from_secs(3);

/// Writes a decider program under the rig's directory for the run of the given id.
type MakeBin = fn(&Path, &str) -> PathBuf;

struct Rig {
    dir: tempfile::TempDir,
    runs: Arc<RunService>,
    run_id: String,
    shutdown: CancellationToken,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

impl Rig {
    /// A run service with the milestone-8b services, the deciders in `mode` run by
    /// `decider_bin`, and one running run whose only task is blocked (the scheduler
    /// starts nothing).
    fn new(mode: DeciderMode, decider_bin: Option<MakeBin>) -> Rig {
        Rig::with(mode, decider_bin, None)
    }

    /// [`Rig::new`] with Codex's command `codex`'s stand-in (installed) when given.
    fn with(mode: DeciderMode, decider_bin: Option<MakeBin>, codex: Option<MakeBin>) -> Rig {
        let dir = tempfile::Builder::new()
            .prefix("anthrex-role-route-")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("d.sock");
        let data = dir.path().join("data");
        let mut run = run_of(1);
        run.state = proto::RunState::Running;
        run.data_dir = data.join("runs").join(&run.id);
        run.root = dir.path().join("repo");
        block(task_mut(&mut run, "t0"), BlockReason::Question, "which?");
        let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
        config.claude_bin = "/nonexistent/anthrex-test/claude".into();
        config.codex_bin = match codex {
            Some(make) => make(dir.path(), &run.id).display().to_string(),
            None => "/nonexistent/anthrex-test/codex".into(),
        };
        config.worktrees_root = dir.path().join("worktrees");
        config.launch_gate = LaunchGate::open_already();
        if let Some(make) = decider_bin {
            config.decider_bin = Some(make(dir.path(), &run.id).display().to_string());
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
        crate::profile::service::wire(&manager, &runs, &data, &socket, &orchestrator);
        let run_id = run.id.clone();
        crate::lock(&runs.state).runs.insert(run_id.clone(), run);
        let shutdown = CancellationToken::new();
        runs.spawn(shutdown.clone());
        Rig {
            dir,
            runs,
            run_id,
            shutdown,
        }
    }

    /// Run scout `id` in `state`, put in the engine by hand.
    fn scout_in(&self, id: &str, state: RunScoutState) {
        let mut engine = crate::lock(&self.runs.state);
        let run = engine.runs.get_mut(&self.run_id).unwrap();
        run.orch
            .run_scouts
            .push(scout(id, state, &["crates/m0/**"]));
    }

    /// Every later `run.json` write of the run fails: its directory would be under a
    /// regular file.
    fn break_saves(&self) {
        let file = self.dir.path().join("not-a-dir");
        std::fs::write(&file, "x").unwrap();
        let mut engine = crate::lock(&self.runs.state);
        engine.runs.get_mut(&self.run_id).unwrap().data_dir = file.join("runs");
    }

    fn ctx(&self) -> OpCtx {
        let state = crate::lock(&self.runs.state);
        OpCtx {
            run_id: self.run_id.clone(),
            ..OpCtx::of(&state.runs[&self.run_id])
        }
    }

    fn run_json(&self) -> String {
        let dir = crate::lock(&self.runs.state).runs[&self.run_id]
            .data_dir
            .clone();
        std::fs::read_to_string(dir.join(RUN_FILE)).unwrap_or_default()
    }

    fn records(&self, role: AgentRole) -> Vec<proto::RoleRoutingDecision> {
        crate::lock(&self.runs.state).runs[&self.run_id]
            .role_routing_decisions
            .iter()
            .filter(|d| d.role == role)
            .cloned()
            .collect()
    }

    /// Every event sent before this one has been handled: the engine answers in order.
    async fn settled(&self) {
        let d = crate::run::orch::roles::decider_record(
            None,
            ("0/0", "triage"),
            &[],
            (&route(), Vec::new()),
            Default::default(),
            0,
        );
        let refused = self.runs.keep_record("no-such-run", d, None).await;
        assert!(refused.is_err(), "{refused:?}");
    }
}

fn route() -> proto::Route {
    proto::Route {
        runtime: proto::Runtime::Claude,
        model: String::new(),
        effort: proto::Effort::LOW,
    }
}

fn summary_request() -> DeciderRequest {
    DeciderRequest::CheckSummary(CheckSummaryInput {
        task_id: "t0".into(),
        command: "true".into(),
        code: Some(1),
        timed_out: false,
        tail: "failed".into(),
    })
}

/// Review M-2 and M-5 (c): `start_scout` has the engine keep, and save, the scout's
/// record before it starts the session. The scout id is one the service refuses before
/// any await, and the runtime has one thread: a record merely sent (not awaited) would
/// not be in `run.json`, nor in the engine, when the op returns.
#[tokio::test(flavor = "current_thread")]
async fn a_scout_record_is_saved_before_its_session_starts() {
    let rig = Rig::new(DeciderMode::Off, None);
    rig.scout_in("Bad", RunScoutState::Running);
    let spec = bad_scout(&rig);
    let result = rig.runs.start_scout(&rig.ctx(), spec).await;
    assert!(matches!(result, OpResult::Failed { .. }), "{result:?}");
    // Read at once, with no await in between.
    let saved = rig.run_json();
    let id = format!("{}/scout/Bad", rig.run_id);
    assert!(saved.contains(&id), "run.json has no {id}");
    let records = rig.records(AgentRole::Scout);
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(
        (records[0].record_id.as_str(), records[0].outcome),
        (id.as_str(), None)
    );
    assert_eq!(records[0].source, "role_table");
}

/// A scout spec whose id (`Bad`) the scout service refuses before any await.
fn bad_scout(rig: &Rig) -> crate::scout::spec::ScoutSpec {
    crate::scout::spec::ScoutSpec {
        id: "Bad".into(),
        kind: ScoutKind::Area,
        run_id: Some(rig.run_id.clone()),
        question: "q".into(),
        first_turn: "q".into(),
        cwd: rig.dir.path().join("repo"),
        project: rig.dir.path().join("repo"),
        web: false,
        codex_config: Vec::new(),
        base_sha: "b".repeat(40),
        repo_paths: Vec::new(),
    }
}

/// Re-review 2: a scout the run stopped (`run cancel`) before its record reached the
/// engine is refused there, so `start_scout` starts no session, and nothing is recorded:
/// no session was dispatched (as with M-3's deciders off).
#[tokio::test(flavor = "multi_thread")]
async fn a_scout_stopped_before_its_record_is_kept_is_not_started() {
    let rig = Rig::new(DeciderMode::Off, None);
    rig.scout_in(
        "Bad",
        RunScoutState::Failed {
            reason: "the run was cancelled".into(),
        },
    );
    let result = rig.runs.start_scout(&rig.ctx(), bad_scout(&rig)).await;
    let OpResult::Failed { message } = &result else {
        panic!("{result:?}");
    };
    assert!(message.contains("stopped when the run ended"), "{message}");
    rig.settled().await;
    assert!(rig.records(AgentRole::Scout).is_empty());
}

/// Re-review 3: a record whose `run.json` save failed is refused, so the session does
/// not start: the scout's op fails with the reason, and the decider is not run (its
/// stand-in writes nothing) and falls back; its record says it was not started.
#[tokio::test(flavor = "multi_thread")]
async fn a_record_that_could_not_be_saved_starts_no_session() {
    let rig = Rig::new(DeciderMode::Claude, Some(witness));
    rig.scout_in("Bad", RunScoutState::Running);
    rig.break_saves();
    let result = rig.runs.start_scout(&rig.ctx(), bad_scout(&rig)).await;
    let OpResult::Failed { message } = &result else {
        panic!("{result:?}");
    };
    assert!(message.contains("could not be saved"), "{message}");
    let result = rig
        .runs
        .decide_as(&rig.ctx(), Some((5, vec!["t0".into()])), summary_request())
        .await;
    assert!(matches!(result, OpResult::Decided(_)), "{result:?}");
    assert!(
        !rig.dir.path().join("mark").exists(),
        "the decider ran though its record was not saved"
    );
    rig.settled().await;
    let decider = rig.records(AgentRole::Decider);
    assert_eq!(
        decider[0].outcome,
        Some(RoleOutcome::Failed),
        "{decider:#?}"
    );
    assert!(
        decider[0]
            .result
            .as_deref()
            .is_some_and(|r| r.contains("could not be saved"))
    );
}

/// A decider stand-in that writes whether `run.json` held its record when it started,
/// then exits (so the call falls back).
fn witness(dir: &Path, run_id: &str) -> PathBuf {
    let run_json = dir.join("data/runs").join(run_id).join(RUN_FILE);
    let mark = dir.join("mark");
    let text = format!(
        "#!/bin/sh\ncat >/dev/null\nif grep -q '\"record_id\":\"{run_id}/decider/5\"' '{}'; then echo present > '{}'; else echo absent > '{}'; fi\n",
        run_json.display(),
        mark.display(),
        mark.display()
    );
    testexec::write_executable(dir.join("decider.sh"), text)
}

/// Review M-2: a run-bound decider starts only once its record is saved. The run's
/// `run.json` writes are stalled while the engine keeps the record; the stand-in
/// decider, had it started then, would see no record.
#[tokio::test(flavor = "multi_thread")]
async fn a_decider_record_is_saved_before_its_call() {
    let rig = Rig::new(DeciderMode::Claude, Some(witness));
    let slot = rig.runs.writes.slot(&rig.run_id);
    let (release, released) = std::sync::mpsc::channel::<()>();
    let (held, holding) = std::sync::mpsc::channel::<()>();
    let stall = std::thread::spawn(move || {
        let _guard = crate::lock(&slot);
        held.send(()).unwrap();
        let _ = released.recv_timeout(WAIT);
    });
    holding.recv_timeout(WAIT).unwrap();
    let (runs, ctx) = (rig.runs.clone(), rig.ctx());
    let call = tokio::spawn(async move {
        runs.decide_as(&ctx, Some((5, vec!["t0".into()])), summary_request())
            .await
    });
    // The engine keeps the record while its save is stalled (or, if an earlier step's
    // save holds the loop, has not seen it yet) ...
    let deadline = Instant::now() + STALL;
    while rig.records(AgentRole::Decider).is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // ... and a decider started meanwhile would run while `run.json` lacks it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    release.send(()).unwrap();
    stall.join().unwrap();
    let result = tokio::time::timeout(WAIT, call).await.unwrap().unwrap();
    assert!(matches!(result, OpResult::Decided(_)), "{result:?}");
    let mark = std::fs::read_to_string(rig.dir.path().join("mark")).unwrap_or_default();
    assert_eq!(
        mark.trim(),
        "present",
        "the decider started before its record was saved"
    );
    rig.settled().await;
    let records = rig.records(AgentRole::Decider);
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].outcome, Some(RoleOutcome::Fallback));
    assert_eq!(records[0].task_id.as_deref(), Some("t0"));
    // Milestone 9.8: the `check_summary` row (the built-in `helpers`, no fallback).
    let models: Vec<&str> = (records[0].candidates.iter())
        .map(|c| c.route.model.as_str())
        .collect();
    assert_eq!(models, ["claude-haiku-4-5"], "{:#?}", records[0].candidates);
}

/// The live settings of `rig` with `helpers` = `model` falling back to `fallback`.
fn helpers_row(rig: &Rig, model: &str, fallback: Option<&str>) {
    use proto::models::{ModelRef, Role, RoleChoice};
    let mut config = config::Orchestrator::default();
    let row = RoleChoice {
        model: ModelRef::parse(model).unwrap(),
        effort: None,
        fallback: fallback.map(|f| ModelRef::parse(f).unwrap()),
    };
    config.roles.rows.insert(Role::Helpers, row);
    let live = rig.runs.live_settings();
    live.swap_owned(&config, Default::default());
}

/// Milestone 9.8 decision 42: a run-bound decider reads the live table at each call
/// (decision 9 freezes only runs), so a save made since the run started is the one its
/// record lists.
#[tokio::test(flavor = "multi_thread")]
async fn a_decider_record_lists_the_live_table() {
    let rig = Rig::new(DeciderMode::Claude, None);
    helpers_row(&rig, "codex:gpt-6-sol", None);
    let result = rig
        .runs
        .decide_as(&rig.ctx(), Some((5, vec!["t0".into()])), summary_request())
        .await;
    assert!(matches!(result, OpResult::Decided(_)), "{result:?}");
    rig.settled().await;
    let records = rig.records(AgentRole::Decider);
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].chosen.model, "gpt-6-sol");
    assert_eq!(records[0].source, "role_table");
}

/// Review M-3: with the daemon's deciders off no decider session starts, so no record
/// is opened.
#[tokio::test(flavor = "multi_thread")]
async fn no_decider_record_when_the_deciders_are_off() {
    let rig = Rig::new(DeciderMode::Off, None);
    let result = rig
        .runs
        .decide_as(&rig.ctx(), Some((5, vec!["t0".into()])), summary_request())
        .await;
    assert!(matches!(result, OpResult::Decided(_)), "{result:?}");
    rig.settled().await;
    assert!(rig.records(AgentRole::Decider).is_empty());
}

/// Whole-branch fix round 2, item 1, milestone 9.8: the scout the driver starts runs on
/// the `research` row its run froze, not on the live table the scout service reads.
/// Here the run's row is the Codex default while the live one is Claude's; the launch
/// fails naming the role and the runtime it tried (MR §7).
#[tokio::test(flavor = "multi_thread")]
async fn a_run_scout_is_started_on_its_runs_research_row() {
    let rig = Rig::new(DeciderMode::Off, None);
    std::fs::create_dir_all(rig.dir.path().join("repo")).unwrap();
    rig.scout_in("s1", RunScoutState::Running);
    {
        let mut engine = crate::lock(&rig.runs.state);
        let run = engine.runs.get_mut(&rig.run_id).unwrap();
        let research = proto::models::Role::Research;
        crate::run::test_support::set_row(run, research, "codex:default", None, None);
    }
    let live = rig.runs.adaptation.get().unwrap().scouts.context().clone();
    assert_eq!(
        crate::scout::spec::scout_route(&live).runtime,
        proto::Runtime::Claude
    );
    let spec = crate::scout::spec::ScoutSpec {
        id: "s1".into(),
        ..bad_scout(&rig)
    };
    let result = rig.runs.start_scout(&rig.ctx(), spec).await;
    let OpResult::Failed { message } = &result else {
        panic!("{result:?}");
    };
    // MR §7: the missing program names the role and the way out.
    let want = "research: codex not found; choose another model in C-b S";
    assert_eq!(message, want);
    rig.settled().await;
    let records = rig.records(AgentRole::Scout);
    assert_eq!(
        records[0].chosen.runtime,
        proto::Runtime::Codex,
        "{records:#?}"
    );
}

/// Whole-branch fix round 3, item 2: a scout whose run is gone is refused, never routed
/// on the daemon's config.
#[tokio::test(flavor = "multi_thread")]
async fn a_scout_of_a_run_that_is_gone_is_refused() {
    let rig = Rig::new(DeciderMode::Off, None);
    std::fs::create_dir_all(rig.dir.path().join("repo")).unwrap();
    let ctx = rig.ctx();
    crate::lock(&rig.runs.state).runs.remove(&rig.run_id);
    let spec = crate::scout::spec::ScoutSpec {
        id: "s1".into(),
        ..bad_scout(&rig)
    };
    let result = rig.runs.start_scout(&ctx, spec).await;
    let OpResult::Failed { message } = &result else {
        panic!("{result:?}");
    };
    assert_eq!(message, "the run is gone");
}

/// A stand-in that exits at once: never an agent.
fn exits(dir: &Path, _run_id: &str) -> PathBuf {
    testexec::write_executable(dir.join("exits.sh"), "#!/bin/sh\nexit 0\n")
}

/// Ruling T10b-1, milestone 9.8 (D2): a decider whose row names Claude, with only Codex
/// installed, is moved to the row's Codex fallback; the record skips Claude's route as
/// `not installed`, and the run log says so.
#[tokio::test(flavor = "multi_thread")]
async fn a_decider_whose_runtime_is_not_installed_moves_and_says_so() {
    let rig = Rig::with(DeciderMode::Claude, Some(exits), Some(exits));
    helpers_row(&rig, "claude:claude-haiku-4-5", Some("codex:gpt-6-luna"));
    let result = rig
        .runs
        .decide_as(&rig.ctx(), Some((5, vec!["t0".into()])), summary_request())
        .await;
    assert!(matches!(result, OpResult::Decided(_)), "{result:?}");
    rig.settled().await;
    let records = rig.records(AgentRole::Decider);
    assert_eq!(records.len(), 1, "{records:#?}");
    let d = &records[0];
    assert_eq!(d.chosen.runtime, proto::Runtime::Codex);
    let first = &d.candidates[0];
    assert_eq!(
        (first.route.runtime, first.skipped_reason.as_deref()),
        (proto::Runtime::Claude, Some("not installed"))
    );
    let log: Vec<String> = crate::lock(&rig.runs.state).runs[&rig.run_id]
        .log
        .iter()
        .map(|e| e.text.clone())
        .collect();
    let line = "decider: claude is not installed; using codex".to_string();
    assert!(log.contains(&line), "{log:#?}");
}
