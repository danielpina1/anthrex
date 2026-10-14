//! Milestone 9.16 task M9.16.5: the engine gate (decisions 8, 11–14, 18, 21; rulings
//! C3–C6). Every brake here is switched on by a `BrakesConfig` event first (ruling C3:
//! the engine's default brakes nothing). The readers' half (sub-planners, run scouts,
//! the spinning-loop guard) is `brakes_readers.rs`. Pure: `now` moves by hand.

use proto::brakes::{DispatchHold, Permille, QuotaWindow};
use proto::{RunState, Runtime, TaskState};

use super::control::resume;
use super::control_restore::restart;
use super::dispatch::replies;
use super::fixture::*;
use super::gates::{CHECK_MODE, only_op};
use super::gates_support::{blocking, in_review, reviewer, submit, verdict};
use super::holds::delivers;
use crate::headless::quota::QuotaSample;
use crate::run::engine::brakes::{HOLD_TEXT, quota_blind_text};
use crate::run::engine::quota_book::BrakeConfig;
use crate::run::engine::{Effect, EngineState, Event, EventKind, OpKind, OpResult, step};
use crate::run::snapshot::snapshot;

/// The brake switched on at U11's thresholds (90 % 5h, 95 % 7d).
pub(super) fn brake_on() -> EventKind {
    EventKind::BrakesConfig(BrakeConfig {
        quota_brake: true,
        ..BrakeConfig::default()
    })
}

/// One reading of `runtime`'s `window` at `permille`, resetting at `resets_at`.
pub(super) fn reading(
    runtime: Runtime,
    window: QuotaWindow,
    permille: u16,
    resets_at: u64,
) -> EventKind {
    EventKind::Quota {
        runtime,
        sample: QuotaSample {
            windows: vec![(window, Permille(permille), Some(resets_at))],
            rejected: None,
        },
    }
}

/// The brake on, then Claude's 7-day window at 96 %, resetting in three hours: a trip.
pub(super) fn trip_claude(fx: &mut Fixture) -> u64 {
    let until = fx.now + 3 * 3600;
    fx.next(brake_on());
    fx.next(reading(Runtime::Claude, QuotaWindow::SevenDay, 960, until));
    until
}

/// The held line [`trip_claude`]'s trip gives.
pub(super) fn claude_held(until: u64) -> String {
    format!(
        "claude dispatch held: 7d at 96% (≥95%) until {} UTC",
        common::text::hh_mm(until)
    )
}

pub(super) fn attention(fx: &Fixture) -> Vec<String> {
    snapshot(&fx.state, fx.now).runs[0].attention.clone()
}

pub(super) fn logged(fx: &Fixture, text: &str) -> usize {
    fx.run().log.iter().filter(|e| e.text == text).count()
}

/// The config that puts size M on Codex, size S staying on Claude (no fallback on
/// either row: nothing fails over before M9.16.6).
fn mixed() -> config::Orchestrator {
    codex_medium()
}

/// A run of Claude `t1` (S) and Codex `t2` (M) under `config`.
fn two_runtimes(profile: &str) -> Fixture {
    let plan = plan_with(
        profile,
        &[task("t1", "S", "a", ""), task("t2", "M", "b", "")],
    );
    Fixture::with_config(&plan, mixed())
}

fn hold(since: u64) -> Option<DispatchHold> {
    Some(DispatchHold {
        since,
        by: "user".into(),
    })
}

#[test]
fn brake_holds_claude_dispatch_not_codex() {
    let mut fx = two_runtimes(PROFILE);
    let until = trip_claude(&mut fx);
    fx.ready(true);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    let launched = fx.launch_all();
    assert_eq!(
        launched.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>(),
        vec!["t2"]
    );
    assert_eq!(fx.task("t1").state, TaskState::Queued, "still held");
    assert_eq!(fx.task("t2").state, TaskState::Working);
    assert!(
        attention(&fx).contains(&claude_held(until)),
        "{:?}",
        attention(&fx)
    );
    // Its race decision is not latched while it waits (ruling C5).
    assert_eq!(fx.task("t1").race_decision, None);
}

