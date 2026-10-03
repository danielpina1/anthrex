//! Milestone 9.5 decision 46 (FU-F36): in round 2 or later the scouts and window limit
//! texts name the round they count from (milestone 9.3 decision 14); round 1 keeps
//! its texts.

use proto::{BlockReason, TaskState};
use serde_json::json;

use super::fixture::*;
use super::goal_rounds_end::create_stages;
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::orch::{answer, error, launched};
use super::run_scouts::scout;

/// Every task blocked with `text` as its environment block.
fn blocked_with(fx: &Fixture, text: &str) -> Vec<String> {
    let run = fx.run();
    run.tasks
        .iter()
        .filter(|t| t.state == TaskState::Blocked)
        .filter(|t| {
            t.block
                .as_ref()
                .is_some_and(|b| b.reason == BlockReason::Environment && b.text == text)
        })
        .map(|t| t.id().to_string())
        .collect()
}

#[test]
fn limit_texts_name_the_round() {
    // Round 1 (pinning): today's texts.
    let mut fx = launched(false);
    fx.run_mut().limits.orch.max_scouts = 1;
    assert!(answer(&scout(&mut fx, "a")).0);
    assert_eq!(
        error(&scout(&mut fx, "b")),
        format!("run {RUN_ID} already has 1 scouts, the most max_scouts allows")
    );

    // Round 2: its own scouts, counted from its start, and the text names the round.
    let mut fx = complete();
    fx.run_mut().limits.orch.max_scouts = 1;
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert!(answer(&scout(&mut fx, "c")).0);
    assert_eq!(
        error(&scout(&mut fx, "d")),
        format!("round 2 of run {RUN_ID} already has 1 scouts, the most max_scouts allows")
    );
}

/// Round 2's window limit: one window from the round's start, so one of its two tasks
/// is blocked with the round's text (round 1's is pinned by `dispatch_slots.rs` and
/// `goal_rounds_end.rs`).
#[test]
fn a_later_rounds_window_limit_names_the_round() {
    let mut fx = complete();
    fx.run_mut().limits.max_windows = 1;
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(
        &mut fx,
        json!([add_in("t2", "mail", 2, &[]), add_in("t3", "docs", 2, &[])]),
    );
    create_stages(&mut fx);
    fx.complete_prepares();
    let blocked = blocked_with(&fx, "round 2's window limit (1) reached");
    assert_eq!(blocked.len(), 1, "{:#?}", fx.run().tasks);
    assert!(blocked_with(&fx, "run window limit (1) reached").is_empty());
}
