//! M8a.12 fix round 4 (ruling T12-R4): the fallback's count and claim belong to the
//! turn they were issued for (OS-1), an interrupted turn stays interrupted after
//! activity cancelled the grace kill (N3-1), and a turn Claude starts by itself counts
//! as a turn (N3-2). Each test ends with the liveness check.

use serde_json::json;

use super::fixture::*;
use super::holds::delivers;
use super::turns::{exited, queue, working_on};
use super::turns_fixes::assert_alive;
use crate::run::contract::{DONE_NUDGE, NO_COMMIT_NUDGE, RESUME_AFTER_EXIT, stall_nudge};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpId, OpKind, OpResult};
use crate::run::messages::DELIVERY_RETRY_SECS;
use crate::run::model::FallbackState;

const ROOMY: &str = "[task.budget]\ntool_calls = 1000\nminutes = 1000";
const CODEX_ROOMY: &str = "[task.route]\nruntime = \"codex\"\nmodel = \"\"\n[task.budget]\ntool_calls = 1000\nminutes = 1000";

fn args() -> serde_json::Value {
    json!({"summary": "did it", "test": "a::works", "red": "abcdef1"})
}

fn commits(count: u32) -> OpResult {
    OpResult::Commits {
        count,
        head: HEAD.into(),
    }
}

fn dirty(fx: &Fixture) -> OpResult {
    let mut result = fx.clean_check("t1");
    if let OpResult::DoneChecked { dirty_tracked, .. } = &mut result {
        *dirty_tracked = 2;
    }
    result
}

fn undelivered(fx: &Fixture) -> Vec<String> {
    fx.run()
        .outbox
        .iter()
        .filter(|m| m.delivered_at.is_none())
        .map(|m| m.text.clone())
        .collect()
}

/// The delivery in `effects` fails ("busy"): its messages wait for the retry.
fn fail_delivery(fx: &mut Fixture, effects: &[Effect]) {
    let ids = effects
        .iter()
        .find_map(|e| match e {
            Effect::Deliver { message_ids, .. } => Some(message_ids.clone()),
            _ => None,
        })
        .expect("a delivery");
    fx.next(EventKind::Delivered {
        run_id: RUN_ID.into(),
        message_ids: ids,
        ok: false,
        error: Some("busy".into()),
    });
}

fn count_of(effects: &[Effect]) -> OpId {
    let counts = ops_in(effects, "CountCommits");
    assert_eq!(counts.len(), 1, "one count: {effects:#?}");
    counts[0].0
}

/// The later turn's end runs the fallback afresh: its count finds no commit, and the
/// no-commit nudge goes out; no stall.
fn fresh_fallback_nudges(fx: &mut Fixture, effects: &[Effect]) {
    let count = count_of(effects);
    let effects = fx.done(count, commits(0));
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    assert_eq!(fx.task("t1").stalls, 0);
    assert_alive(fx);
}

/// OS-1 (probe RD, in flight): a count that lands while a later turn is open is that
/// earlier turn's, and is dropped; the later turn's end counts afresh. Before the fix
/// the count's nudge was queued mid-turn and the next 0 was a stall.
#[test]
fn a_count_that_lands_in_a_later_turn_is_dropped() {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    queue(&mut fx, "[anthrex] go on");
    assert_eq!(delivers(&fx.tick()).len(), 1);
    fx.done(count, commits(0));
    assert!(undelivered(&fx).is_empty(), "{:?}", undelivered(&fx));
    assert_eq!(fx.task("t1").rounds[0].fallback, FallbackState::None);
    assert_alive(&fx);
    let effects = fx.turn_completed(w);
    fresh_fallback_nudges(&mut fx, &effects);
}

/// OS-1: a count dropped after the later turn has already ended leaves nothing pending
/// unless the fallback runs again there and then.
#[test]
fn a_count_dropped_after_the_later_turn_ended_counts_again() {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    queue(&mut fx, "[anthrex] go on");
    fx.tick();
    // One count at a time: the later turn's end waits for the one in flight.
    assert!(ops_in(&fx.turn_completed(w), "CountCommits").is_empty());
    let effects = fx.done(count, commits(0));
    fresh_fallback_nudges(&mut fx, &effects);
}

