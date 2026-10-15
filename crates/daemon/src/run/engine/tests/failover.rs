//! Milestone 9.16 task M9.16.6: cross-runtime failover (U17, decisions 36–45), the
//! writers' half. Every run's role table is built here with the rows named (no test
//! depends on the built-ins); every brake is switched on by its own `BrakesConfig`
//! (ruling C3); `now` moves by hand. The readers, the reviewer rule, the history record
//! and the orchestrator are `failover_readers.rs`.

use proto::brakes::{DispatchHold, QuotaWindow};
use proto::models::{ModelRef, Role, RoleChoice};
use proto::{Effort, Route, Runtime, TaskState};

use super::brakes::{attention, brake_on, claude_held, logged, reading, trip_claude};
use super::control::resume;
use super::control_restore::restart;
use super::fixture::*;
use super::gates::only_op;
use super::pair::{PAIRED, claim_red, red_check};
use crate::run::engine::brakes::HOLD_TEXT;
use crate::run::engine::failover::{self, Admit, Starter};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::model_roles::RunModels;

pub(super) const SONNET: &str = "claude:claude-sonnet-5";
pub(super) const OPUS: &str = "claude:claude-opus-5-5";
pub(super) const HAIKU: &str = "claude:claude-haiku-4-5";
pub(super) const SOL: &str = "codex:gpt-6-sol";

/// `model` (`<runtime>:<id>`) at its default effort, as a route.
pub(super) fn route(model: &str) -> Route {
    RunModels::route_of(&ModelRef::parse(model).expect("a model"), None)
}

fn choice(model: &str, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: ModelRef::parse(model).expect("a model"),
        effort: None,
        fallback: fallback.map(|f| ModelRef::parse(f).expect("a model")),
    }
}

/// The role table every test starts from, `over` replacing rows: Claude everywhere,
/// `implementer.medium` (Sonnet) failing over to Codex's `gpt-6-sol`.
pub(super) fn rows(over: &[(Role, &str, Option<&str>)]) -> config::Orchestrator {
    let mut config = config::Orchestrator::default();
    let base: [(Role, &str, Option<&str>); 8] = [
        (Role::Orchestrator, OPUS, None),
        (Role::Planner, OPUS, None),
        (Role::ImplementerSmall, SONNET, None),
        (Role::ImplementerMedium, SONNET, Some(SOL)),
        (Role::ImplementerHub, SONNET, None),
        (Role::TestWriter, SONNET, None),
        (Role::Reviewer, OPUS, None),
        (Role::Research, HAIKU, None),
    ];
    for (role, model, fallback) in base.iter().chain(over) {
        config.roles.rows.insert(*role, choice(model, *fallback));
    }
    config
}

/// The brake on, then `runtime`'s 7-day window at 96 %, resetting in three hours.
pub(super) fn trip(fx: &mut Fixture, runtime: Runtime) -> u64 {
    let until = fx.now + 3 * 3600;
    fx.next(brake_on());
    fx.next(reading(runtime, QuotaWindow::SevenDay, 960, until));
    until
}

/// Decision 42's run-log line for a Claude session of `who` moved to `model`.
pub(super) fn failover_line(who: &str, model: &str) -> String {
    format!("{who}: claude braked (7d at 96%); started on {model} (fallback)")
}

/// Every failover line in the run log.
pub(super) fn failover_lines(fx: &Fixture) -> Vec<String> {
    (fx.run().log.iter())
        .filter(|e| e.text.ends_with("(fallback)"))
        .map(|e| e.text.clone())
        .collect()
}

pub(super) fn hold(fx: &mut Fixture, on: bool) {
    let reply = fx.reply();
    let hold = on.then(|| DispatchHold {
        since: 1_900,
        by: "user".into(),
    });
    fx.next(EventKind::Hold { reply, hold });
}

fn fixture(tasks: &[String], config: config::Orchestrator) -> Fixture {
    Fixture::with_config(&plan_with(PROFILE, tasks), config)
}

/// A run of one medium task `t3` under `config`, Claude tripped before its dispatch.
fn tripped_t3(config: config::Orchestrator) -> (Fixture, u64) {
    let mut fx = fixture(&[task("t3", "M", "a", "")], config);
    let until = trip_claude(&mut fx);
    fx.ready(true);
    (fx, until)
}

