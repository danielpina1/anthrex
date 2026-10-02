//! Milestone 9.2 task M9.2.7: what a `pr`-mode run refuses and how it ends. `run
//! accept` is refused in `pr` mode and `run discard` until the run is complete
//! (decisions 38–39, ruling R-1), in the request and in the action menu alike;
//! `run cancel` closes nothing and completes with the PRs it left open (decision 39);
//! `run deliver` and `run watch` (decision 25); completion waits for every stage PR to
//! land, without 9.1's tier 3 (decision 37, ruling R-8).

use proto::{ActionKind, FinishAction, PrState, RunState, TaskState};

use super::delivery_open::{answer, deliver, green, host_ops, host_ops_in, open_stage, pr_on, url};
use super::dispatch::replies;
use super::fixture::*;
use super::full::{full_jobs, merge_tiered, profile};
use super::merge::{commit, doc_task, merge, pending, pending_one, start_on, to_queue, window_of};
use super::propagate::land_propagates;
use crate::run::delivery::ops::HostResult;
use crate::run::engine::actions::{ActionNode, available, check};
use crate::run::engine::delivery::{DeliveryRequest, cancel_outcome};
use crate::run::engine::{AgentSignal, Effect, EventKind, OpKind, OpResult};

const ACCEPT_REFUSED: &str = "run engine-test-3f9a is delivered by pull request; merge its pull requests on GitHub (anthrex never merges)";
const DISCARD_REFUSED: &str = "run engine-test-3f9a is delivered by pull request; its branches back its pull requests, so discard is refused (anthrex run cancel stops its agents)";

fn finish(fx: &mut Fixture, action: FinishAction) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
    })
}

fn watch(fx: &mut Fixture, on: bool) -> Result<String, String> {
    let reply = fx.reply();
    let request = DeliveryRequest::Watch {
        reply,
        run_id: RUN_ID.into(),
        on,
    };
    let effects = fx.next(EventKind::Delivery(request));
    replies(&effects).remove(0)
}

fn kinds(fx: &Fixture) -> Vec<ActionKind> {
    available(fx.run(), &ActionNode::Run)
        .into_iter()
        .map(|a| a.kind)
        .collect()
}

/// A `pr`-mode `Single` run whose `t1` merged at `commit(1)` and whose PR #7 is open.
fn delivered() -> Fixture {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 7);
    fx
}

/// The user lands stage `n`'s PR on GitHub (what M9.2.11's watch will record).
fn land(fx: &mut Fixture, n: u16, state: PrState) -> Vec<Effect> {
    let stage = fx
        .run_mut()
        .delivery
        .stages
        .get_mut(usize::from(n) - 1)
        .unwrap();
    stage.pr.as_mut().unwrap().state = state;
    fx.tick()
}

fn verify_ok(fx: &mut Fixture) -> Vec<Effect> {
    let (op, _) = pending_one(fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk)
}

