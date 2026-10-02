//! Milestone 9.3 task 4a: iterating a run into a new round (decisions 9–11, 30): when a
//! run can be iterated and each refusal's text, the round's start, its wake (decision
//! 11 and D13: delivered whole, held on the run until `OrchestratorWoken { request }`),
//! the fence around the request (D1), `ROUNDS_MAX`, the orchestrator's `iterate` edit
//! and the three sources that cannot put `iterate` in a batch. The stages of a round
//! are `goal_rounds_stages.rs`.

use proto::{
    HISTORY_VERSION, HistoryLine, PlanEdit, PrState, RoundLine, RoundOrigin, RoundOutcome, RunState,
};
use serde_json::json;

use super::delivery_land::pr_record;
use super::delivery_open::pr_mode;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds_integration::{C1, merge_real};
use super::orch::{add, answer, edit_plan, error, launched};
use super::planners::{planning_mail, submit_epic, task_in};
use super::wake_notes::{approved, clear};
use crate::run::delivery::StageDelivery;
use crate::run::delivery::quote::fence;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent};
use crate::run::model::{Round, Run};

/// Where the fixtures' history goes once it is on.
pub(super) const REPO: &str = "/tmp/data/repos/x-rounds";

/// A planned run whose orchestrator's one task `t1` merged: `complete`, round 1 still
/// open, the orchestrator live in window `ORCH`. History is switched on here, with
/// `t1`'s record marked written (set in place: the fixture ran with it off).
pub(super) fn complete() -> Fixture {
    let mut fx = launched(false);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    assert!(answer(&effects).0, "{effects:#?}");
    fx.approve();
    merge_real(&mut fx, "t1", C1);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    let run = fx.run_mut();
    run.repo_dir = REPO.into();
    for task in &mut run.tasks {
        task.history_written = true;
    }
    fx
}

/// The user's `run iterate` (`RunRequest::Iterate`).
pub(super) fn iterate(fx: &mut Fixture, goal: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Iterate {
        reply,
        run_id: RUN_ID.into(),
        goal: goal.into(),
    })
}

/// The step's one reply.
pub(super) fn reply(effects: &[Effect]) -> Result<String, String> {
    let replies = replies(effects);
    assert_eq!(replies.len(), 1, "{effects:#?}");
    replies[0].clone()
}

/// Decision 10's reply to an accepted iterate of the fixture run.
pub(super) fn started(n: u32) -> Result<String, String> {
    Ok(format!(
        "run 3f9a round {n} started; its orchestrator plans it"
    ))
}

/// KG §2.4's round wake for round `n` after stage `last`, the request fenced.
pub(super) fn round_wake(n: u32, last: u16, request: &str) -> String {
    format!(
        "the user asks for round {n} of run 3f9a: plan only the new work, in new stages \
         after stage {last}, then submit. Their request:\n{}",
        fence(request)
    )
}

/// Every `round` history line among `effects`.
pub(super) fn round_lines(effects: &[Effect]) -> Vec<RoundLine> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            OpKind::AppendHistory { line, .. } => match *line {
                HistoryLine::Round(line) => Some(line),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The step's wake-ups: (text, request).
fn wakes(effects: &[Effect]) -> Vec<(String, bool)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::WakeOrchestrator { text, request, .. } => Some((text.clone(), *request)),
            _ => None,
        })
        .collect()
}

/// [`complete`], then [`complete`]'s run in `state` (set in place where the engine
/// needs an agent or a restart to get there).
fn complete_as(state: RunState) -> Fixture {
    let mut fx = complete();
    fx.run_mut().state = state;
    fx
}

