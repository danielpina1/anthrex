//! M8b.15 review (I3, I4, I5): the run service as the OTLP receiver's sink.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{RunState, TokenUsage};
use tokio_util::sync::CancellationToken;

use super::super::{Msg, RunService};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::metering::UsageSink;
use crate::run::engine::EventKind;
use crate::run::model::Run;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn service(data: &Path) -> Arc<RunService> {
    let config = ManagerConfig::for_tests("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots))
}

/// A run in `state` that dispatches nothing: `Paused` is live and schedules no work.
fn run(id: &str, data: &Path, state: RunState) -> Run {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = id.to_string();
    run.data_dir = data.join("runs").join(id);
    run.state = state;
    run
}

fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        output: 2 * n,
        cache_read: 3 * n,
        cache_write: 4 * n,
    }
}

/// Waits, to a deadline, until `done` holds.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A step that changes nothing, so the loop refreshes the live runs.
fn nudge(s: &RunService) {
    s.send(EventKind::OrchestratorUsage {
        run_id: "r-none".into(),
        usage: TokenUsage::default(),
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn only_runs_the_daemon_has_and_that_have_not_ended_are_live() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    {
        let mut state = crate::lock(&s.state);
        for (id, run_state) in [("r1", RunState::Paused), ("r2", RunState::Discarded)] {
            state
                .runs
                .insert(id.into(), run(id, data.path(), run_state));
        }
    }
    let handle = s.spawn(CancellationToken::new());
    nudge(&s);
    until("r1 is live", || s.is_live("r1")).await;
    assert!(!s.is_live("r2"), "a discarded run is not live");
    assert!(!s.is_live("r-junk"));
    let generation = s.live_generation();
    // An unchanged set keeps its generation.
    nudge(&s);
    nudge(&s);
    s.post("r-none".into(), TokenUsage::default());
    until("the posts are drained", || {
        crate::lock(&s.metered.pending).is_empty()
    })
    .await;
    assert_eq!(s.live_generation(), generation);
    // A run that ends is no longer live, and the generation moves.
    crate::lock(&s.state).runs.get_mut("r1").unwrap().state = RunState::Accepted;
    nudge(&s);
    until("r1 is no longer live", || !s.is_live("r1")).await;
    assert!(s.live_generation() > generation);
    s.stop().await;
    let _ = handle.await;
}

/// Review I4: a flood of posts queues one drain, and the engine gets the latest total
/// of each run once.
#[tokio::test]
async fn a_flood_of_posts_queues_one_drain_with_the_latest_totals() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut rx = crate::lock(&s.rx).take().expect("the loop never ran");
    for n in 1..=1000 {
        s.post("r1".into(), usage(n));
        s.post("r2".into(), usage(2 * n));
    }
    let mut drains = 0;
    while let Ok(msg) = rx.try_recv() {
        assert!(matches!(msg, Msg::Usage), "only drains are queued");
        drains += 1;
    }
    assert_eq!(drains, 1);
    let pending = s.metered.take();
    assert_eq!(
        pending.into_iter().collect::<Vec<_>>(),
        vec![
            ("r1".to_string(), usage(1000)),
            ("r2".to_string(), usage(2000))
        ]
    );
    // Once drained, the next post queues the next drain.
    s.post("r1".into(), usage(1));
    assert!(matches!(rx.try_recv(), Ok(Msg::Usage)));
    assert!(rx.try_recv().is_err());
}

/// Review I5: a restored run is live, and a total posted after the restart adds to the
/// usage it had.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_run_is_live_and_its_usage_adds_to_what_was_stored() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("r-back", data.path(), RunState::Running);
    stored.orchestrator_usage = usage(999);
    stored.revision = 3;
    crate::run::journal::save_run(&stored).unwrap();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert!(s.is_live("r-back"), "a restored run is live at once");
    let handle = s.spawn(CancellationToken::new());
    s.post("r-back".into(), usage(5));
    let total = || crate::lock(&s.state).runs["r-back"].orchestrator_usage;
    until("the total reaches the run", || total() == usage(1004)).await;
    s.stop().await;
    let _ = handle.await;
}