#[test]
fn in_flight_task_keeps_its_reviewer_and_fix_round_while_braked() {
    // The reviewer on Claude too, so its start is on the braked runtime.
    let mut config = config::Orchestrator::default();
    let reviewer_row = "claude:claude-opus-5-5";
    with_row(
        &mut config,
        proto::models::Role::Reviewer,
        reviewer_row,
        None,
    );
    let plan = plan_with(
        &profile_with("max_writers = 1"),
        &[task("t1", "S", "a", CHECK_MODE), task("t2", "S", "b", "")],
    );
    let mut fx = Fixture::with_config(&plan, config);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t2").state, TaskState::Queued, "one writer slot");
    trip_claude(&mut fx);
    // The check passes: its review starts on the braked runtime.
    let (op, _) = in_review(&mut fx, window);
    let (rwindow, kind) = reviewer(&mut fx, op, "diff --git a/x b/x");
    let OpKind::CreateWindow { spec, .. } = kind else {
        unreachable!()
    };
    assert_eq!(spec.runtime, Runtime::Claude);
    // `t1` in review holds no writer slot, yet `t2` stays queued.
    assert_eq!(crate::run::engine::schedule::writers_busy(fx.run()), 0);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    // Its fix round: the findings go to its worker, which works again.
    let effects = submit(&mut fx, rwindow, verdict("changes", blocking()));
    super::turns::exited(&mut fx, rwindow);
    let mut fixes = delivers(&effects);
    fixes.extend(delivers(&fx.tick()));
    assert!(
        fixes.iter().any(|t| t.contains("critical text")),
        "{fixes:?}"
    );
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(!tasks_of(&fx.log, "PrepareWorktree").contains(&"t2".to_string()));
}

#[test]
fn restore_relaunch_and_deciders_are_not_gated() {
    // A launch the restart lost is relaunched under the brake.
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
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

    // A queued decider (the `helpers` row: Claude) takes the freed reader slot.
    let (mut fx, decider_id, review) = super::deciders::slot_taken();
    trip_claude(&mut fx);
    let effects = fx.done(
        review,
        OpResult::Failed {
            message: "no space".into(),
        },
    );
    let decides = ops_in(&effects, "Decide");
    assert_eq!(decides.len(), 1, "{effects:#?}");
    assert!(fx.queued_deciders().is_empty());
    assert!(
        (fx.run().pending_ops.values()).any(|p| matches!(
            &p.kind,
            OpKind::Decide { decider_id: id, .. } if *id == decider_id
        )),
        "the queued decider runs"
    );
}

#[test]
fn launch_ready_waits_and_keeps_ready_from() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.ready(true);
    let (prepare, _) = fx.op("PrepareWorktree");
    // The worktree comes back while the restored run is paused: ready, not launched.
    restart(
        &mut fx,
        vec![(prepare, OpResult::Worktree { head: BASE.into() })],
    );
    assert_eq!(fx.task("t1").ready_from.as_deref(), Some(BASE));
    let until = trip_claude(&mut fx);
    let effects = resume(&mut fx);
    assert!(ops_in(&effects, "CreateWindow").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert_eq!(fx.task("t1").ready_from.as_deref(), Some(BASE));
    assert!(attention(&fx).contains(&claude_held(until)));
    // The window resets: it launches from the commit it kept.
    let effects = fx.send(until, EventKind::Tick);
    assert_eq!(ops_in(&effects, "CreateWindow").len(), 1, "{effects:#?}");
    assert_eq!(fx.task("t1").ready_from, None);
}

#[test]
fn release_on_tick_republishes() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    let until = trip_claude(&mut fx);
    fx.ready(true);
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    let revision = fx.state.revision;
    let effects = fx.send(until, EventKind::Tick);
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert!(fx.state.revision > revision);
    assert!(effects.iter().any(|e| matches!(e, Effect::Publish { .. })));
    assert!(!attention(&fx).contains(&claude_held(until)));

    // With nothing to launch, the release alone still moves the revision and publishes
    // (decision 14: the snapshot's brakes changed).
    let mut state = EngineState::default();
    let send = |state: EngineState, now, kind| step(state, Event { now, kind });
    (state, _) = send(state, 100, brake_on());
    let resets = 200;
    (state, _) = send(
        state,
        101,
        reading(Runtime::Claude, QuotaWindow::FiveHour, 950, resets),
    );
    let braked = snapshot(&state, 101).brakes.quota[0].trip.clone();
    assert!(braked.is_some());
    let revision = state.revision;
    (state, _) = send(state, 150, EventKind::Tick);
    assert_eq!(state.revision, revision, "nothing changed before the reset");
    let (state, effects) = send(state, resets, EventKind::Tick);
    assert_eq!(state.revision, revision + 1);
    assert_eq!(effects, vec![Effect::Publish { structural: true }]);
    assert_eq!(snapshot(&state, resets).brakes.quota[0].trip, None);
}