/// `t3` waited: still queued, nothing failed over, decision 13's line shown.
fn waited(fx: &Fixture, until: u64) {
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert_eq!(fx.task("t3").route, route(SONNET));
    assert!(failover_lines(fx).is_empty(), "{:?}", failover_lines(fx));
    assert!(fx.run().failovers.is_empty());
    assert!(
        attention(fx).contains(&claude_held(until)),
        "{:?}",
        attention(fx)
    );
}

#[test]
fn braked_claude_writer_starts_on_codex_fallback() {
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    fx.start(true);
    let models = fx.run().limits.models.clone();
    trip_claude(&mut fx);
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    let t3 = fx.task("t3");
    assert_eq!(t3.state, TaskState::Preparing);
    assert_eq!(t3.route, route(SOL));
    assert_eq!(t3.route.effort, Effort::DEFAULT);
    assert_eq!(t3.rung, 0);
    fx.complete_prepares();
    let (_, kind) = fx.op("CreateWindow");
    let OpKind::CreateWindow {
        spec, first_turn, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(
        (spec.runtime, spec.model.as_str()),
        (Runtime::Codex, "gpt-6-sol")
    );
    let want = crate::run::contract::worker_prompt(fx.run(), fx.task("t3"), "", "");
    assert_eq!(first_turn, want, "the worker prompt, not a hand-over");
    fx.complete_windows();
    let t3 = fx.task("t3");
    assert_eq!(t3.rounds.last().map(|r| &r.route), Some(&route(SOL)));
    assert_eq!(t3.rung, 0);
    assert_eq!(logged(&fx, &failover_line("t3", SOL)), 1);
    let lines = attention(&fx);
    assert!(
        lines.contains(&"claude braked: t3 started on codex (fallback)".to_string()),
        "{lines:?}"
    );
    assert!(
        !lines.iter().any(|l| l.starts_with("claude dispatch held")),
        "{lines:?}"
    );
    assert_eq!(fx.run().limits.models, models, "the table is untouched");
}

#[test]
fn resume_never_fails_over() {
    // A launch the restart lost is relaunched on its Claude route.
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    fx.ready(true);
    fx.complete_prepares();
    let (lost, _) = fx.op("CreateWindow");
    restart(&mut fx, Vec::new());
    trip_claude(&mut fx);
    let effects = resume(&mut fx);
    let (op, kind) = only_op(&effects, "CreateWindow");
    assert_ne!(op, lost);
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(fx.task("t3").route, route(SONNET));
    assert!(failover_lines(&fx).is_empty());
    assert!(fx.run().failovers.is_empty());

    // A paired task's implementer, due once red is confirmed, starts on its Claude
    // route (its test writer on Codex's own row).
    let config = rows(&[(Role::TestWriter, SOL, None)]);
    let mut fx = fixture(&[task("t1", "M", "a", PAIRED)], config);
    fx.ready(true);
    let writer = fx.launch_all()[0].1;
    trip_claude(&mut fx);
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    let effects = fx.done(op, red_check(true));
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Claude, "the implementer");
    assert_eq!(fx.task("t1").route, route(SONNET));
    assert!(failover_lines(&fx).is_empty());
    assert!(fx.run().failovers.is_empty());
}

#[test]
fn same_runtime_fallback_waits() {
    let (fx, until) = tripped_t3(rows(&[(Role::ImplementerMedium, SONNET, Some(OPUS))]));
    waited(&fx, until);
}

#[test]
fn no_fallback_waits() {
    let (fx, until) = tripped_t3(rows(&[(Role::ImplementerMedium, SONNET, None)]));
    waited(&fx, until);
}

#[test]
fn fallback_runtime_braked_too_waits() {
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    let until = trip_claude(&mut fx);
    trip(&mut fx, Runtime::Codex);
    fx.ready(true);
    waited(&fx, until);
}

#[test]
fn hold_blocks_failover_too() {
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    trip_claude(&mut fx);
    hold(&mut fx, true);
    fx.ready(true);
    fx.tick();
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
    assert!(attention(&fx).contains(&HOLD_TEXT.to_string()));
    assert!(failover_lines(&fx).is_empty());
    // The hold released, Claude still tripped: it fails over.
    hold(&mut fx, false);
    assert_eq!(fx.task("t3").state, TaskState::Preparing);
    assert_eq!(fx.task("t3").route, route(SOL));
    assert_eq!(failover_lines(&fx), [failover_line("t3", SOL)]);
}