/// Review minor 2 (mutant B): a step that ends one run and starts another keeps the
/// count of live runs but changes them, and the new set is taken.
#[test]
fn live_runs_that_change_but_keep_their_count_are_refreshed() {
    let data = tempfile::tempdir().unwrap();
    let metered = super::Metered::default();
    let mut state = crate::run::engine::EngineState::default();
    state
        .runs
        .insert("r1".into(), run("r1", data.path(), RunState::Paused));
    metered.refresh_live(&state);
    let generation = metered.generation.load(std::sync::atomic::Ordering::SeqCst);
    state.runs.get_mut("r1").unwrap().state = RunState::Accepted;
    state
        .runs
        .insert("r2".into(), run("r2", data.path(), RunState::Paused));
    metered.refresh_live(&state);
    let live = crate::lock(&metered.live).clone();
    assert_eq!(live, ["r2".to_string()].into_iter().collect());
    assert!(metered.generation.load(std::sync::atomic::Ordering::SeqCst) > generation);
}

/// Review minor 2 (mutant D): one drain gives every pending run its total, not only
/// the first.
#[tokio::test(flavor = "multi_thread")]
async fn one_drain_gives_every_pending_run_its_total() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    {
        let mut state = crate::lock(&s.state);
        for id in ["r1", "r2", "r3"] {
            state
                .runs
                .insert(id.into(), run(id, data.path(), RunState::Paused));
        }
    }
    // Queued before the loop runs, so the three totals are in one drain.
    s.post("r1".into(), usage(1));
    s.post("r2".into(), usage(2));
    s.post("r3".into(), usage(3));
    let handle = s.spawn(CancellationToken::new());
    let total = |id: &str| crate::lock(&s.state).runs[id].orchestrator_usage;
    until("every run has its total", || {
        (total("r1"), total("r2"), total("r3")) == (usage(1), usage(2), usage(3))
    })
    .await;
    s.stop().await;
    let _ = handle.await;
}

/// Review minor 1: the base a restore sets is never stored, so a run the restore does
/// not otherwise change is not rewritten because it has orchestrator usage.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_run_with_orchestrator_usage_writes_nothing() {
    use std::os::unix::fs::MetadataExt;
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("done", data.path(), RunState::Accepted);
    stored.orchestrator_usage = usage(999);
    stored.revision = 4;
    // Milestone 9.3 decision 18: a run from 9.3 on has its round 1 stored.
    stored.rounds.push(crate::run::model::Round::first(&stored));
    crate::run::journal::save_run(&stored).unwrap();
    let file = stored.data_dir.join(crate::run::journal::RUN_FILE);
    let before = std::fs::metadata(&file).unwrap().ino();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert_eq!(
        crate::lock(&s.state).runs["done"].orchestrator_usage,
        usage(999)
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().ino(),
        before,
        "run.json was rewritten"
    );
}

/// M8b.15 re-review 2, m1: a run the restore does change is saved, orchestrator usage
/// and all. `same_on_disk` answering "same" for it would leave `run.json` stale.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_run_the_restore_changes_saves_it() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("live", data.path(), RunState::Running);
    stored.orchestrator_usage = usage(999);
    crate::run::journal::save_run(&stored).unwrap();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert_eq!(crate::lock(&s.state).runs["live"].state, RunState::Paused);
    let file = stored.data_dir.join(crate::run::journal::RUN_FILE);
    let on_disk: Run = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        on_disk.state,
        RunState::Paused,
        "run.json was not rewritten"
    );
    assert_eq!(on_disk.orchestrator_usage, usage(999));
}