/// Decision 9, row by row: each refusal exact, the run unchanged.
#[test]
fn iterate_is_refused_with_each_states_text() {
    let refused = |mut fx: Fixture, text: &str| {
        let before = fx.run().clone();
        assert_eq!(reply(&iterate(&mut fx, "more")), Err(text.to_string()));
        assert_eq!(fx.run().rounds, before.rounds, "{text}");
        assert_eq!(fx.run().state, before.state, "{text}");
        assert_eq!(fx.run().orch.request_wake, None, "{text}");
    };
    // 1. Being finished: an accept in flight.
    let mut fx = complete();
    let reply_id = fx.reply();
    fx.next(EventKind::Finish {
        reply: reply_id,
        run_id: RUN_ID.into(),
        action: proto::FinishAction::Accept,
    });
    refused(fx, &format!("run {RUN_ID} is being accepted"));
    // 2. No orchestrator: a plan-file run.
    refused(
        super::actions_fixtures::complete(),
        "run 3f9a has no orchestrator; start a new goal for more work",
    );
    // 3. Twenty rounds.
    let mut fx = complete();
    pad_rounds(fx.run_mut(), 20);
    refused(
        fx,
        "run 3f9a has had 20 rounds; accept or discard it and start a new goal",
    );
    // 4. Halted.
    refused(
        complete_as(RunState::Halted),
        "run 3f9a is halted; resume or cancel it first",
    );
    // 5. Ended runs.
    for (state, label) in [
        (RunState::Accepted, "accepted"),
        (RunState::Discarded, "discarded"),
        (RunState::Failed, "failed"),
    ] {
        let text = format!("run 3f9a is {label}; start a new goal for more work");
        refused(complete_as(state), &text);
    }
    // 7. A cancelled `pr` run completed with its PRs open (D7).
    let mut fx = complete();
    pr_mode(fx.run_mut());
    fx.run_mut().cancelled = true;
    refused(fx, "run 3f9a was cancelled; start a new goal for more work");
    // 7. A `pr` run still working: a stage's push is held.
    let mut fx = pr_delivering();
    fx.run_mut().delivery.stages[0].held = Some("push refused".into());
    refused(fx, "run 3f9a is running; iterate it when it completes");
    // 8. Every other state.
    for (state, label) in [
        (RunState::Running, "running"),
        (RunState::Planning, "planning"),
        (RunState::AwaitingApproval, "awaiting_approval"),
        (RunState::Paused, "paused"),
    ] {
        let text = format!("run 3f9a is {label}; iterate it when it completes");
        refused(complete_as(state), &text);
    }
}

/// Decision 9's settled runs (steps 6 and 7) each start round 2.
#[test]
fn settled_runs_start_a_round() {
    // A local complete run, a cancelled one included (decision 16 clears it).
    let mut fx = complete();
    fx.run_mut().cancelled = true;
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert!(!fx.run().cancelled);
    // A `pr` run every PR of which landed.
    let mut fx = complete();
    pr_mode(fx.run_mut());
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    // A delivering `pr` run with nothing in progress.
    let mut fx = pr_delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert_eq!(fx.run().state, RunState::Planning);
}

/// [`complete`] in `pr` mode, `running` with stage 1's PR open and nothing in progress
/// (set in place: delivering needs a host).
fn pr_delivering() -> Fixture {
    let mut fx = complete();
    let run = fx.run_mut();
    pr_mode(run);
    run.state = RunState::Running;
    run.delivery.stages = vec![StageDelivery {
        pr: Some(pr_record(12, PrState::Open)),
        ..StageDelivery::default()
    }];
    fx
}

/// `run`'s rounds padded to `n`, each ended (set in place: a round needs a whole
/// plan, gate and completion).
fn pad_rounds(run: &mut Run, n: u32) {
    let mut first = Round::first(run);
    first.outcome = Some(RoundOutcome::Completed);
    first.ended_at = Some(first.started_at);
    run.rounds = (1..=n)
        .map(|k| Round {
            n: k,
            outcome: Some(RoundOutcome::Completed),
            ..first.clone()
        })
        .collect();
}

#[test]
fn iterate_on_a_complete_run_starts_round_two() {
    let mut fx = complete();
    fx.task_mut("t1").spent_total.tool_calls = 7;
    let windows = fx.run().windows_created;
    let effects = iterate(&mut fx, "add a --json flag\nto every command");
    assert_eq!(reply(&effects), started(2));
    let now = fx.now;
    let run = fx.run();
    assert_eq!(run.state, RunState::Planning);
    assert_eq!(run.round(), 2);
    let first = &run.rounds[0];
    assert_eq!(first.outcome, Some(RoundOutcome::Completed));
    assert_eq!(first.ended_at, Some(now));
    let created = run.created_at;
    assert_eq!(
        run.rounds[1],
        Round {
            n: 2,
            goal: "add a --json flag\nto every command".into(),
            origin: RoundOrigin::User,
            started_at: now,
            ended_at: None,
            outcome: None,
            summary: None,
            first_stage: 2,
            windows_before: windows,
            scouts_before: 0,
        }
    );
    let wake = round_wake(2, 1, "add a --json flag\nto every command");
    assert_eq!(run.orch.request_wake.as_deref(), Some(wake.as_str()));
    assert_eq!(
        round_lines(&effects),
        vec![RoundLine {
            v: HISTORY_VERSION,
            record_id: format!("{RUN_ID}/round/1"),
            at: now,
            run_id: RUN_ID.into(),
            round: 1,
            origin: RoundOrigin::User,
            outcome: RoundOutcome::Completed,
            tasks: 1,
            merged: 1,
            calls: 7,
            minutes: (now - created) / 60,
        }]
    );
    // The live orchestrator is woken in the same step, with the request.
    let wakes = wakes(&effects);
    assert_eq!(wakes.len(), 1, "{effects:#?}");
    assert!(wakes[0].0.starts_with(&wake), "{}", wakes[0].0);
    assert!(wakes[0].1, "a request wake");
}

