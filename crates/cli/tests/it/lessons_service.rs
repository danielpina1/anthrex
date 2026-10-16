//! Task M9.15.10 (decision 12; ruling C6): the distill worker against real `fake-agent`
//! decider processes. Each fixture's decider program is `support::decider`'s wrapper,
//! so the worker can reach nothing but `fake-agent`, and the runtimes' own commands
//! are paths that do not exist. The worker runs in the background: every test waits
//! for its lines with a deadline loop on the store, never one read.

use crate::support;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use daemon::lessons::LESSONS_FILE;
use daemon::lessons::fold::fold;
use daemon::lessons::service::{LessonService, ServiceParts, ServiceTiming};
use daemon::lessons::store::{LessonLocks, read_store};
use daemon::manager::{ManagerConfig, WindowManager};
use daemon::run::driver::RunService;
use daemon::server::GitWiring;
use proto::{
    DeciderMode, DistillCause, HistoryLine, LESSONS_VERSION, LessonLine, LessonStatus, QueuedLine,
    RunReply, RunRequest,
};
use serde_json::{Value, json};
use support::decider::{Fixture, KILL_GRACE, SPAWN_SLACK, TIMEOUT_SECS, assert_gone, running};
use support::run_harness::REQUEST_WAIT;
use support::runtime;

/// One call's deadline in a test: the routing probe (`INSTALLED_PROBE_TIMEOUT`, 5 s),
/// the call's own timeout (`TIMEOUT_SECS`, 5 s), the wait for its exit and the kill's
/// grace (2 s each) and a spawn on a loaded machine (`SPAWN_SLACK`, 5 s): 19 s. The
/// worker's file reads and appends are milliseconds.
fn call_wait() -> Duration {
    daemon::run::driver::INSTALLED_PROBE_TIMEOUT
        + Duration::from_secs(TIMEOUT_SECS)
        + daemon::decider::call::EXIT_WAIT
        + KILL_GRACE
        + SPAWN_SLACK
}

/// The queued entry every test serves.
const QUEUED: &str = "fix-login-3f2a/t1/distill";

fn repo_dir(fx: &Fixture) -> PathBuf {
    let repo = fx.root().join("data").join("repos").join("app-0001");
    std::fs::create_dir_all(&repo).unwrap();
    repo
}

fn queue(repo: &Path) {
    let line = LessonLine::Queued(QueuedLine {
        v: LESSONS_VERSION,
        record_id: QUEUED.into(),
        at: 1_800_000_000,
        run_id: "fix-login-3f2a".into(),
        task_id: Some("t1".into()),
        cause: DistillCause::TaskRecovered,
        dossier: "## outcome\ncause task_recovered\ntask t1: Add a retry".into(),
        fps: vec!["0123456789abcdef".into()],
    });
    daemon::run::history_io::append_json(&repo.join(LESSONS_FILE), &line).unwrap();
}

/// A worker on `fx`'s decider context in `mode`, its retries `retry` apart.
fn start(fx: &Fixture, mode: DeciderMode, retry: Duration) -> Arc<LessonService> {
    let locks = Arc::new(LessonLocks::default());
    start_with(fx, mode, retry, (locks, config::Lessons::default()))
}

fn start_with(
    fx: &Fixture,
    mode: DeciderMode,
    retry: Duration,
    (locks, config): (Arc<LessonLocks>, config::Lessons),
) -> Arc<LessonService> {
    let parts = ServiceParts {
        deciders: fx.context(mode),
        data_dir: fx.root().join("data"),
        config,
        locks,
        braked: None,
    };
    let timing = ServiceTiming {
        bound: daemon::lessons::service::DISTILL_BOUND,
        retry: [retry, retry],
        ..ServiceTiming::default()
    };
    LessonService::start_with(parts, timing)
}

fn lines(repo: &Path) -> Vec<LessonLine> {
    let (lines, problems) = read_store(&repo.join(LESSONS_FILE));
    assert!(problems.is_empty(), "{problems:?}");
    lines
}

fn closed(repo: &Path) -> bool {
    lines(repo)
        .iter()
        .any(|l| matches!(l, LessonLine::Distilled(_) | LessonLine::GaveUp(_)))
}