/// OS-1: a count dropped while a message waits for its delivery retry does not count
/// again: that message's turn is coming, and its end counts.
#[test]
fn a_count_dropped_while_a_message_waits_counts_at_that_turns_end() {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    queue(&mut fx, "[anthrex] go on");
    fx.tick();
    fx.turn_completed(w);
    queue(&mut fx, "[anthrex] and this");
    let effects = fx.tick();
    fail_delivery(&mut fx, &effects);
    assert!(!fx.task("t1").rounds[0].turn_open);
    let effects = fx.done(count, commits(0));
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(undelivered(&fx), vec!["[anthrex] and this".to_string()]);
    assert_alive(&fx);
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert_eq!(delivers(&effects).len(), 1);
    let effects = fx.turn_completed(w);
    fresh_fallback_nudges(&mut fx, &effects);
}

/// R4-1 (probe PA; ruling T12-R5): `NO_COMMIT_NUDGE`'s delivery fails and waits for its
/// retry while Claude starts and ends a turn by itself. That turn's end does not count
/// again while the nudge is unread; the nudge goes out at the retry, and its turn's end
/// counts.
#[test]
fn no_count_while_the_nudge_waits_for_its_delivery_retry() {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    let effects = fx.done(count, commits(0));
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    fail_delivery(&mut fx, &effects);
    fx.signal(w, AgentSignal::TurnStarted);
    let effects = fx.turn_completed(w);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(fx.task("t1").stalls, 0);
    assert_eq!(undelivered(&fx), vec![NO_COMMIT_NUDGE.to_string()]);
    assert_alive(&fx);
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert_eq!(delivers(&effects), vec![NO_COMMIT_NUDGE.to_string()]);
    let count = count_of(&fx.turn_completed(w));
    fx.done(count, commits(0));
    assert_eq!(
        fx.task("t1").stalls,
        1,
        "the read nudge's empty turn is the stall"
    );
}

/// OS-1 (probe RD, retry): a failed count's retry is its turn's; once a later turn has
/// started it is not sent, and that turn's end counts afresh.
#[test]
fn a_retried_count_is_not_sent_into_a_later_turn() {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    fx.done(
        count,
        OpResult::Failed {
            message: "git broke".into(),
        },
    );
    queue(&mut fx, "[anthrex] go on");
    fx.tick();
    let effects = fx.send(fx.now + DELIVERY_RETRY_SECS, EventKind::Tick);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert!(undelivered(&fx).is_empty());
    assert_alive(&fx);
    let effects = fx.turn_completed(w);
    fresh_fallback_nudges(&mut fx, &effects);
}

/// The fallback's own claim: two commits, `DONE_NUDGE`'s turn ends, the claim goes out.
fn fallback_claim() -> (Fixture, u32, OpId) {
    let (mut fx, w) = working_on(ROOMY);
    let count = count_of(&fx.turn_completed(w));
    let effects = fx.done(count, commits(2));
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);
    let effects = fx.turn_completed(w);
    let verify = ops_in(&effects, "VerifyDone");
    assert_eq!(verify.len(), 1, "{effects:#?}");
    (fx, w, verify[0].0)
}

/// OS-1: the fallback's claim answered while a later turn is open is dropped: no
/// rejection is queued into that turn, and its end runs the fallback afresh.
#[test]
fn a_fallback_claim_answered_in_a_later_turn_is_dropped() {
    let (mut fx, w, verify) = fallback_claim();
    queue(&mut fx, "[anthrex] meanwhile");
    assert_eq!(delivers(&fx.tick()).len(), 1);
    let result = dirty(&fx);
    fx.done(verify, result);
    assert!(fx.task("t1").claim.is_none());
    assert!(undelivered(&fx).is_empty(), "{:?}", undelivered(&fx));
    assert_alive(&fx);
    let effects = fx.turn_completed(w);
    count_of(&effects);
    assert_alive(&fx);
}

/// OS-1: the same after the later turn has ended (its end skipped the fallback while
/// the claim was in flight): the drop runs the fallback at once.
#[test]
fn a_fallback_claim_dropped_after_the_later_turn_ended_counts_again() {
    let (mut fx, w, verify) = fallback_claim();
    queue(&mut fx, "[anthrex] meanwhile");
    fx.tick();
    fx.turn_completed(w);
    let result = dirty(&fx);
    let effects = fx.done(verify, result);
    assert!(fx.task("t1").claim.is_none());
    assert!(undelivered(&fx).is_empty(), "{:?}", undelivered(&fx));
    count_of(&effects);
    assert_alive(&fx);
}

