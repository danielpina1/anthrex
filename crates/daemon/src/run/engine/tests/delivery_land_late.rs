//! Milestone 9.7's final fix wave, FW-1 and FW-2 (review A I1, I2): work that reaches a
//! stage after its PR merged. A task merge onto a merged stage is judged late, as a
//! deferred cancel's is; a bisect or a tier-3 red on a merged stage with no live stage
//! above adds no fix task; and a lower merged stage's kept fix tasks are cancelled once
//! the stage above merges too.

use proto::TaskState;

use super::bisect::{TEST, answer as probes, with_orchestrator};
use super::delivery_ci_repro::tiered_watched;
use super::delivery_land::{ci_fix, head, logged, merged_view};
use super::delivery_land_merged::one_stage_with_a_fix;
use super::delivery_open::{answer, host_ops};
use super::delivery_watch::{PR, poll_with};
use super::delivery_watch_adopt::{poll_stage, two_stages};
use super::fixture::*;
use super::full::{full_job, outcome, tier};
use super::merge::{commit, merge, to_queue, window_of};
use crate::host::FetchOutcome;
use crate::run::contract::sha7;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::full::{FullWhy, holds_completion, request};
use crate::run::model::FixOf;

/// A tier-3 job on stage 1's head of `tiered_watched(["t1", "t2", "t3"])`, PR #7 open
/// and green on that head: the idle trigger has no delivery filter (ruling T9-1).
fn tier3_after_open() -> Fixture {
    let mut fx = tiered_watched(&["t1", "t2", "t3"]);
    with_orchestrator(&mut fx);
    fx.run_mut().stages[0].full.green_at = None;
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(request(fx.run_mut(), 1, FullWhy::Idle, now, &mut effects));
    fx
}

/// PR #7 merged at stage 1's head (delivered), and the base fetch answered.
fn merge_pr7(fx: &mut Fixture) {
    let at = head(fx, 1);
    poll_with(fx, merged_view(PR, &at, &commit(90)));
    let fetch = host_ops(fx)
        .into_iter()
        .find(|(_, op)| matches!(op, HostOp::Fetch { stage: None, .. }));
    if let Some((op, _)) = fetch {
        let outcome = FetchOutcome::Fetched {
            sha: commit(91),
            parents: Some(1),
            contains: None,
        };
        answer(fx, op, HostResult::Fetched(outcome));
    }
}

fn bisect_fixes(fx: &Fixture) -> Vec<String> {
    (fx.run().tasks.iter())
        .filter(|t| matches!(t.fixes, Some(FixOf::Bisect { .. })))
        .map(|t| t.id().to_string())
        .collect()
}

/// FW-1 (2): a bisect still open when its top stage's PR merges names its culprit, but
/// adds no fix task, which nothing could deliver; it ends with the line.
#[test]
fn a_bisect_open_when_its_top_stage_merges_adds_no_fix() {
    let mut fx = tier3_after_open();
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    assert!(
        super::super::bisect::bisecting(fx.run()),
        "{:#?}",
        fx.run().log
    );
    merge_pr7(&mut fx);
    probes(&mut fx, 2);
    assert!(bisect_fixes(&fx).is_empty(), "{:#?}", fx.run().tasks);
    assert!(
        logged(&fx, "stage 1 PR merged; the culprit's fix is not added"),
        "{:#?}",
        fx.run().log
    );
    assert!(!super::super::bisect::bisecting(fx.run()));
    let now = fx.now;
    assert!(!holds_completion(fx.run(), now), "its red holds nothing");
}