/// Waits (a deadline loop, yielding to the worker) until the entry is closed.
async fn wait_closed(repo: &Path, within: Duration) {
    let deadline = Instant::now() + within;
    while !closed(repo) {
        assert!(
            Instant::now() < deadline,
            "the entry was not closed within {within:?}: {:#?}",
            lines(repo)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn attempts(repo: &Path) -> Vec<proto::AttemptLine> {
    (lines(repo).into_iter())
        .filter_map(|l| match l {
            LessonLine::Attempt(a) => Some(a),
            _ => None,
        })
        .collect()
}

fn distill_calls(fx: &Fixture) -> Vec<Value> {
    (fx.calls().into_iter())
        .filter(|c| c["kind"] == "distill")
        .collect()
}

fn two_items() -> Value {
    json!({"answer": {"items": [
        {"layer": "episodic", "title": "The retry test needs the fixture server",
         "body": "Start it before the check.", "files": ["tests/retry/**"]},
        {"layer": "semantic", "title": "Fetch goes through the client module",
         "body": "Never call reqwest directly.", "files": ["src/client.rs"]},
    ]}, "usage": {"input": 10, "output": 20, "cache_read": 0, "cache_write": 0}})
}

#[test]
fn a_queued_entry_is_distilled_once() {
    let fx = Fixture::new();
    fx.script("distill", 1, two_items());
    let repo = repo_dir(&fx);
    runtime().block_on(async {
        let service = start(&fx, DeciderMode::Claude, Duration::ZERO);
        queue(&repo);
        service.wake(repo.clone());
        wait_closed(&repo, call_wait()).await;
        // Woken again: the closed entry is not asked twice.
        let before = service.passes();
        service.wake(repo.clone());
        let deadline = Instant::now() + call_wait();
        while service.passes() <= before {
            assert!(Instant::now() < deadline, "no pass after the wake");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        service.stop();
    });
    let all = lines(&repo);
    let attempts = attempts(&repo);
    assert_eq!(attempts.len(), 1, "{all:#?}");
    assert_eq!(attempts[0].record_id, format!("{QUEUED}/attempt/1"));
    assert_eq!((attempts[0].n, attempts[0].error.as_deref()), (1, None));
    let distilled: Vec<_> = (all.iter())
        .filter_map(|l| match l {
            LessonLine::Distilled(d) => Some(d),
            _ => None,
        })
        .collect();
    assert_eq!(distilled.len(), 1, "{all:#?}");
    assert_eq!(distilled[0].record_id, format!("{QUEUED}/distilled"));
    assert_eq!((distilled[0].items.len(), distilled[0].dropped), (2, 0));
    let store = fold(&all);
    let statuses: Vec<LessonStatus> = store.lessons.values().map(|l| l.status).collect();
    assert!(statuses.contains(&LessonStatus::OnTrial), "{statuses:?}");
    assert!(statuses.contains(&LessonStatus::Proposed), "{statuses:?}");
    assert!(!store.queued[QUEUED].open);
    // One call, and its prompt is decision 14's with the entry's dossier.
    let calls = distill_calls(&fx);
    assert_eq!(calls.len(), 1, "{calls:#?}");
    let prompt = calls[0]["prompt"].as_str().unwrap();
    assert!(
        prompt.starts_with("[anthrex decider] distill v1\n"),
        "{prompt}"
    );
    assert!(
        prompt.ends_with(
            "Cause: task_recovered\n--- DOSSIER ---\n## outcome\ncause task_recovered\ntask t1: Add a retry\n--- END OF DOSSIER ---"
        ),
        "{prompt}"
    );
}

#[test]
fn restart_resumes_unfinished_queued() {
    let fx = Fixture::new();
    fx.script("distill", 1, two_items());
    let repo = repo_dir(&fx);
    // Written before the worker exists, and never woken for: the start scan finds it.
    queue(&repo);
    runtime().block_on(async {
        let service = start(&fx, DeciderMode::Claude, Duration::ZERO);
        wait_closed(&repo, call_wait()).await;
        service.stop();
    });
    assert_eq!(attempts(&repo).len(), 1);
    assert!(
        (lines(&repo).iter()).any(|l| l.record_id() == format!("{QUEUED}/distilled")),
        "{:#?}",
        lines(&repo)
    );
}

#[test]
fn distill_failure_three_attempts_then_gave_up() {
    let fx = Fixture::new();
    for n in 1..=3 {
        fx.script("distill", n, json!({"fail_turn": "rate_limit"}));
    }
    let repo = repo_dir(&fx);
    queue(&repo);
    runtime().block_on(async {
        let service = start(&fx, DeciderMode::Claude, Duration::ZERO);
        wait_closed(&repo, call_wait() * 3).await;
        service.stop();
    });
    let attempts = attempts(&repo);
    assert_eq!(
        attempts.iter().map(|a| a.n).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "{attempts:#?}"
    );
    let last = attempts[2]
        .error
        .clone()
        .expect("a failed attempt's reason");
    assert!(last.starts_with("the decider's turn failed"), "{last}");
    let gave_up: Vec<_> = (lines(&repo).into_iter())
        .filter_map(|l| match l {
            LessonLine::GaveUp(g) => Some(g),
            _ => None,
        })
        .collect();
    assert_eq!(gave_up.len(), 1);
    assert_eq!(gave_up[0].record_id, format!("{QUEUED}/gave_up"));
    assert_eq!(gave_up[0].reason, last);
    assert_eq!(distill_calls(&fx).len(), 3);
}

#[test]
fn deciders_off_gives_up_at_once() {
    let fx = Fixture::new();
    fx.script("distill", 1, two_items());
    let repo = repo_dir(&fx);
    queue(&repo);
    runtime().block_on(async {
        let service = start(&fx, DeciderMode::Off, Duration::ZERO);
        // No call is made: the bound is a pass, not a call.
        wait_closed(&repo, SPAWN_SLACK).await;
        service.stop();
    });
    let all = lines(&repo);
    assert!(attempts(&repo).is_empty(), "{all:#?}");
    match all.last() {
        Some(LessonLine::GaveUp(g)) => assert_eq!(g.reason, "deciders are off"),
        other => panic!("not a gave_up: {other:?}"),
    }
    assert!(fx.calls().is_empty(), "{:#?}", fx.calls());
}

/// The review's config gate: an entry queued before `[lessons] distill = false` (or
/// `enabled = false`) is neither called for nor closed by a worker started with that
/// config, at its start scan or on a wake, with the deciders on or off; it stays
/// queued, and a later worker with distillation back on distills it.
#[test]
fn a_disabled_config_keeps_queued_entries_uncalled() {
    let off = [
        (
            "distill = false",
            config::Lessons {
                distill: false,
                ..config::Lessons::default()
            },
        ),
        (
            "enabled = false",
            config::Lessons {
                enabled: false,
                ..config::Lessons::default()
            },
        ),
    ];
    for (name, lessons) in off {
        for mode in [DeciderMode::Claude, DeciderMode::Off] {
            let fx = Fixture::new();
            fx.script("distill", 1, two_items());
            let repo = repo_dir(&fx);
            // Queued before the restart: the start scan finds it.
            queue(&repo);
            runtime().block_on(async {
                let locks = Arc::new(LessonLocks::default());
                let service = start_with(&fx, mode, Duration::ZERO, (locks, lessons.clone()));
                // The start scan's pass, then a wake's: both leave the entry alone.
                let deadline = Instant::now() + call_wait();
                while service.passes() < 1 {
                    assert!(Instant::now() < deadline, "{name}: no first pass");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                service.wake(repo.clone());
                while service.passes() < 2 {
                    assert!(Instant::now() < deadline, "{name}: no pass after the wake");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                service.stop();
            });
            let all = lines(&repo);
            assert!(
                distill_calls(&fx).is_empty(),
                "{name}, {mode:?}: {:#?}",
                fx.calls()
            );
            assert_eq!(
                all.len(),
                1,
                "{name}, {mode:?}: the entry was touched: {all:#?}"
            );
            assert!(matches!(all[0], LessonLine::Queued(_)), "{all:#?}");
        }
        // Kept, not dropped: back on, the next worker distills it.
        let fx = Fixture::new();
        fx.script("distill", 1, two_items());
        let repo = repo_dir(&fx);
        queue(&repo);
        runtime().block_on(async {
            let locks = Arc::new(LessonLocks::default());
            let service = start_with(&fx, DeciderMode::Claude, Duration::ZERO, (locks, lessons));
            let deadline = Instant::now() + call_wait();
            while service.passes() < 1 {
                assert!(Instant::now() < deadline, "{name}: no first pass");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            service.stop();
        });
        assert!(!closed(&repo), "{name}: {:#?}", lines(&repo));
        runtime().block_on(async {
            let service = start(&fx, DeciderMode::Claude, Duration::ZERO);
            wait_closed(&repo, call_wait()).await;
            service.stop();
        });
        assert!(
            (lines(&repo).iter()).any(|l| l.record_id() == format!("{QUEUED}/distilled")),
            "{name}: {:#?}",
            lines(&repo)
        );
        assert_eq!(distill_calls(&fx).len(), 1, "{name}");
    }
}

#[test]
fn distill_route_is_recorded() {
    let fx = Fixture::new();
    fx.script("distill", 1, two_items());
    let repo = repo_dir(&fx);
    queue(&repo);
    runtime().block_on(async {
        let service = start(&fx, DeciderMode::Claude, Duration::ZERO);
        wait_closed(&repo, call_wait()).await;
        service.stop();
    });
    // The record is appended before the attempt line, which closed the wait.
    let (history, problems) =
        daemon::run::history_io::read_history(&repo.join(daemon::run::engine::HISTORY_FILE));
    assert!(problems.is_empty(), "{problems:?}");
    let records: Vec<_> = (history.into_iter())
        .filter_map(|l| match l {
            HistoryLine::RoleRoute(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(records.len(), 1, "{records:#?}");
    let record = serde_json::to_value(&records[0]).unwrap();
    assert_eq!(record["record_id"], format!("distill/{QUEUED}/1"));
    assert_eq!(record["run_id"], Value::Null);
    assert_eq!(record["role"], "decider");
    assert_eq!(record["task_id"], "t1");
    assert_eq!(record["outcome"], "completed");
}

/// Review focus 5 (ruling C6): while a call hangs, the window manager's lock, the run
/// service's engine and the store's lock are all free; the hung call's process is gone
/// once the worker stops.
#[test]
fn distill_runs_off_lock() {
    let fx = Fixture::new();
    fx.script("distill", 1, json!({"hang": true}));
    let repo = repo_dir(&fx);
    queue(&repo);
    let marker = fx.marker();
    runtime().block_on(async {
        let mut config = ManagerConfig::for_tests(fx.root().join("d.sock"), "/bin/sh".into());
        config.claude_bin = support::decider::NO_CLAUDE_BIN.into();
        config.codex_bin = support::decider::NO_CODEX_BIN.into();
        let (manager, _events) = WindowManager::new(config);
        let git = GitWiring::new(config::Git {
            enabled: false,
            ..config::Git::default()
        });
        let data = fx.root().join("data");
        let runs = RunService::for_manager(&manager, data, git.registry.clone());
        let shutdown = tokio_util::sync::CancellationToken::new();
        runs.spawn(shutdown.clone());
        let locks = runs.lesson_locks();
        let parts = (locks.clone(), config::Lessons::default());
        let service = start_with(&fx, DeciderMode::Claude, Duration::ZERO, parts);
        // The decider's process is up (a deadline loop that yields to the worker).
        let deadline = Instant::now() + call_wait();
        while running(&marker) == 0 {
            assert!(Instant::now() < deadline, "the hung decider never started");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The call is in flight: each lock is taken and released at once.
        let started = Instant::now();
        assert!(manager.list().is_empty());
        let store = tokio::time::timeout(REQUEST_WAIT, locks.lock(&repo)).await;
        drop(store.expect("the store's lock is free during the call"));
        let reply = tokio::time::timeout(REQUEST_WAIT, runs.request(RunRequest::List)).await;
        assert!(
            matches!(reply, Ok(RunReply::Snapshot(_))),
            "the engine answered during the call: {reply:?}"
        );
        assert!(started.elapsed() < REQUEST_WAIT);
        assert!(!closed(&repo), "the call is still in flight");
        service.stop();
        shutdown.cancel();
    });
    // Stopping the worker dropped the call, which killed its process.
    assert_gone(&marker);
    assert!(attempts(&repo).is_empty());
}
