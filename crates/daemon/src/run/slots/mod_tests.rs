//! Milestone 9.1 task M9.1.8: the async scheduler, its grant environment, the default
//! slot count, and the worker caps in a real launch spec.

use std::time::{Duration, Instant};

use proto::{Effort, Route, Runtime, Strength};

use super::*;
use crate::run::model::Run;
use crate::run::orch::launch::{orchestrator_role, planner_spec};
use crate::run::orch::{EpicRecord, PlannerPhase};
use crate::run::role_launch::{reviewer_spec, worker_spec};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// How long a deadline loop below may spin. Nothing in the code under test has a
/// timeout: a grant is made in the same call that releases the slot, so the only cost
/// is the current-thread runtime polling a spawned task a few times (standing rule 1:
/// the bound has no constant to exceed; it only turns a hang into a failure).
const DEADLINE: Duration = Duration::from_secs(10);

fn req(priority: Priority, want: Want) -> SlotRequest {
    SlotRequest {
        priority,
        critical: false,
        want,
        exclusive: false,
        label: "test".into(),
    }
}

fn value<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// Polls the runtime until `task` finishes, within [`DEADLINE`].
async fn finished<T>(task: &tokio::task::JoinHandle<T>) {
    let deadline = Instant::now() + DEADLINE;
    while !task.is_finished() {
        assert!(Instant::now() < deadline, "the waiter was never granted");
        tokio::task::yield_now().await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn grant_env_carries_the_slot_variables_and_load() {
    let sched = TestScheduler::new(4);
    let grant = sched.acquire(req(Priority::Gate, Want::Half)).await;
    assert_eq!(grant.slots(), 2);
    let env = grant.env();
    let names: Vec<&str> = env.iter().map(|(key, _)| key.as_str()).collect();
    assert_eq!(
        names,
        [
            "CARGO_BUILD_JOBS",
            "NEXTEST_TEST_THREADS",
            "RUST_TEST_THREADS",
            "ANTHREX_TEST_SLOTS",
            "ANTHREX_TEST_LOAD",
        ]
    );
    for name in SLOT_VARS {
        assert_eq!(value(&env, name), Some("2"), "{name}");
    }
    assert_eq!(value(&env, LOAD_VAR), Some("0.5"));

    // The load is the book's right after this grant: 2 held, then 3 of 4.
    let one = sched.acquire(req(Priority::Gate, Want::One)).await;
    assert_eq!(value(&one.env(), LOAD_VAR), Some("0.8"));
    assert_eq!(value(&one.env(), "ANTHREX_TEST_SLOTS"), Some("1"));
    // Unchanged by later grants: what the step started under.
    assert_eq!(value(&grant.env(), LOAD_VAR), Some("0.5"));
}

#[tokio::test(flavor = "current_thread")]
async fn acquire_waits_without_blocking_the_runtime() {
    let sched = TestScheduler::new(1);
    let first = sched.acquire(req(Priority::Gate, Want::One)).await;
    let waiter = sched.clone();
    let second =
        tokio::spawn(async move { waiter.acquire(req(Priority::Gate, Want::One)).await.slots() });
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    assert!(!second.is_finished(), "the only slot is held");
    // The runtime still runs other work while the second acquire waits.
    let other = tokio::time::timeout(DEADLINE, async {}).await;
    assert!(other.is_ok());
    drop(first);
    finished(&second).await;
    assert_eq!(second.await.unwrap(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn a_dropped_acquire_gives_up_its_place() {
    let sched = TestScheduler::new(2);
    let held = sched.acquire(req(Priority::Gate, Want::All)).await;
    let waiter = sched.clone();
    // An exclusive request at the head of the queue, abandoned while it waits.
    let blocked = tokio::spawn(async move {
        let exclusive = SlotRequest {
            exclusive: true,
            ..req(Priority::Candidate, Want::All)
        };
        waiter.acquire(exclusive).await.slots()
    });
    tokio::task::yield_now().await;
    blocked.abort();
    assert!(blocked.await.unwrap_err().is_cancelled());
    let waiter = sched.clone();
    let later =
        tokio::spawn(async move { waiter.acquire(req(Priority::Gate, Want::One)).await.slots() });
    drop(held);
    finished(&later).await;
    assert_eq!(
        later.await.unwrap(),
        1,
        "not held back by the abandoned wait"
    );
    assert_eq!(crate::lock(&sched.shared.inner).book.held(), 0);
}

#[test]
fn default_slots_is_cores_minus_two_at_least_one() {
    assert_eq!(slots_for(Some(16)), 14);
    assert_eq!(slots_for(Some(3)), 1);
    assert_eq!(slots_for(Some(2)), 1);
    assert_eq!(slots_for(Some(1)), 1);
    assert_eq!(slots_for(None), 1);
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    assert_eq!(default_slots(), slots_for(Some(cores)));
}

fn run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.test_slots = 8;
    run.limits.max_writers = 3;
    run
}

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

fn caps_in(env: &[(String, String)]) -> Vec<String> {
    SLOT_VARS
        .iter()
        .filter_map(|name| value(env, name).map(|v| format!("{name}={v}")))
        .collect()
}

#[test]
fn worker_env_carries_the_caps() {
    assert_eq!(
        worker_caps(8, 3),
        slot_vars(2),
        "max(1, 8/3) for each of the four"
    );
    assert_eq!(worker_caps(2, 4), slot_vars(1));

    let run = run();
    let worker = worker_spec(&run, &run.tasks[0]);
    assert_eq!(
        caps_in(&worker.env),
        SLOT_VARS.map(|name| format!("{name}=2")),
        "{:?}",
        worker.env
    );
    // Before its `TMPDIR`, which stays last (decision 27).
    assert_eq!(
        worker.env.last().map(|(key, _)| key.as_str()),
        Some("TMPDIR")
    );
    // A profile value of the same name does not survive beside the cap.
    let mut profiled = run.clone();
    profiled
        .profile
        .env
        .insert("CARGO_BUILD_JOBS".into(), "64".into());
    let env = worker_spec(&profiled, &profiled.tasks[0]).env;
    let jobs: Vec<_> = env
        .iter()
        .filter(|(k, _)| k == "CARGO_BUILD_JOBS")
        .collect();
    assert_eq!(jobs, [&("CARGO_BUILD_JOBS".to_string(), "2".to_string())]);

    // No other role runs tests; none gets the caps.
    let task = &run.tasks[0];
    for runtime in [Runtime::Claude, Runtime::Codex] {
        let reviewer = reviewer_spec(&run, task, &route(runtime));
        assert_eq!(caps_in(&reviewer.env), Vec::<String>::new(), "reviewer");
    }
    let epic = EpicRecord::new("auth", PlannerPhase::Planning);
    let planner = planner_spec(&run, &epic, 1);
    assert_eq!(
        caps_in(&planner.headless.env),
        Vec::<String>::new(),
        "planner"
    );
    let orchestrator = orchestrator_role(&run, &route(Runtime::Claude));
    assert_eq!(
        caps_in(&orchestrator.env),
        Vec::<String>::new(),
        "orchestrator"
    );
}