/// FW-1 (3): a tier-3 red that comes back after its top stage's PR merged is ignored:
/// no bisect, no red mark, nothing woken, and completion is not held.
#[test]
fn a_tier_3_red_on_a_merged_top_stage_is_ignored() {
    let mut fx = tier3_after_open();
    let (op, _) = full_job(&fx);
    merge_pr7(&mut fx);
    super::wake_notes::clear(&mut fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    assert!(!super::super::bisect::bisecting(fx.run()));
    assert!(bisect_fixes(&fx).is_empty());
    assert_eq!(fx.run().stages[0].full.red_at, None);
    assert!(super::wake_notes::notes(&fx).is_empty());
    let line = format!("stage 1: tier 3 red ({TEST}); its PR merged, so it is not bisected");
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    let now = fx.now;
    assert!(!holds_completion(fx.run(), now));
    // Nor does tier 3 run again on the merged stage.
    super::full::later(&mut fx, 3_600);
    assert!(super::merge::pending(&fx, "Tier", None).is_empty());
}

/// FW-1 (1): a fix task added after its top stage's PR merged (any path) that then
/// lands is judged late: its work is not delivered, with the attention line.
#[test]
fn a_fix_that_lands_on_a_merged_top_stage_says_it_is_not_delivered() {
    let (mut fx, _, _) = one_stage_with_a_fix();
    let at = head(&fx, 1);
    poll_stage(&mut fx, 1, merged_view(11, &at, &commit(70)));
    let fix = ci_fix(&mut fx);
    let mut windows = Vec::new();
    for _ in 0..2 {
        windows.extend(fx.launch_all());
    }
    to_queue(&mut fx, &fix, window_of(&windows, &fix));
    let landed = commit(5);
    merge(&mut fx, &fix, &landed);
    assert_eq!(fx.task(&fix).state, TaskState::Merged);
    let unlanded = format!(
        "stage 1 PR #11 was merged at {}, without {}; that work is not delivered (anthrex run cancel gives up)",
        sha7(&at),
        sha7(&landed)
    );
    assert!(logged(&fx, &unlanded), "{:#?}", fx.run().log);
    assert_eq!(fx.run().delivery.alerts.get("1/unlanded"), Some(&unlanded));
}

/// FW-2 (review A I2, the reviewer's probe): stage 1 merges under a live stage 2, so its
/// fix task is kept; once stage 2 merges too, nothing can carry it up, and it is
/// cancelled with stage 1's reason.
#[test]
fn a_lower_merged_stages_kept_fix_is_cancelled_once_the_stage_above_merges() {
    let (mut fx, _) = two_stages(true);
    let fix = ci_fix(&mut fx);
    for _ in 0..2 {
        fx.launch_all();
    }
    assert!(!fx.task(&fix).state.is_finished());
    let h1 = head(&fx, 1);
    poll_stage(&mut fx, 1, merged_view(11, &h1, &commit(70)));
    assert!(!fx.task(&fix).state.is_finished(), "stage 2 is live");
    let h2 = head(&fx, 2);
    poll_stage(&mut fx, 2, merged_view(12, &h2, &commit(80)));
    let task = fx.task(&fix);
    assert_eq!(task.state, TaskState::Cancelled, "{:#?}", task.history);
    assert_eq!(
        task.history.last().map(|e| e.text.as_str()),
        Some("cancelled: stage 1 PR merged")
    );
}

/// FW-7 (review A m5): a review fix whose deferred cancel arrived too late lands after
/// the merge; its reply is queued and dropped at once with the "missed the merge" line,
/// never left unready on the merged stage.
#[test]
fn a_late_review_fix_drops_its_reply_with_the_missed_line() {
    use super::delivery_review::said;
    use super::delivery_review_reply::by_alice;
    use super::delivery_watch::view;
    let mut fx = by_alice();
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Please update the README.")];
    let (at, _) = poll_with(&mut fx, v);
    fx.run_mut().delivery.watching = false;
    fx.send(at + 1, crate::run::engine::EventKind::Tick);
    fx.tick();
    let windows = fx.launch_all();
    to_queue(&mut fx, "fix1", window_of(&windows, "fix1"));
    fx.run_mut().delivery.watching = true;
    poll_with(&mut fx, merged_view(PR, &commit(1), &commit(90)));
    assert!(
        fx.task("fix1").cancel_deferred,
        "{:#?}",
        fx.task("fix1").history
    );
    merge(&mut fx, "fix1", &commit(2));
    let missed = "PR #7 was merged at 1eeeeee before fix task fix1 reached it: the fix missed the merge, so thread 7:c5 gets no reply";
    assert!(logged(&fx, missed), "{:#?}", fx.run().log);
    assert_eq!(
        fx.run().delivery.alerts.get("1/missed").map(String::as_str),
        Some(missed)
    );
    fx.tick();
    let stage = fx.run().delivery.stage(1).unwrap();
    assert!(stage.replies.is_empty(), "{:#?}", stage.replies);
}