#[test]
fn run_accept_and_discard_are_refused_in_pr_mode() {
    let mut fx = delivered();
    // Running: both refused, nothing emitted, neither listed.
    for (action, text) in [
        (FinishAction::Accept, ACCEPT_REFUSED),
        (FinishAction::Discard, DISCARD_REFUSED),
    ] {
        let effects = finish(&mut fx, action);
        assert_eq!(replies(&effects), vec![Err(text.to_string())]);
        assert!(ops_in(&effects, "Accept").is_empty() && ops_in(&effects, "Discard").is_empty());
    }
    let listed = kinds(&fx);
    assert!(!listed.contains(&ActionKind::Accept), "{listed:?}");
    assert!(!listed.contains(&ActionKind::Discard), "{listed:?}");
    let run = fx.run();
    assert_eq!(
        check(run, &ActionNode::Run, &ActionKind::Accept),
        Err(ACCEPT_REFUSED.into())
    );
    assert_eq!(
        check(run, &ActionNode::Run, &ActionKind::Discard),
        Err(DISCARD_REFUSED.into())
    );

    // Complete (its PR merged): accept is still refused and never listed; discard is
    // listed and allowed, locally only (ruling R-1): one `Discard`, no host op.
    land(&mut fx, 1, PrState::Merged);
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    let effects = finish(&mut fx, FinishAction::Accept);
    assert_eq!(replies(&effects), vec![Err(ACCEPT_REFUSED.to_string())]);
    assert!(ops_in(&effects, "Accept").is_empty());
    let listed = available(fx.run(), &ActionNode::Run);
    let kinds: Vec<&ActionKind> = listed.iter().map(|a| &a.kind).collect();
    assert_eq!(kinds, vec![&ActionKind::Discard]);
    assert_eq!(listed[0].refused_why, None);
    let effect = &listed[0].effect;
    let local = format!(
        "discard: remove the run's local worktrees and anthrex/{RUN_ID}/* branches, never a PR or a remote branch ("
    );
    assert!(effect.starts_with(&local), "{effect}");
    let effects = finish(&mut fx, FinishAction::Discard);
    assert!(
        replies(&effects).is_empty(),
        "answered with the op's result"
    );
    let discards = ops_in(&effects, "Discard");
    assert_eq!(discards.len(), 1);
    let OpKind::Discard { branch_prefix, .. } = &discards[0].1 else {
        unreachable!()
    };
    assert_eq!(branch_prefix, &format!("anthrex/{RUN_ID}/"));
    assert!(
        host_ops_in(&effects).is_empty(),
        "the discard calls no host"
    );
}

#[test]
fn a_local_complete_run_still_lists_accept_and_discard() {
    let (mut fx, windows) = start_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(kinds(&fx), vec![ActionKind::Accept, ActionKind::Discard]);
}

#[test]
fn cancel_leaves_prs_open_and_completes_with_the_outcome() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 7);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert!(host_ops_in(&effects).is_empty(), "nothing is closed");
    assert!(!fx.run().delivery.watching, "watching stops");
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    let t2 = window_of(&windows, "t2");
    fx.signal(
        t2,
        AgentSignal::ProcessExited {
            code: None,
            killed_by_engine: true,
            pid: 0,
        },
    );
    let (op, _) = pending_one(&fx, "RemoveWorktree", Some("t2"));
    fx.done(op, OpResult::Removed { salvage_ref: None });
    // Stage 2 merged nothing and the run is cancelled: no PR waits.
    verify_ok(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    let outcome = "cancelled; 1 pull request left open: #7";
    assert_eq!(run.outcome.as_deref(), Some(outcome));
    assert!(run.log.iter().any(|l| l.text == outcome));
    assert_eq!(run.delivery.pr(1).unwrap().state, PrState::Open);
    assert!(
        ops_in(&fx.log, "Host").len() == 2,
        "the push and the open only"
    );
}

#[test]
fn cancel_outcome_lists_every_open_pr() {
    let mut fx = delivered();
    fx.run_mut().cancelled = true;
    assert_eq!(
        cancel_outcome(fx.run()).as_deref(),
        Some("cancelled; 1 pull request left open: #7")
    );
    let mut second = fx.run().delivery.stages[0].clone();
    second.pr.as_mut().unwrap().number = 9;
    fx.run_mut().delivery.stages.push(second);
    assert_eq!(
        cancel_outcome(fx.run()).as_deref(),
        Some("cancelled; 2 pull requests left open: #7, #9")
    );
    for s in fx.run_mut().delivery.stages.iter_mut() {
        s.pr.as_mut().unwrap().state = PrState::Merged;
    }
    assert_eq!(cancel_outcome(fx.run()).as_deref(), Some("cancelled"));
    // Not cancelled, or local: no outcome of its own.
    fx.run_mut().cancelled = false;
    assert_eq!(cancel_outcome(fx.run()), None);
}

