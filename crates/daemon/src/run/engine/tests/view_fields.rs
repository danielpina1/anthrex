//! Milestone 8c task 1: the facts the live run view reads — when the plan was approved,
//! its edit log, when a rate limit began, when rung 1 sent a worker back — and the
//! snapshot that publishes them as raw unix seconds.

use proto::{AgentRole, DeciderSource, PlanEdit, RunPath, Scale, TaskKind, TaskState, TriageInfo};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op};
use crate::headless::FailureKind;
use crate::run::engine::{AgentSignal, Effect, EventKind, TurnOutcome};
use crate::run::model::{Run, TaskEvent};
use crate::run::snapshot::snapshot;
use crate::run::triage::mark_fast;

fn one_task(extra: &str) -> Fixture {
    Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "a", extra)]))
}

fn amend(priority: i32) -> Vec<PlanEdit> {
    vec![PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: Some(priority),
        size: None,
    }]
}

fn rate_limited(kind: FailureKind) -> TurnOutcome {
    TurnOutcome::Failed {
        error: "rate_limit".into(),
        kind,
    }
}

fn retry() -> AgentSignal {
    AgentSignal::ApiRetry {
        error: "rate_limit".into(),
        delay_ms: 60_000,
    }
}

#[test]
fn approve_records_when_the_plan_was_approved() {
    // At the gate: not yet; `Approve` records its own time.
    let mut fx = one_task("");
    fx.start(false);
    assert_eq!(fx.run().approved_at, None);
    fx.approve();
    assert_eq!(fx.run().approved_by.as_deref(), Some("user"));
    assert_eq!(fx.run().approved_at, Some(fx.now));

    // `--yes`: approved when it starts.
    let mut fx = one_task("");
    fx.start(true);
    assert_eq!(fx.run().approved_at, Some(fx.now));

    // The fast path: no gate, approved when it starts.
    let mut fx = one_task("");
    fx.start_with(false, |run: &mut Run| {
        let triage = TriageInfo {
            kinds: vec![TaskKind::Code],
            scale: Scale::Single,
            path: RunPath::Fast,
            reason: "one small change".into(),
            source: DeciderSource::Decider,
            fallback_reason: None,
            at: 1_000,
        };
        mark_fast(run, triage, None);
    });
    assert_eq!(fx.run().path, Some(RunPath::Fast));
    assert_eq!(fx.run().approved_at, Some(fx.now));
}

#[test]
fn accepted_edits_are_logged_and_counted_after_approval() {
    let mut fx = one_task("");
    fx.start(false);
    let effects = edit(&mut fx, amend(5));
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    let at = fx.now;
    assert_eq!(fx.run().plan_edits.len(), 1);
    assert_eq!(fx.run().plan_edits[0].at, at);
    assert_eq!(fx.run().plan_edits[0].text, "amend t1");
    assert_eq!(fx.run().plan_edits_since_approval, 0, "before approval");

    fx.approve();
    edit(&mut fx, amend(6));
    edit(&mut fx, vec![PlanEdit::Pause, PlanEdit::Resume]);
    assert_eq!(fx.run().plan_edits_since_approval, 2);
    assert_eq!(fx.run().plan_edits.len(), 3);
    assert_eq!(fx.run().plan_edits[2].text, "pause, resume");

    // A refused batch logs nothing.
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t404".into(),
        }],
    );
    assert!(replies(&effects)[0].is_err(), "{effects:#?}");
    assert_eq!(fx.run().plan_edits.len(), 3);
    assert_eq!(fx.run().plan_edits_since_approval, 2);
}