/// `run` with an orchestrator whose OTLP token is `token`, continued by `next` if set.
fn chained(id: &str, data: &Path, state: RunState, token: &str, next: Option<&str>) -> Run {
    let mut run = run(id, data, state);
    let mut o = crate::run::orch::test_support::orchestrator();
    o.otlp_token = token.into();
    run.orch.orchestrator = Some(o);
    run.continued_by = next.map(String::from);
    run
}

/// `run`, its session adopted when its counter was `at`.
fn adopted(mut run: Run, at: TokenUsage) -> Run {
    run.orch.orchestrator.as_mut().unwrap().usage_at_adopt = Some(at);
    run
}

/// Milestone 9.5 decision 37 (FU-F43): an adopted session posts under the chain's first
/// run, which resolves to the chain's current run: live while it is, its token, and its
/// credit. The continued runs are not live themselves.
#[test]
fn a_continued_runs_post_is_credited_to_the_current_run() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut rx = crate::lock(&s.rx).take().expect("the loop never ran");
    let mut state = crate::run::engine::EngineState::default();
    for (id, run_state, token, next) in [
        ("r1", RunState::Complete, "t1", Some("r2")),
        ("r2", RunState::Paused, "t2", None),
    ] {
        let run = chained(id, data.path(), run_state, token, next);
        state.runs.insert(id.into(), adopted(run, usage(0)));
    }
    s.metered.refresh_live(&state);
    assert!(s.is_live("r1"), "r1 resolves to r2, which is live");
    assert_eq!(s.token("r1").as_deref(), Some("t2"));
    assert_eq!(s.live_orchestrators(), 1, "r1's session is r2's");
    assert_eq!(
        *crate::lock(&s.metered.live),
        ["r1", "r2"].into_iter().map(String::from).collect()
    );
    s.post("r1".into(), usage(5));
    assert!(matches!(rx.try_recv(), Ok(Msg::Usage)));
    assert_eq!(
        s.metered.take().into_iter().collect::<Vec<_>>(),
        vec![("r2".to_string(), usage(5))]
    );
    // A third run: the chain's first run resolves to its last one.
    state.runs.get_mut("r2").unwrap().continued_by = Some("r3".into());
    let third = chained("r3", data.path(), RunState::Paused, "t3", None);
    state.runs.insert("r3".into(), adopted(third, usage(0)));
    s.metered.refresh_live(&state);
    assert_eq!(s.token("r1").as_deref(), Some("t3"));
    s.post("r1".into(), usage(6));
    assert_eq!(
        s.metered.take().into_iter().collect::<Vec<_>>(),
        vec![("r3".to_string(), usage(6))]
    );
    // Once the current run ends, nothing in the chain is live.
    let generation = s.live_generation();
    state.runs.get_mut("r3").unwrap().state = RunState::Accepted;
    s.metered.refresh_live(&state);
    assert!(!s.is_live("r1") && !s.is_live("r2") && !s.is_live("r3"));
    assert_eq!(s.token("r1"), None);
    assert!(s.live_generation() > generation);
}

/// FU-F40: a delivered `pr` run a next goal continued keeps the usage it had; the
/// session's later totals go to the run that adopted it.
#[tokio::test(flavor = "multi_thread")]
async fn a_delivered_previous_run_is_not_credited() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    {
        let mut state = crate::lock(&s.state);
        let mut r1 = chained("r1", data.path(), RunState::Complete, "t", Some("r2"));
        r1.orchestrator_usage = usage(1000);
        let r2 = chained("r2", data.path(), RunState::Paused, "t", None);
        state.runs.insert("r1".into(), r1);
        state.runs.insert("r2".into(), adopted(r2, usage(1000)));
    }
    let handle = s.spawn(CancellationToken::new());
    nudge(&s);
    until("r1 resolves to r2", || s.is_live("r1")).await;
    s.post("r1".into(), usage(1300));
    let total = |id: &str| crate::lock(&s.state).runs[id].orchestrator_usage;
    until("r2 is credited", || total("r2") == usage(300)).await;
    assert_eq!(total("r1"), usage(1000));
    s.stop().await;
    let _ = handle.await;
}