#[test]
fn deliver_and_watch_requests_and_their_refusals() {
    // Local mode refuses both.
    let (mut fx, _) = start_on(PROFILE, &[doc_task("t1", "")]);
    let local = "run engine-test-3f9a delivers locally; run deliver and run watch apply to pr mode";
    assert_eq!(deliver(&mut fx, 1), Err(local.into()));
    assert_eq!(watch(&mut fx, false), Err(local.into()));

    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    let none = "stage 3 is not ready: it has no stage 3";
    assert_eq!(deliver(&mut fx, 3), Err(none.into()));
    let none = "stage 0 is not ready: it has no stage 0";
    assert_eq!(deliver(&mut fx, 0), Err(none.into()));
    let unfinished = "stage 1 is not ready: 1 of its tasks are not finished";
    assert_eq!(deliver(&mut fx, 1), Err(unfinished.into()));
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    // Ready: tier 3 is (already) requested on the head; the PR opens when it is green.
    let accepted = "stage 1: tier 3 requested; its PR opens when tier 3 is green";
    assert_eq!(deliver(&mut fx, 1), Ok(accepted.into()));
    assert_eq!(full_jobs(&fx.log).len(), 1, "one tier-3 job");
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 7);
    let open = format!("stage 1's PR is already open: {}", url(7));
    assert_eq!(deliver(&mut fx, 1), Ok(open));
    // A stage whose lower PR was closed without merging.
    let stage = &mut fx.run_mut().delivery.stages[0];
    stage.pr.as_mut().unwrap().state = PrState::Closed;
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    let closed = "stage 2 is not ready: stage 1's PR was closed without merging";
    assert_eq!(deliver(&mut fx, 2), Err(closed.into()));
    assert!(host_ops(&fx).is_empty() && pending(&fx, "Tier", None).is_empty());
    let stage = &mut fx.run_mut().delivery.stages[0];
    stage.pr.as_mut().unwrap().state = PrState::Open;

    // Watch: off stops polling; on makes every open PR due at once.
    let off = "stopped watching run engine-test-3f9a's pull requests; anthrex run watch engine-test-3f9a --on resumes";
    assert_eq!(watch(&mut fx, false), Ok(off.into()));
    assert!(!fx.run().delivery.watching);
    fx.run_mut().delivery.stages[0]
        .pr
        .as_mut()
        .unwrap()
        .next_poll_at = u64::MAX;
    let on = "watching run engine-test-3f9a's pull requests";
    assert_eq!(watch(&mut fx, true), Ok(on.into()));
    assert!(fx.run().delivery.watching);
    let due = fx.run().delivery.pr(1).unwrap().next_poll_at;
    assert_eq!(due, fx.now, "due at once");

    // A paused run takes no `run deliver`; a terminal one neither request.
    fx.run_mut().state = RunState::Paused;
    let paused = "run engine-test-3f9a is paused";
    assert_eq!(deliver(&mut fx, 2), Err(paused.into()));
    fx.run_mut().state = RunState::Discarded;
    let ended = "run engine-test-3f9a is discarded";
    assert_eq!(deliver(&mut fx, 2), Err(ended.into()));
    assert_eq!(watch(&mut fx, true), Err(ended.into()));

    let reply = fx.reply();
    let request = DeliveryRequest::Watch {
        reply,
        run_id: "nope".into(),
        on: true,
    };
    let effects = fx.next(EventKind::Delivery(request));
    assert_eq!(replies(&effects), vec![Err("unknown run nope".to_string())]);
}

