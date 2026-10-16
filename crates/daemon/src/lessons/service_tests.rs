//! Task M9.15.10: the distill worker's scheduling (ruling C6): the decider call is the
//! injected seam and the clock is injected, so these tests start no process and sleep
//! for no retry or cap. The `fake-agent`-backed tests are in the cli crate
//! (`tests/it/lessons_service.rs`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use proto::{
    AttemptLine, DeciderMode, DeciderSource, DistillCause, DistilledLine, LESSONS_VERSION,
    LessonDraft, LessonLayer, LessonLine, QueuedLine,
};

use super::{DecideFn, LessonService, ServiceParts, ServiceTiming};
use crate::decider::{DeciderAnswer, DeciderContext, DeciderKind, Decision};
use crate::lessons::LESSONS_FILE;
use crate::lessons::store::{LessonLocks, read_store};

/// A pass reads one small store per repository on `spawn_blocking`; ten seconds is a
/// deadline for a loaded machine, not a cost.
const PASS_WAIT: Duration = Duration::from_secs(10);
/// The clock every test starts at.
const T: u64 = 1_800_000_000;

struct Rig {
    _dir: tempfile::TempDir,
    data: PathBuf,
    clock: Arc<AtomicU64>,
    calls: Arc<AtomicUsize>,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        Rig {
            _dir: dir,
            data,
            clock: Arc::new(AtomicU64::new(T)),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn repo(&self, name: &str) -> PathBuf {
        let repo = self.data.join("repos").join(name);
        std::fs::create_dir_all(&repo).unwrap();
        repo
    }

    /// A worker whose calls answer one episodic lesson, and are counted.
    fn start(&self, max_per_day: u32, mode: DeciderMode) -> Arc<LessonService> {
        self.start_braked(max_per_day, mode, None)
    }

    /// [`Rig::start`] with ruling C28's probe, looked at again every second.
    fn start_braked(
        &self,
        max_per_day: u32,
        mode: DeciderMode,
        braked: Option<crate::run::driver::BrakeProbe>,
    ) -> Arc<LessonService> {
        let mut cfg = config::Orchestrator::default();
        cfg.deciders.mode = mode;
        let mut manager =
            crate::manager::ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
        manager.decider_bin = None;
        let live = crate::live_config::LiveSettings::defaults_of(cfg);
        let deciders = DeciderContext::new(live, &manager, &self.data);
        let config = config::Lessons {
            max_distills_per_day: max_per_day,
            ..config::Lessons::default()
        };
        let clock = self.clock.clone();
        let timing = ServiceTiming {
            bound: Duration::from_secs(5),
            retry: [Duration::from_secs(60), Duration::from_secs(300)],
            now: Arc::new(move || clock.load(Ordering::SeqCst)),
            brake_recheck: Duration::from_secs(1),
        };
        let calls = self.calls.clone();
        let decide: DecideFn = Arc::new(move |_request, _project| {
            calls.fetch_add(1, Ordering::SeqCst);
            Box::pin(async move { (None, answered()) })
        });
        let parts = ServiceParts {
            deciders,
            data_dir: self.data.clone(),
            config,
            locks: Arc::new(LessonLocks::default()),
            braked,
        };
        LessonService::start_seamed(parts, timing, decide)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn set_clock(&self, at: u64) {
        self.clock.store(at, Ordering::SeqCst);
    }
}

fn answered() -> Decision {
    Decision {
        kind: DeciderKind::Distill,
        answer: DeciderAnswer::Distill {
            items: vec![LessonDraft {
                layer: LessonLayer::Episodic,
                title: "The fixture must exist".into(),
                body: "Create it first.".into(),
                files: vec![],
            }],
            dropped: 0,
        },
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: None,
        secs: 0,
    }
}

fn queued(id: &str, at: u64) -> LessonLine {
    LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: format!("{id}/t1/distill"),
        at,
        run_id: id.into(),
        task_id: Some("t1".into()),
        cause: DistillCause::TaskRecovered,
        dossier: "## outcome\nrecovered".into(),
        fps: vec![],
    })
}

fn attempt(id: &str, n: u8, at: u64, error: Option<&str>) -> LessonLine {
    let queued = format!("{id}/t1/distill");
    LessonLine::Attempt(AttemptLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/attempt/{n}"),
        at,
        queued,
        n,
        error: error.map(String::from),
        usage: None,
    })
}

fn distilled(id: &str, at: u64) -> LessonLine {
    let queued = format!("{id}/t1/distill");
    LessonLine::Distilled(DistilledLine {
        v: LESSONS_VERSION,
        record_id: format!("{queued}/distilled"),
        at,
        queued,
        items: vec![],
        dropped: 0,
        usage: None,
    })
}

fn write(repo: &Path, lines: &[LessonLine]) {
    for line in lines {
        crate::run::history_io::append_json(&repo.join(LESSONS_FILE), line).unwrap();
    }
}

fn lines(repo: &Path) -> Vec<LessonLine> {
    read_store(&repo.join(LESSONS_FILE)).0
}

/// Whether `repo`'s entry of run `id` is closed by a `distilled` line.
fn is_distilled(repo: &Path, id: &str) -> bool {
    let want = format!("{id}/t1/distill/distilled");
    lines(repo).iter().any(|l| l.record_id() == want)
}