#[test]
fn stale_reading_fails_open_and_announces_once() {
    let mut fx = two_runtimes(PROFILE);
    fx.next(brake_on());
    fx.ready(true);
    // No reading: both start, each announced once in this run.
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    assert_eq!(logged(&fx, &quota_blind_text(Runtime::Claude)), 1);
    assert_eq!(logged(&fx, &quota_blind_text(Runtime::Codex)), 1);
    fx.launch_all();
    fx.tick();
    assert_eq!(logged(&fx, &quota_blind_text(Runtime::Claude)), 1);

    // A second run's Claude launch in a later step logs nothing.
    let mut second = build(
        &plan_with(PROFILE, &[task("u1", "S", "c", "")]),
        &fx.config,
        true,
    );
    second.id = "second-run-7c1d".into();
    let id = second.id.clone();
    let reply = fx.reply();
    fx.next(EventKind::Start {
        reply,
        run: Box::new(second),
    });
    let run = &fx.state.runs[&id];
    let op = (run.pending_ops.values())
        .find(|p| matches!(p.kind, OpKind::CreateRunBranch { .. }))
        .map(|p| p.op)
        .expect("its run branch");
    fx.next(EventKind::OpDone {
        run_id: id.clone(),
        op,
        result: OpResult::Worktree { head: BASE.into() },
    });
    let run = &fx.state.runs[&id];
    assert_eq!(run.tasks[0].state, TaskState::Preparing);
    let blind = quota_blind_text(Runtime::Claude);
    assert!(!run.log.iter().any(|e| e.text == blind), "{:?}", run.log);

    // With the brake off nothing is announced at all (ruling C3).
    let mut fx = two_runtimes(PROFILE);
    fx.ready(true);
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert_eq!(logged(&fx, &quota_blind_text(Runtime::Claude)), 0);
    assert_eq!(logged(&fx, &quota_blind_text(Runtime::Codex)), 0);
}

#[test]
fn hold_gates_every_runtime_and_changes_no_run_state() {
    let mut fx = two_runtimes(PROFILE);
    fx.start(true);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Hold {
        reply,
        hold: hold(1_900),
    });
    assert_eq!(
        replies(&effects),
        vec![Ok(
            "dispatch held daemon-wide; running sessions continue · anthrex hold off releases it"
                .to_string()
        )]
    );
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx.tick();
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(
        (run.paused_from, run.paused_at, run.paused_secs),
        (None, None, 0)
    );
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
    assert!(attention(&fx).contains(&HOLD_TEXT.to_string()));
    assert_eq!(snapshot(&fx.state, fx.now).brakes.hold, hold(1_900));
    // A second `on` keeps the first `since`.
    let reply = fx.reply();
    let effects = fx.next(EventKind::Hold {
        reply,
        hold: hold(5_000),
    });
    let text = format!(
        "dispatch was already held since {} UTC",
        common::text::hh_mm(1_900)
    );
    assert_eq!(replies(&effects), vec![Ok(text)]);
    assert_eq!(fx.state.brakes.hold, hold(1_900));

    let reply = fx.reply();
    let effects = fx.next(EventKind::Hold { reply, hold: None });
    assert_eq!(replies(&effects), vec![Ok("dispatch released".to_string())]);
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    assert!(!attention(&fx).contains(&HOLD_TEXT.to_string()));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Hold { reply, hold: None });
    assert_eq!(
        replies(&effects),
        vec![Ok("dispatch is not held".to_string())]
    );
}