/// Rule 9 (decision 39): `t3` and `t4` overlap in `owns`; `t4` declared first holds
/// `t3` behind it on Claude, so neither fails over while `t4` is unfinished.
#[test]
fn overlapping_owns_never_split_runtimes() {
    let tasks = [task("t3", "M", "a", ""), task("t4", "M", "a", "")];
    let mut fx = fixture(&tasks, rows(&[]));
    let until = trip_claude(&mut fx);
    fx.ready(true);
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert!(
        matches!(fx.task("t4").state, TaskState::Queued | TaskState::Pending),
        "{:?}",
        fx.task("t4").state
    );
    assert!(fx.task("t3").implicit_deps.is_empty(), "t3 is runnable");
    assert!(failover_lines(&fx).is_empty());
    assert!(attention(&fx).contains(&claude_held(until)));

    // With `t4` merged first, `t3` fails over.
    let mut fx = fixture(&tasks, rows(&[]));
    trip_claude(&mut fx);
    fx.start(true);
    fx.task_mut("t4").state = TaskState::Merged;
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(fx.task("t3").state, TaskState::Preparing);
    assert_eq!(fx.task("t3").route, route(SOL));
}

#[test]
fn test_writer_fails_over_on_its_own_row() {
    let config = rows(&[
        (Role::TestWriter, SONNET, Some(SOL)),
        (Role::ImplementerMedium, SONNET, None),
    ]);
    let mut fx = fixture(&[task("t1", "M", "a", PAIRED)], config);
    trip_claude(&mut fx);
    fx.ready(true);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Preparing);
    assert_eq!(
        t1.route,
        route(SONNET),
        "the implementer's route is unchanged"
    );
    let pair = t1.pair.clone().expect("paired at dispatch");
    assert_eq!(pair.writer_route, route(SOL));
    fx.complete_prepares();
    let (_, kind) = fx.op("CreateWindow");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Codex, "the test writer");
    let writer = fx.complete_windows()[0].1;
    assert_eq!(failover_lines(&fx), [failover_line("t1", SOL)]);
    // Red confirmed: the implementer starts on its Claude route, not gated.
    let (op, _) = claim_red(&mut fx, writer, &HEAD[..7]);
    let effects = fx.done(op, red_check(true));
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Claude);
    assert_eq!(fx.task("t1").route, route(SONNET));
}

#[test]
fn race_never_fails_over() {
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", super::race::RACING)]);
    let mut fx = Fixture::with_config(&plan, rows(&[]));
    trip_claude(&mut fx);
    fx.ready(true);
    fx.tick();
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Queued, "it waits");
    assert!(t1.race.is_none(), "no race starts with a braked lane");
    assert_eq!(t1.route, route(SONNET));
    assert!(failover_lines(&fx).is_empty());
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
}

#[test]
fn user_set_route_and_route_on_fallback_wait() {
    // A route the user set to another model than the row's waits; a sibling on the
    // row's own model fails over.
    let tasks = [task("t3", "M", "a", ""), task("t4", "M", "b", "")];
    let mut fx = fixture(&tasks, rows(&[]));
    let until = trip_claude(&mut fx);
    fx.start_with(true, |run| run.tasks[0].route = route(HAIKU));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert_eq!(fx.task("t3").route, route(HAIKU));
    assert_eq!(fx.task("t4").route, route(SOL));
    assert_eq!(failover_lines(&fx), [failover_line("t4", SOL)]);
    assert!(attention(&fx).contains(&claude_held(until)));

    // A route rung 2 already moved to the fallback, whose runtime is braked now.
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    trip(&mut fx, Runtime::Codex);
    fx.start_with(true, |run| run.tasks[0].route = route(SOL));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    assert_eq!(fx.task("t3").route, route(SOL));
    assert!(failover_lines(&fx).is_empty());
}

#[test]
fn escalation_continues_from_the_fallback() {
    let (mut fx, _) = tripped_t3(rows(&[]));
    assert_eq!(fx.task("t3").route, route(SOL));
    fx.with_efforts();
    let sol = ModelRef::parse(SOL).unwrap();
    let models = fx.run_mut().limits.models.as_mut().unwrap();
    models.efforts.get_mut(&sol).unwrap().default = Some("medium".into());
    let i = (fx.run().tasks.iter())
        .position(|t| t.id() == "t3")
        .unwrap();
    // Rung 2 climbs the fallback's list from its default effort, never back to Claude.
    let next = crate::run::role_step::rung2_route(fx.run(), i);
    let high = RunModels::route_of(&sol, Some("high"));
    assert_eq!(next, high);
    // The fallback's list exhausted: nothing is left, and rung 2 keeps the route.
    fx.task_mut("t3").route = high.clone();
    let models = fx.run().limits.models();
    let left = crate::run::role_step::escalate(models, Role::ImplementerMedium, &high, &[]);
    assert_eq!(left, None);
    assert_eq!(crate::run::role_step::rung2_route(fx.run(), i), high);
}