/// Waits (a deadline loop) until the worker has finished `n` passes.
async fn passes(service: &LessonService, n: u64) {
    let deadline = Instant::now() + PASS_WAIT;
    while service.passes() < n {
        assert!(
            Instant::now() < deadline,
            "no pass {n} within {PASS_WAIT:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Waits (a deadline loop) until `done` holds.
async fn until(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + PASS_WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "{what} not within {PASS_WAIT:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn daily_cap_waits() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    // One attempt 100 s ago (a closed entry's) fills a cap of one.
    let at = T - 100;
    write(
        &repo,
        &[
            queued("old", at),
            attempt("old", 1, at, None),
            distilled("old", at),
            queued("new", T),
        ],
    );
    let service = rig.start(1, DeciderMode::Claude);
    passes(&service, 1).await;
    assert_eq!(rig.calls(), 0);
    assert!(!is_distilled(&repo, "new"));
    // A second before the attempt ages out: still capped.
    rig.set_clock(at + super::DAY_SECS - 1);
    let before = service.passes();
    service.wake(repo.clone());
    passes(&service, before + 1).await;
    assert_eq!(rig.calls(), 0);
    // The window passed: one call, and the entry is distilled.
    rig.set_clock(at + super::DAY_SECS);
    service.wake(repo.clone());
    until("the distilled line", || is_distilled(&repo, "new")).await;
    assert_eq!(rig.calls(), 1);
    service.stop();
}

/// Ruling C19: a capped repository never holds another's entries.
#[tokio::test]
async fn a_capped_repository_never_holds_another() {
    let rig = Rig::new();
    let (capped, free) = (rig.repo("a"), rig.repo("b"));
    let at = T - 10;
    write(
        &capped,
        &[
            queued("old", at),
            attempt("old", 1, at, None),
            distilled("old", at),
            queued("new", T - 5),
        ],
    );
    write(&free, &[queued("other", T)]);
    let service = rig.start(1, DeciderMode::Claude);
    until("b's distilled line", || is_distilled(&free, "other")).await;
    passes(&service, 1).await;
    assert_eq!(rig.calls(), 1);
    assert!(!is_distilled(&capped, "new"));
    service.stop();
}

/// A failed attempt's retry waits `DISTILL_RETRY_SECS` from its line, across a
/// restart: the clock decides, not the worker's memory.
#[tokio::test]
async fn a_restart_keeps_the_retry_schedule() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(
        &repo,
        &[
            queued("r", T),
            attempt("r", 1, T, Some("the decider timed out")),
        ],
    );
    rig.set_clock(T + 59);
    let service = rig.start(20, DeciderMode::Claude);
    passes(&service, 1).await;
    assert_eq!(rig.calls(), 0);
    rig.set_clock(T + 60);
    service.wake(repo.clone());
    until("the distilled line", || is_distilled(&repo, "r")).await;
    assert_eq!(rig.calls(), 1);
    let n2 = lines(&repo)
        .into_iter()
        .any(|l| matches!(l, LessonLine::Attempt(a) if a.n == 2 && a.error.is_none()));
    assert!(n2, "{:#?}", lines(&repo));
    service.stop();
}

/// Three attempts written and no `gave_up` (a stop between them): closed with the last
/// reason, with no fourth call.
#[tokio::test]
async fn a_third_failed_attempt_is_closed_without_another_call() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(
        &repo,
        &[
            queued("r", T),
            attempt("r", 1, T, Some("first")),
            attempt("r", 2, T, Some("second")),
            attempt(
                "r",
                3,
                T,
                Some("the decider exited before answering (code 2)"),
            ),
        ],
    );
    let service = rig.start(20, DeciderMode::Claude);
    let gave_up = || {
        lines(&repo).into_iter().find_map(|l| match l {
            LessonLine::GaveUp(g) => Some(g.reason),
            _ => None,
        })
    };
    until("the gave_up line", || gave_up().is_some()).await;
    assert_eq!(
        gave_up().as_deref(),
        Some("the decider exited before answering (code 2)")
    );
    passes(&service, 1).await;
    assert_eq!(rig.calls(), 0);
    service.stop();
}

/// Decision 31: `max_distills_per_day = 0` stops the worker from calling; the entry
/// stays open.
#[tokio::test]
async fn a_cap_of_zero_never_calls() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("r", T)]);
    let service = rig.start(0, DeciderMode::Claude);
    passes(&service, 1).await;
    assert_eq!(rig.calls(), 0);
    assert_eq!(lines(&repo).len(), 1);
    service.stop();
}

/// Ruling C29: with the deciders off, every open entry is closed at once, without a
/// call or an attempt line.
#[tokio::test]
async fn deciders_off_close_every_open_entry_without_a_call() {
    let rig = Rig::new();
    let repo = rig.repo("a");
    write(&repo, &[queued("a", T), queued("b", T + 1)]);
    let service = rig.start(20, DeciderMode::Off);
    let closed = || {
        (lines(&repo).into_iter())
            .filter(|l| matches!(l, LessonLine::GaveUp(g) if g.reason == "deciders are off"))
            .count()
    };
    until("two gave_up lines", || closed() == 2).await;
    assert_eq!(rig.calls(), 0);
    assert!(
        !(lines(&repo).iter()).any(|l| matches!(l, LessonLine::Attempt(_))),
        "{:#?}",
        lines(&repo)
    );
    service.stop();
}

#[cfg(test)]
#[path = "service_brake_tests.rs"]
mod brake;