#[test]
fn resume_run_does_not_lift_hold() {
    let mut fx = two_runtimes(PROFILE);
    let orch = super::orch::launched(false).run().orch.orchestrator.clone();
    let reply = fx.reply();
    fx.next(EventKind::Hold {
        reply,
        hold: hold(1_900),
    });
    fx.ready(true);
    fx.run_mut().orch.orchestrator = orch;
    let now = fx.now;
    crate::run::engine::merge::halt(fx.run_mut(), "git timed out".into(), now);
    fx.run_mut().halt_retryable = true;
    let edit = serde_json::json!({"op": "resume_run", "reason": "the refs read again"});
    let effects = super::orch::edit_plan(&mut fx, serde_json::json!({"edits": [edit]}));
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        format!("run {RUN_ID} resumed")
    );
    assert_ne!(fx.run().state, RunState::Halted);
    fx.tick();
    assert!(fx.state.brakes.hold.is_some());
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(ops_in(&fx.log, "PrepareWorktree").is_empty());
}

#[test]
fn brakes_config_event_reconfigures() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    fx.next(brake_on());
    let until = fx.now + 3 * 3600;
    // 92 % is under the 95 % default.
    fx.next(reading(Runtime::Claude, QuotaWindow::SevenDay, 920, until));
    fx.start(true);
    fx.next(EventKind::BrakesConfig(BrakeConfig {
        quota_brake: true,
        pause_7d_percent: 90,
        ..BrakeConfig::default()
    }));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    assert_eq!(fx.task("t1").state, TaskState::Queued);
    let line = format!(
        "claude dispatch held: 7d at 92% (≥90%) until {} UTC",
        common::text::hh_mm(until)
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    // Switched off, the stored reading brakes nothing.
    fx.next(EventKind::BrakesConfig(BrakeConfig::default()));
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
}

#[test]
fn snapshot_carries_brakes_info() {
    use proto::brakes::QuotaState;
    // Off (ruling C3): empty, so every existing snapshot keeps its shape.
    let fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    assert!(snapshot(&fx.state, fx.now).brakes.is_empty());

    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", "")]));
    let until = trip_claude(&mut fx);
    let reply = fx.reply();
    fx.next(EventKind::Hold {
        reply,
        hold: hold(1_900),
    });
    let brakes = snapshot(&fx.state, fx.now).brakes;
    assert_eq!(brakes.hold, hold(1_900));
    assert!(brakes.quota_brake);
    let states: Vec<_> = (brakes.quota.iter())
        .map(|q| (q.runtime, q.state))
        .collect();
    assert_eq!(
        states,
        vec![
            (Runtime::Claude, QuotaState::Braked),
            (Runtime::Codex, QuotaState::Unmeasured)
        ]
    );
    let trip = brakes.quota[0].trip.clone().expect("Claude's trip");
    assert_eq!(
        (trip.window, trip.used, trip.threshold_percent, trip.until),
        (QuotaWindow::SevenDay, Permille(960), 95, until)
    );
}

/// Ruling C5: a race is gated before it is decided. Its own runtime braked, it waits
/// with no decision latched; its peer's braked, the race waits for its slot, then runs
/// single, and no lane starts on the braked runtime.
#[test]
fn a_race_never_starts_a_braked_lane() {
    let plan = plan_with(PROFILE, &[task("t1", "M", "a", super::race::RACING)]);
    let mut fx = super::race::racers(&plan);
    let until = trip_claude(&mut fx);
    fx.ready(true);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Queued);
    assert_eq!((t1.race_decision.clone(), t1.race_wait_since), (None, None));
    fx.send(until, EventKind::Tick);
    assert!(fx.task("t1").race.is_some(), "released, it races");

    // Its peer (the row's Codex fallback) braked: no race starts.
    let mut fx = super::race::racers(&plan);
    fx.next(brake_on());
    let until = fx.now + 3 * 3600;
    fx.next(reading(Runtime::Codex, QuotaWindow::SevenDay, 960, until));
    fx.ready(true);
    assert!(fx.task("t1").race.is_none());
    assert_eq!(
        fx.task("t1").state,
        TaskState::Queued,
        "it waits for its slot"
    );
    let wait = fx.run().limits.race_slot_wait_secs;
    fx.send(fx.now + wait, EventKind::Tick);
    let t1 = fx.task("t1");
    assert!(t1.race.is_none());
    assert_eq!(t1.state, TaskState::Preparing, "it runs single");
    assert!(
        t1.notes
            .iter()
            .any(|n| n.starts_with("race skipped: no second writer slot")),
        "{:?}",
        t1.notes
    );
}