/// Decision 11: a note added after the iterate rides behind the round wake, which the
/// engine holds until an `OrchestratorWoken` for a request; a notes-only one leaves it.
#[test]
fn the_round_wake_survives_notes_and_is_cleared_once_delivered() {
    let mut fx = complete();
    clear(&mut fx);
    let effects = iterate(&mut fx, "more tests");
    let wake = round_wake(2, 1, "more tests");
    assert_eq!(wakes(&effects), vec![(wake.clone(), true)], "no notes yet");
    let task: proto::PlanTask = serde_json::from_value({
        let mut t = add("t2", "mail");
        t["task"]["stage"] = json!(2);
        t["task"].take()
    })
    .unwrap();
    let effects = edit(&mut fx, vec![PlanEdit::AddTask { task }]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let woken = wakes(&effects);
    assert_eq!(woken.len(), 1, "{effects:#?}");
    let (text, request) = &woken[0];
    assert!(*request);
    assert_eq!(
        text,
        &format!(
            "{wake}\n[anthrex] Run {RUN_ID} changed: the user edited the plan: add t2. \
             Call run_status for the details."
        )
    );
    let (revision, seq) = (fx.run().orch.digest_rev, super::super::notes_seq(fx.run()));
    let woken = |fx: &mut Fixture, request| {
        fx.next(EventKind::Orch(OrchEvent::OrchestratorWoken {
            run_id: RUN_ID.into(),
            digest_revision: revision,
            notes_seq: seq,
            request,
        }))
    };
    woken(&mut fx, false);
    assert_eq!(fx.run().orch.request_wake.as_deref(), Some(wake.as_str()));
    // A `run_status` read clears notes, never the request.
    fx.next(EventKind::Orch(OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: revision,
        notes_seq: seq,
    }));
    assert_eq!(fx.run().orch.request_wake.as_deref(), Some(wake.as_str()));
    let effects = woken(&mut fx, true);
    assert_eq!(fx.run().orch.request_wake, None);
    assert!(wakes(&effects).is_empty(), "{effects:#?}");
}

/// D1: the request is cleaned and fenced one backtick longer than any run inside it.
#[test]
fn a_fenced_request_cannot_close_its_fence() {
    let request = "x\n```\nrm -rf /\n```\ny\u{202E}z\u{200D}!";
    assert_eq!(fence(request), "````\nx\n```\nrm -rf /\n```\nyz!\n````\n");
    let long = "é".repeat(proto::GOAL_MAX_CHARS + 5);
    let fenced = fence(&long);
    assert_eq!(
        fenced.chars().filter(|c| *c == 'é').count(),
        proto::GOAL_MAX_CHARS
    );
    let mut fx = complete();
    assert_eq!(reply(&iterate(&mut fx, request)), started(2));
    let run = fx.run();
    assert_eq!(run.rounds[1].goal, "x\n```\nrm -rf /\n```\nyz!");
    let wake = run.orch.request_wake.clone().unwrap();
    assert!(
        wake.ends_with("Their request:\n````\nx\n```\nrm -rf /\n```\nyz!\n````\n"),
        "{wake}"
    );
    assert!(!wake.contains('\u{202E}') && !wake.contains('\u{200D}'));
}

/// KG §2.1: twenty rounds at most; the 20th is still accepted.
#[test]
fn the_twenty_first_round_is_refused() {
    let mut fx = complete();
    pad_rounds(fx.run_mut(), 19);
    assert_eq!(reply(&iterate(&mut fx, "the 20th")), started(20));
    assert_eq!(fx.run().rounds.len(), 20);
    fx.run_mut().state = RunState::Complete;
    fx.run_mut().rounds[19].outcome = Some(RoundOutcome::Completed);
    let effects = iterate(&mut fx, "the 21st");
    assert_eq!(
        reply(&effects),
        Err("run 3f9a has had 20 rounds; accept or discard it and start a new goal".into())
    );
    assert_eq!(fx.run().rounds.len(), 20);
}