#[test]
fn a_retry_streak_records_its_start() {
    let (mut fx, window) = super::turns::working();
    let first = fx.now + 100;
    fx.send(first, signal(window, retry()));
    let round = &fx.task("t1").rounds[0];
    assert!(round.rate_limited_until.is_some());
    assert_eq!(round.rate_limited_since, Some(first));
    // The streak goes on: its start stays.
    fx.send(first + 30, signal(window, retry()));
    assert_eq!(fx.task("t1").rounds[0].rate_limited_since, Some(first));
    // Any other event clears the limit (signals.rs, the streak's end), and its start.
    fx.send(first + 31, signal(window, AgentSignal::Activity));
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.rate_limited_until, None);
    assert_eq!(round.rate_limited_since, None);

    // A failed turn's rate limit starts one; its continue clears it.
    let (mut fx, window) = super::turns::working();
    fx.turn_ended(window, rate_limited(FailureKind::RateLimit));
    let at = fx.now;
    let until = fx.task("t1").rounds[0].rate_limited_until.expect("limited");
    assert_eq!(fx.task("t1").rounds[0].rate_limited_since, Some(at));
    fx.send(until, EventKind::Tick);
    let round = &fx.task("t1").rounds[0];
    assert_eq!(round.rate_limited_until, None);
    assert_eq!(round.rate_limited_since, None);

    // A reviewer's, likewise.
    let (mut fx, _, rwindow) = super::gates_review::reviewed(PROFILE, "");
    fx.turn_ended(rwindow, rate_limited(FailureKind::RateLimit));
    let at = fx.now;
    let round = fx.task("t1").rounds.last().unwrap().clone();
    assert_eq!(round.role, AgentRole::Reviewer);
    assert_eq!(round.rate_limited_since, Some(at));
    let until = round.rate_limited_until.expect("limited");
    fx.send(until, EventKind::Tick);
    let round = fx.task("t1").rounds.last().unwrap();
    assert_eq!(round.rate_limited_until, None);
    assert_eq!(round.rate_limited_since, None);
}

fn signal(window_id: u32, signal: AgentSignal) -> EventKind {
    EventKind::Signal { window_id, signal }
}