/// OS-1 (probe OD): a rejection queued into a later turn. That turn's end sends the
/// rejection and holds the count back while it is undelivered (ruling T12-R5; in round
/// 4 the count went out and was dropped as stale), so the rejection's turn end counts,
/// and `DONE_NUDGE` follows it rather than racing the fallback's own claim.
#[test]
fn the_count_after_a_late_rejection_waits_for_the_rejection_turn() {
    let (mut fx, w) = working_on(ROOMY);
    let effects = fx.tool(w, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    queue(&mut fx, "[anthrex] meanwhile");
    assert_eq!(delivers(&fx.turn_completed(w)).len(), 1);
    let result = dirty(&fx);
    fx.done(verify, result);
    let effects = fx.turn_completed(w);
    assert_eq!(delivers(&effects).len(), 1, "the rejection");
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_alive(&fx);
    let effects = fx.turn_completed(w);
    assert!(ops_in(&effects, "VerifyDone").is_empty(), "{effects:#?}");
    assert!(fx.task("t1").claim.is_none());
    let count = count_of(&effects);
    let effects = fx.done(count, commits(2));
    assert_eq!(delivers(&effects), vec![DONE_NUDGE.to_string()]);
    assert_alive(&fx);
}

/// N3-1 (probe OB): an interrupt, one event (which cancels the grace kill), then a
/// completed end: the interrupted turn's end sends the stall nudge only, never the
/// fallback's count.
#[test]
fn an_interrupted_turn_with_activity_gets_the_nudge_not_the_fallback() {
    let (mut fx, w) = working_on(ROOMY);
    let stall = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    let effects = fx.send(quiet + stall + 1, EventKind::Tick);
    assert!(effects.contains(&Effect::Interrupt { window_id: w }));
    fx.signal(w, AgentSignal::Activity);
    let effects = fx.turn_completed(w);
    assert!(ops_in(&effects, "CountCommits").is_empty(), "{effects:#?}");
    assert_eq!(delivers(&effects), vec![stall_nudge(stall / 60)]);
    assert_alive(&fx);
}

/// N3-1 for Codex, whose interrupt ends the turn with an exit: activity in the grace
/// does not turn that exit into a death resumed with `RESUME_AFTER_EXIT`.
#[test]
fn a_codex_interrupt_exit_after_activity_is_the_turns_end() {
    let (mut fx, w) = working_on(CODEX_ROOMY);
    fx.signal(
        w,
        AgentSignal::Init {
            session_id: "thread-1".into(),
        },
    );
    let stall = fx.run().limits.stall_after_secs;
    let quiet = fx.task("t1").rounds[0].last_event;
    fx.send(quiet + stall + 1, EventKind::Tick);
    fx.signal(w, AgentSignal::Activity);
    let effects = exited(&mut fx, w);
    let resumes: Vec<String> = ops_in(&effects, "ResumeSession")
        .into_iter()
        .map(|(_, k)| match k {
            OpKind::ResumeSession { message, .. } => message,
            _ => unreachable!(),
        })
        .collect();
    assert!(
        !resumes.contains(&RESUME_AFTER_EXIT.to_string()),
        "{effects:#?}"
    );
    assert_eq!(fx.task("t1").rounds[0].deaths, 0);
    assert_eq!(delivers(&effects), vec![stall_nudge(stall / 60)]);
    assert_alive(&fx);
}

/// N3-2 (probe OC): a verdict during a turn Claude started by itself (a background
/// sub-agent finished) is a later turn's: the rejection is queued for its end.
#[test]
fn a_verdict_during_a_self_started_turn_is_queued_for_its_end() {
    let (mut fx, w) = working_on(ROOMY);
    let effects = fx.tool(w, "task_done", args());
    let (verify, _) = ops_in(&effects, "VerifyDone")[0].clone();
    fx.turn_completed(w);
    let turns = fx.task("t1").rounds[0].turns;
    fx.signal(w, AgentSignal::TurnStarted);
    assert_eq!(fx.task("t1").rounds[0].turns, turns + 1);
    // A second `TurnStarted` in the same turn counts nothing.
    fx.signal(w, AgentSignal::TurnStarted);
    assert_eq!(fx.task("t1").rounds[0].turns, turns + 1);
    let result = dirty(&fx);
    fx.done(verify, result);
    let queued = undelivered(&fx);
    assert_eq!(queued.len(), 1, "{queued:?}");
    assert!(queued[0].contains("task_done rejected"), "{queued:?}");
    let effects = fx.turn_completed(w);
    assert_eq!(delivers(&effects).len(), 1, "{effects:#?}");
    assert_alive(&fx);
}