/// Decision 37 (h): Codex's cap is one writer, which `t1` (failed over first: its
/// dependent `t5` puts it ahead) takes; `t3` waits until a Codex slot frees.
#[test]
fn failover_needs_room_on_the_other_runtime() {
    let tasks = [
        task("t1", "M", "b", ""),
        task("t3", "M", "a", ""),
        task("t5", "M", "c", "deps = [\"t1\"]"),
    ];
    let mut fx = fixture(&tasks, rows(&[]));
    trip_claude(&mut fx);
    fx.start_with(true, |run| {
        run.limits.adaptive_concurrency = true;
        let mut codex = crate::run::model::RuntimeConcurrency::new(run.limits.max_writers);
        codex.cap = 1;
        run.concurrency.insert("codex".into(), codex);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(fx.task("t1").route, route(SOL));
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert_eq!(fx.task("t3").state, TaskState::Queued, "no Codex room");
    assert_eq!(fx.task("t3").route, route(SONNET));
    assert_eq!(failover_lines(&fx), [failover_line("t1", SOL)]);
    // A Codex slot frees: it fails over.
    let codex = fx.run_mut().concurrency.get_mut("codex").unwrap();
    codex.cap = 2;
    fx.tick();
    assert_eq!(fx.task("t3").state, TaskState::Preparing);
    assert_eq!(fx.task("t3").route, route(SOL));
}

#[test]
fn missing_or_failed_fallback_waits() {
    // Codex recorded not installed.
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    let until = trip_claude(&mut fx);
    fx.start_with(true, |run| {
        run.orch.installed.insert("codex".into(), false);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    waited(&fx, until);

    // Ruling RL-1: the fallback failed in this task for an environment reason. One
    // writer slot keeps `t3` queued behind `t1` while its rounds are set.
    let tasks = [task("t1", "M", "b", ""), task("t3", "M", "a", "")];
    let mut fx = Fixture::with_config(
        &plan_with(&profile_with("max_writers = 1"), &tasks),
        rows(&[]),
    );
    fx.ready(true);
    fx.launch_all();
    assert_eq!(fx.task("t3").state, TaskState::Queued);
    trip_claude(&mut fx);
    let mut failed = fx.task("t1").rounds[0].clone();
    failed.route = route(SOL);
    failed.environment_failed = true;
    let i = (fx.run().tasks.iter())
        .position(|t| t.id() == "t3")
        .unwrap();
    let ask = |fx: &Fixture| {
        let gate = &fx.state.gate;
        failover::admit(
            gate,
            fx.run(),
            Starter::Worker(i),
            Role::ImplementerMedium,
            &route(SONNET),
        )
    };
    assert!(matches!(ask(&fx), Admit::FailOver(_)), "{:?}", ask(&fx));
    fx.task_mut("t3").rounds.push(failed);
    assert_eq!(ask(&fx), Admit::Wait);
}

#[test]
fn failover_attention_goes_with_the_brake() {
    let (mut fx, until) = tripped_t3(rows(&[]));
    let line = "claude braked: t3 started on codex (fallback)".to_string();
    assert!(attention(&fx).contains(&line));
    fx.send(until, EventKind::Tick);
    assert!(!attention(&fx).contains(&line), "{:?}", attention(&fx));
    assert_eq!(
        logged(&fx, &failover_line("t3", SOL)),
        1,
        "the log keeps it"
    );
    assert_eq!(fx.task("t3").route, route(SOL), "nothing switches back");
    assert_eq!(fx.run().failovers.len(), 1);
}

#[test]
fn codex_braked_claude_fallback_mirrors() {
    let config = rows(&[(Role::ImplementerMedium, SOL, Some(SONNET))]);
    let mut fx = fixture(&[task("t3", "M", "a", "")], config);
    trip(&mut fx, Runtime::Codex);
    fx.ready(true);
    assert_eq!(fx.task("t3").state, TaskState::Preparing);
    assert_eq!(fx.task("t3").route, route(SONNET));
    let line = format!("t3: codex braked (7d at 96%); started on {SONNET} (fallback)");
    assert_eq!(logged(&fx, &line), 1);
    let blind = crate::run::engine::brakes::quota_blind_text(Runtime::Claude);
    assert_eq!(logged(&fx, &blind), 1);
    assert!(attention(&fx).contains(&"codex braked: t3 started on claude (fallback)".to_string()));
}

/// Task 6's review, the writer ruling (U17 read symmetrically): a writer that fails over
/// never lands on the model its reviewer will use. A test writer whose fallback is the
/// task's reviewer's model waits; a worker's reviewer is chosen again against the
/// fallback it moves to (decision 41), so it never is that model.
#[test]
fn a_failing_over_writer_never_lands_on_its_reviewers_model() {
    // The test writer: its fallback is the reviewer row's model, the task's reviewer.
    let config = rows(&[
        (Role::TestWriter, SONNET, Some(SOL)),
        (Role::ImplementerMedium, SONNET, None),
        (Role::Reviewer, SOL, Some(OPUS)),
    ]);
    let mut fx = fixture(&[task("t1", "M", "a", PAIRED)], config);
    let until = trip_claude(&mut fx);
    fx.ready(true);
    let t1 = fx.task("t1");
    assert_eq!(t1.review_route, Some(route(SOL)));
    assert_eq!(t1.state, TaskState::Queued);
    assert!(
        t1.pair
            .as_ref()
            .is_none_or(|p| p.writer_route != route(SOL))
    );
    assert!(failover_lines(&fx).is_empty(), "{:?}", failover_lines(&fx));
    assert!(fx.run().failovers.is_empty());
    assert!(attention(&fx).contains(&claude_held(until)));

    // The worker: it fails over, and its reviewer moves off the fallback.
    let config = rows(&[(Role::Reviewer, SOL, Some(OPUS))]);
    let (fx, _) = tripped_t3(config);
    let t3 = fx.task("t3");
    assert_eq!(t3.route, route(SOL));
    assert_eq!(t3.review_route, Some(route(OPUS)));
    assert_eq!(failover_lines(&fx), [failover_line("t3", SOL)]);
}

/// Task 6's review M1: `launch_ready`'s failover. A `Preparing` task whose worktree came
/// back while its run was paused (`ready_from`) finds Claude braked when it is resumed:
/// it waits, keeping `ready_from`, while Codex has no room (decision 37 (h)), then
/// launches on its row's Codex fallback from the commit it kept.
#[test]
fn a_ready_task_fails_over_at_launch() {
    let mut fx = fixture(&[task("t3", "M", "a", "")], rows(&[]));
    fx.ready(true);
    let (prepare, _) = fx.op("PrepareWorktree");
    restart(
        &mut fx,
        vec![(prepare, OpResult::Worktree { head: BASE.into() })],
    );
    assert_eq!(fx.task("t3").ready_from.as_deref(), Some(BASE));
    assert_eq!(fx.task("t3").route, route(SONNET));
    let run = fx.run_mut();
    run.limits.adaptive_concurrency = true;
    let mut codex = crate::run::model::RuntimeConcurrency::new(run.limits.max_writers);
    codex.cap = 0;
    run.concurrency.insert("codex".into(), codex);
    trip_claude(&mut fx);
    let effects = resume(&mut fx);
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    let t3 = fx.task("t3");
    assert_eq!(t3.state, TaskState::Preparing);
    assert_eq!(t3.ready_from.as_deref(), Some(BASE));
    assert_eq!(t3.route, route(SONNET));
    assert!(failover_lines(&fx).is_empty());
    // A Codex slot frees: it launches on the fallback from the commit it kept.
    fx.run_mut().concurrency.get_mut("codex").unwrap().cap = 1;
    let effects = fx.tick();
    let (_, kind) = only_op(&effects, "CreateWindow");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(
        (spec.runtime, spec.model.as_str()),
        (Runtime::Codex, "gpt-6-sol")
    );
    let t3 = fx.task("t3");
    assert_eq!(
        (t3.route.clone(), t3.ready_from.clone()),
        (route(SOL), None)
    );
    assert_eq!(failover_lines(&fx), [failover_line("t3", SOL)]);
}