fn delivered_texts(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Deliver { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn rung_one_marks_the_worker_round_sent_back() {
    // A failed check at rung 0: rung 1 sends the same session back, and says so.
    let (mut fx, window) = super::gates::working_on(PROFILE, CHECK_MODE);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    let effects = fx.done(op, check_result(false));
    let first = fx.now;
    assert_eq!(fx.task("t1").rung, 1);
    assert_eq!(delivered_texts(&effects).len(), 1, "the text is queued");
    let t1 = fx.task("t1");
    assert_eq!(t1.rounds.len(), 1);
    assert_eq!(t1.rounds[0].sent_back_at, vec![first]);

    // The second failure is rung 2: a fresh session, a new round, not another bounce.
    fx.turn_completed(window);
    let effects = accepted(&mut fx, window, json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "Check");
    fx.done(op, check_result(false));
    assert_eq!(fx.task("t1").rung, 2);
    let effects = super::turns::killed_exit(&mut fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.done(
        op,
        crate::run::engine::OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    fx.complete_windows();
    let t1 = fx.task("t1");
    assert_eq!(t1.rounds.len(), 2);
    assert_eq!(t1.rounds[0].sent_back_at, vec![first]);
    assert_eq!(
        (t1.rounds[1].session, t1.rounds[1].role),
        (2, AgentRole::Worker)
    );
    assert!(t1.rounds[1].sent_back_at.is_empty());

    // `told`: the tool reply carried the failure; still sent back, nothing queued.
    let (mut fx, window) = super::gates::working_on(PROFILE, CHECK_MODE);
    let effects = fx.tool(window, "task_done", json!({"summary": "s"}));
    let (op, _) = only_op(&effects, "VerifyDone");
    let mut result = fx.clean_check("t1");
    if let crate::run::engine::OpResult::DoneChecked {
        protected_changed, ..
    } = &mut result
    {
        *protected_changed = vec!["AGENTS.md".into()];
    }
    let queued = fx.run().outbox.len();
    let effects = fx.done(op, result);
    assert!(replies(&effects)[0].is_err(), "{effects:#?}");
    assert_eq!(fx.task("t1").rung, 1);
    assert_eq!(fx.task("t1").state, TaskState::Working);
    assert_eq!(fx.run().outbox.len(), queued, "told: nothing queued");
    assert_eq!(fx.task("t1").rounds[0].sent_back_at, vec![fx.now]);
}

/// Every string in `value`, recursively.
fn strings(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

/// `hh:mm` anywhere in `s`.
fn has_clock(s: &str) -> bool {
    s.as_bytes().windows(5).any(|w| {
        w[0].is_ascii_digit()
            && w[1].is_ascii_digit()
            && w[2] == b':'
            && w[3].is_ascii_digit()
            && w[4].is_ascii_digit()
    })
}

#[test]
fn snapshot_carries_the_view_fields() {
    let mut fx = one_task("[task.route]\nruntime = \"claude\"\neffort = \"low\"");
    fx.start(false);
    assert_eq!(snapshot(&fx.state, 777).now, 777);

    // Twelve accepted edits; the snapshot shows the newest ten, newest first.
    for n in 0..12 {
        edit(&mut fx, amend(n));
    }
    let last_edit = fx.now;
    // Twelve events past what the task already had; the newest ten are shown.
    let base = fx.now + 1_000;
    for n in 0..12u64 {
        fx.task_mut("t1").history.push(TaskEvent {
            at: base + n,
            text: format!("event {n}"),
        });
    }
    let now = fx.now;
    let snap = snapshot(&fx.state, now);
    let run = &snap.runs[0];
    assert_eq!(run.plan_edits.len(), 10);
    assert_eq!(run.plan_edits[0].at, last_edit);
    assert!(run.plan_edits.windows(2).all(|w| w[0].at >= w[1].at));
    assert_eq!(run.approved_at, None);
    assert_eq!(run.plan_edits_since_approval, 0);
    let t1 = &run.tasks[0];
    assert_eq!(t1.brief, "Brief t1");
    assert_eq!(t1.acceptance, ["Accept t1"]);
    let spec = &fx.task("t1").spec.route;
    assert_eq!(&t1.route_spec, spec);
    assert_eq!(t1.route_spec.strength, None, "unset in the plan: policy");
    assert_eq!(t1.route_spec.effort, Some(proto::Effort::Low));
    assert_eq!(t1.route.effort, proto::Effort::Low);
    assert_eq!(t1.history.len(), 10);
    assert_eq!(t1.history[0].at, base + 11);
    assert_eq!(t1.history[0].text, "event 11");
    assert_eq!(t1.history[9].at, base + 2);

    // After approval: a rate-limited worker round publishes its raw times.
    let mut fx = one_task("");
    fx.ready(false);
    fx.approve();
    let approved = fx.now;
    let window = fx.launch_all()[0].1;
    let started = fx.now;
    fx.task_mut("t1").rounds[0].set_rate_limited(Some(900), started);
    fx.task_mut("t1").rounds[0].sent_back_at = vec![started + 1];
    let snap = snapshot(&fx.state, started + 2);
    assert_eq!(snap.runs[0].approved_at, Some(approved));
    let round = &snap.runs[0].tasks[0].rounds[0];
    assert_eq!(round.window_id, Some(window));
    assert_eq!(round.rate_limited_until, Some(900));
    assert_eq!(round.rate_limited_since, Some(started));
    assert_eq!(round.sent_back_at, vec![started + 1]);
    let mut texts = Vec::new();
    strings(&serde_json::to_value(&snap).unwrap(), &mut texts);
    let clocks: Vec<&String> = texts.iter().filter(|s| has_clock(s)).collect();
    assert!(clocks.is_empty(), "formatted times: {clocks:?}");

    fx.task_mut("t1").rounds[0].set_rate_limited(None, started + 3);
    let snap = snapshot(&fx.state, started + 3);
    let round = &snap.runs[0].tasks[0].rounds[0];
    assert_eq!(round.rate_limited_until, None);
    assert_eq!(round.rate_limited_since, None);
}

/// A `run.json` written by milestone 8b's code (built with this fixture and
/// `serde_json` before milestone 8c changed the model) loads unchanged.
#[test]
fn old_run_json_loads() {
    let text = include_str!("m8b_run.json");
    let run: Run = serde_json::from_str(text).expect("an M8b run.json loads");
    assert_eq!(run.approved_at, None);
    assert!(run.plan_edits.is_empty());
    assert_eq!(run.plan_edits_since_approval, 0);
    let round = &run.tasks[0].rounds[0];
    assert_eq!(round.rate_limited_since, None);
    assert!(round.sent_back_at.is_empty());
    assert!(
        round.rate_limited_until.is_some(),
        "the stored value is kept"
    );
    // Everything it stored comes back as it was: only the new keys are added.
    let mut back = serde_json::to_value(&run).unwrap();
    let map = back.as_object_mut().unwrap();
    for key in ["approved_at", "plan_edits", "plan_edits_since_approval"] {
        assert!(map.remove(key).is_some(), "{key}");
    }
    for task in back["tasks"].as_array_mut().unwrap() {
        for round in task["rounds"].as_array_mut().unwrap() {
            let round = round.as_object_mut().unwrap();
            for key in ["rate_limited_since", "sent_back_at"] {
                assert!(round.remove(key).is_some(), "{key}");
            }
        }
    }
    let stored: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(back, stored);
}