/// Decision 10, step 1: a blank or too long request.
#[test]
fn a_blank_or_long_request_is_refused() {
    let mut fx = complete();
    assert_eq!(
        reply(&iterate(&mut fx, " \n\t")),
        Err("goal: must not be blank".into())
    );
    let long = "x".repeat(proto::GOAL_MAX_CHARS + 1);
    assert_eq!(
        reply(&iterate(&mut fx, &long)),
        Err("the request is longer than its 16,384-character limit".into())
    );
    let exact = "x".repeat(proto::GOAL_MAX_CHARS);
    assert_eq!(reply(&iterate(&mut fx, &exact)), started(2));
}

/// Decision 30: the orchestrator's `iterate` is decision 10's path with origin
/// `orchestrator`, alone in its call, and decision 9's refusals.
#[test]
fn the_orchestrators_iterate_edit_is_checked_by_the_engine() {
    let mut fx = complete();
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"iterate": "add docs"})));
    assert!(ok, "{value}");
    assert_eq!(value["accepted"], true);
    let run = fx.run();
    assert_eq!(run.state, RunState::Planning);
    assert_eq!(run.rounds[1].origin, RoundOrigin::Orchestrator);
    assert_eq!(run.rounds[1].goal, "add docs");
    let logged = run.plan_edits.last().unwrap();
    assert_eq!(
        (
            logged.text.as_str(),
            logged.source.as_str(),
            logged.accepted
        ),
        ("iterate", "orchestrator", true)
    );
    let alone = "iterate must be the only thing in its edit_plan call";
    for extra in [
        json!({"iterate": "x", "edits": [add("t2", "mail")]}),
        json!({"iterate": "x", "submit": true}),
        json!({"iterate": "x", "summary": "done"}),
    ] {
        let mut fx = complete();
        assert_eq!(error(&edit_plan(&mut fx, extra.clone())), alone, "{extra}");
        assert_eq!(fx.run().rounds.len(), 1);
    }
    let mut fx = approved();
    assert_eq!(
        error(&edit_plan(&mut fx, json!({"iterate": "x"}))),
        "run 3f9a is running; iterate it when it completes"
    );
    let logged = fx.run().plan_edits.last().unwrap().clone();
    assert!(!logged.accepted);
}

/// Decision 30: `iterate` inside an edit batch is refused, in each source's words.
#[test]
fn iterate_inside_edits_is_refused_from_every_source() {
    let op = json!({"op": "iterate", "goal": "more"});
    // The user's `run edit`.
    let mut fx = approved();
    let goal = "more".to_string();
    let effects = edit(&mut fx, vec![PlanEdit::Iterate { goal }]);
    assert_eq!(
        replies(&effects),
        vec![Err("use anthrex run iterate to start a round".to_string())]
    );
    // The orchestrator's `edits` array.
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [op.clone()]})));
    assert!(!ok);
    assert_eq!(
        value["errors"][0]["message"],
        "iterate goes in edit_plan's iterate, alone in its call"
    );
    // A sub-planner's `submit_epic`.
    let mut fx = planning_mail(false);
    let effects = submit_epic(&mut fx, json!([task_in("m1", "mail"), op]));
    assert_eq!(error(&effects), "a sub-planner cannot iterate a run");
    assert_eq!(fx.run().rounds.len(), 1);
}

/// Decision 10, step 6: a dormant orchestrator (after a restart) is relaunched; the
/// round wake waits on the run until its window is live, then goes with the request.
#[test]
fn a_dormant_orchestrator_is_relaunched_for_a_round() {
    let mut fx = complete();
    crate::run::engine::orch_window::restored(fx.run_mut());
    let effects = iterate(&mut fx, "more");
    assert_eq!(reply(&effects), started(2));
    assert!(wakes(&effects).is_empty(), "not live: no wake yet");
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    let wake = round_wake(2, 1, "more");
    assert_eq!(fx.run().orch.request_wake.as_deref(), Some(wake.as_str()));
    let effects = fx.done(restarts[0].0, OpResult::Restarted);
    let woken = wakes(&effects);
    assert_eq!(woken.len(), 1, "{effects:#?}");
    assert!(woken[0].0.starts_with(&wake) && woken[0].1);
}

/// A `pr` run's `running` iterate leaves its delivery records as they are.
#[test]
fn a_delivering_pr_run_keeps_its_records() {
    let mut fx = pr_delivering();
    let delivery = fx.run().delivery.clone();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert_eq!(fx.run().delivery, delivery);
}

#[test]
fn an_unknown_run_is_refused() {
    let mut fx = complete();
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Iterate {
        reply: reply_id,
        run_id: "nope".into(),
        goal: "more".into(),
    });
    assert_eq!(reply(&effects), Err("unknown run nope".into()));
}