/// The OTLP receiver as `metering::server::record` runs one request: once the live runs
/// moved, the ledger evicts every run that is not live; a run that is not live has its
/// points dropped; the rest are applied and the run's orchestrator total posted.
#[derive(Default)]
struct Receiver {
    ledger: crate::metering::OtlpLedger,
    generation: Option<u64>,
}

impl Receiver {
    /// A delta export of `input` tokens under `run`.
    fn export(&mut self, s: &RunService, run: &str, input: u64) {
        use crate::metering::{UsageKind, UsagePoint, otlp::ORCHESTRATOR};
        let generation = s.live_generation();
        if self.generation != Some(generation) {
            self.ledger.retain_runs(|r| s.is_live(r));
            self.generation = Some(generation);
        }
        if !s.is_live(run) {
            return;
        }
        let point = UsagePoint {
            run_id: run.into(),
            role: ORCHESTRATOR.into(),
            session_id: "s".into(),
            model: "m".into(),
            kind: UsageKind::Input,
            value: input,
            cumulative: false,
        };
        self.ledger.apply(&[point]);
        s.post(run.into(), self.ledger.total(run, ORCHESTRATOR));
    }
}

/// A chain driven through the real `engine::step` and `Metered::refresh_live`: runs
/// are added by hand, and `step` keeps the chain table as the daemon does.
struct Chained {
    s: Arc<RunService>,
    state: crate::run::engine::EngineState,
    rx: Receiver,
}

const CHAIN: &str = "o-c1";

impl Chained {
    fn new(data: &Path) -> Chained {
        let s = service(data);
        let mut state = crate::run::engine::EngineState::default();
        let first = chained("r1", data, RunState::Paused, "t", None);
        state.runs.insert("r1".into(), with_chain(first));
        let mut this = Chained {
            s,
            state,
            rx: Receiver::default(),
        };
        this.step(EventKind::Tick);
        assert!(this.state.chains.contains_key(CHAIN));
        this
    }

    /// One step, its usage drained into further steps, then the live runs refreshed.
    fn step(&mut self, kind: EventKind) {
        let event = crate::run::engine::Event { now: 5_000, kind };
        let state = std::mem::take(&mut self.state);
        self.state = crate::run::engine::step(state, event).0;
        for (run_id, usage) in self.s.metered.take() {
            let kind = EventKind::OrchestratorUsage { run_id, usage };
            let event = crate::run::engine::Event { now: 5_000, kind };
            let state = std::mem::take(&mut self.state);
            self.state = crate::run::engine::step(state, event).0;
        }
        self.s.metered.refresh_live(&self.state);
    }

    fn export(&mut self, run: &str, input: u64) {
        self.rx.export(&self.s, run, input);
        self.step(EventKind::Tick);
    }

    /// `prev` ends as `end`, and its chain goes idle.
    fn end(&mut self, prev: &str, end: RunState) {
        self.state.runs.get_mut(prev).unwrap().state = end;
        self.step(EventKind::Tick);
        let chain = &self.state.chains[CHAIN];
        assert_eq!(chain.state, crate::run::chain::ChainState::Idle);
    }

    /// `next` adopts `prev`'s window as `chains::adopt` does.
    fn adopt(&mut self, prev: &str, next: &str, data: &Path) {
        let p = &self.state.runs[prev];
        let o = p.orch.orchestrator.as_ref().unwrap();
        let mut at = p.orchestrator_usage.saturating_sub(p.orchestrator_base);
        at += o.usage_at_adopt.unwrap_or_default();
        let run = chained(next, data, RunState::Paused, "t", None);
        self.state
            .runs
            .insert(next.into(), adopted(with_chain(run), at));
        self.state.runs.get_mut(prev).unwrap().continued_by = Some(next.into());
        let chain = self.state.chains.get_mut(CHAIN).unwrap();
        chain.runs.push(next.into());
        chain.state = crate::run::chain::ChainState::Active;
        self.step(EventKind::Tick);
    }