#[test]
fn completion_waits_for_every_pr_and_skips_tier_3_in_pr_mode() {
    let (mut fx, windows) = pr_on(&profile(), &[doc_task("t1", "")]);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 7);
    // Every task is finished and the queue is empty, but the PR is open.
    fx.tick();
    assert!(
        pending(&fx, "VerifyRefs", None).is_empty(),
        "the PR is open"
    );
    // A background tier 3 found the head red: in local mode that would hold
    // completion; in pr mode CI on the PR is the authority.
    let head = fx.run().run_head.clone();
    let s = &mut fx.run_mut().stages[0];
    s.full.green_at = None;
    s.full.red_at = Some(head);
    land(&mut fx, 1, PrState::Merged);
    let effects = verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(full_jobs(&effects).is_empty(), "no tier 3 at completion");
    assert!(
        ops_in(&effects, "Check").is_empty(),
        "no final check either"
    );
    assert_eq!(fx.run().outcome, None);
}

#[test]
fn a_closed_or_skipped_stage_counts_as_landed() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", ""), doc_task("t2", "stage = 2")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 7);
    to_queue(&mut fx, "t2", window_of(&windows, "t2"));
    merge(&mut fx, "t2", &commit(2));
    green(&mut fx, 2);
    open_stage(&mut fx, 2, 8);
    land(&mut fx, 1, PrState::Merged);
    assert!(
        pending(&fx, "VerifyRefs", None).is_empty(),
        "stage 2 is open"
    );
    land(&mut fx, 2, PrState::Closed);
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
}

#[test]
fn a_host_error_is_retried_after_the_poll_interval() {
    let mut fx = delivered_until_push();
    let (op, _) = super::delivery_open::host_op(&fx);
    let error = crate::host::HostError::TimedOut("git push did not answer".into());
    answer(&mut fx, op, HostResult::Error(error));
    assert!(host_ops(&fx).is_empty(), "not retried at once");
    assert_eq!(fx.run().delivery.failures.get("1/push"), Some(&1));
    let line = "stage 1: push failed: git push did not answer";
    assert!(fx.run().log.iter().any(|l| l.text == line));
    let wait = fx.run().delivery.poll_base_secs;
    let effects = fx.send(fx.now + wait, EventKind::Tick);
    assert_eq!(host_ops_in(&effects).len(), 1, "retried once due");
    let (op, _) = super::delivery_open::host_op(&fx);
    answer(
        &mut fx,
        op,
        HostResult::Pushed(crate::host::PushOutcome::Pushed),
    );
    assert_eq!(
        fx.run().delivery.failures.get("1/push"),
        None,
        "a success resets it"
    );
}

#[test]
fn a_rejected_push_halts_and_a_forbidden_command_halts() {
    let mut fx = delivered_until_push();
    let (op, _) = super::delivery_open::host_op(&fx);
    let rejected = crate::host::PushOutcome::Rejected {
        reason: "non-fast-forward".into(),
    };
    answer(&mut fx, op, HostResult::Pushed(rejected));
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    let rewritten = "remote stage branch was rewritten by someone else";
    assert_eq!(run.halted_reason.as_deref(), Some(rewritten));
    assert!(!run.halt_retryable);

    let mut fx = delivered_until_push();
    let (op, _) = super::delivery_open::host_op(&fx);
    let reason = format!("the remote refused the push of anthrex/{RUN_ID}/stage-1: protected");
    let refused = crate::host::PushOutcome::Refused {
        reason: reason.clone(),
    };
    answer(&mut fx, op, HostResult::Pushed(refused));
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    assert_eq!(run.halted_reason.as_deref(), Some(reason.as_str()));
    assert!(run.halt_retryable, "run resume pushes again");

    let mut fx = delivered_until_push();
    let (op, _) = super::delivery_open::host_op(&fx);
    let forbidden = crate::host::HostError::Forbidden("anthrex never runs: gh pr merge 1".into());
    answer(&mut fx, op, HostResult::Error(forbidden));
    let text = "anthrex refused its own host command: gh pr merge 1";
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
}

/// A `pr`-mode `Single` run whose stage 1 is green and whose push is pending.
fn delivered_until_push() -> Fixture {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    fx
}