    fn credited(&self, run: &str) -> u64 {
        self.state.runs[run].orchestrator_usage.input
    }
}

fn with_chain(mut run: Run) -> Run {
    run.chain = Some(CHAIN.into());
    run
}

/// Ruling T4b-1: an accepted or discarded run's idle chain keeps its session metered
/// (live, with its token, one orchestrator), so an export between the end and the
/// next goal's adoption does not evict its ledger total; the adopter is credited only
/// what was spent after the adoption.
#[test]
fn an_idle_chains_session_stays_metered_until_it_is_adopted() {
    for end in [RunState::Accepted, RunState::Discarded] {
        let data = tempfile::tempdir().unwrap();
        let mut c = Chained::new(data.path());
        c.export("r1", 1000);
        assert_eq!(c.credited("r1"), 1000);
        c.end("r1", end);
        assert!(
            c.s.is_live("r1"),
            "{end:?}: the idle chain's run is metered"
        );
        assert_eq!(c.s.token("r1").as_deref(), Some("t"));
        assert_eq!(c.s.live_orchestrators(), 1);
        c.export("r1", 0);
        assert_eq!(
            c.credited("r1"),
            1000,
            "{end:?}: an ended run keeps its usage"
        );
        c.adopt("r1", "r2", data.path());
        c.export("r1", 300);
        assert_eq!(c.credited("r2"), 300, "{end:?}");
        assert_eq!(c.s.live_orchestrators(), 1);
        // Once the window is gone the chain has ended, and nothing is metered.
        c.end("r2", RunState::Accepted);
        c.state.chains.get_mut(CHAIN).unwrap().ended = true;
        c.step(EventKind::Tick);
        assert!(!c.s.is_live("r1") && !c.s.is_live("r2"));
        assert_eq!(c.s.live_orchestrators(), 0);
    }
}

/// Review I2: every alias of an idle chain stays live, so in a chain of three the
/// session's key (the first run's) survives an export between the second run's accept
/// and the third run's adoption.
#[test]
fn every_alias_of_an_idle_chain_stays_metered() {
    let data = tempfile::tempdir().unwrap();
    let mut c = Chained::new(data.path());
    c.export("r1", 1000);
    c.end("r1", RunState::Accepted);
    c.adopt("r1", "r2", data.path());
    c.export("r1", 500);
    assert_eq!(c.credited("r2"), 500);
    c.end("r2", RunState::Accepted);
    assert!(c.s.is_live("r1") && c.s.is_live("r2"));
    c.export("r1", 0);
    c.adopt("r2", "r3", data.path());
    c.export("r1", 300);
    assert_eq!(c.credited("r3"), 300);
    assert_eq!((c.credited("r1"), c.credited("r2")), (1000, 500));
}

/// Review m1: once the adopting run's session is its own again (a lost adoption, a
/// restart), the earlier run's id no longer resolves to it, so a window still posting
/// under that id cannot overwrite its credit.
#[test]
fn an_alias_resolves_only_while_the_session_is_adopted() {
    let data = tempfile::tempdir().unwrap();
    let mut c = Chained::new(data.path());
    c.export("r1", 1000);
    c.end("r1", RunState::Accepted);
    c.adopt("r1", "r2", data.path());
    assert!(c.s.is_live("r1"));
    let o = c
        .state
        .runs
        .get_mut("r2")
        .unwrap()
        .orch
        .orchestrator
        .as_mut();
    o.unwrap().usage_at_adopt = None;
    c.step(EventKind::Tick);
    assert!(!c.s.is_live("r1"));
    assert!(c.s.is_live("r2"));
}
