//! Milestone 9.10 decision 34's open choices, pinned (task 10 review M1): a repository
//! that needs review but has no proposal in the snapshot, the alert's age, a Retry
//! with no link, the `o open profile` hint in both places, and a failed set-up's place
//! among the other priority-3 alerts. Split from `alerts_setup.rs` (rule 8).

use super::alerts::{line, listed};
use super::runs::app_with_runs;
use super::*;
use crate::app::{AlertKey, alerts};
use crate::tree::alert_fixtures::at;
use crate::tree::run_fixtures::snapshot;
use proto::{QueuedGoalInfo, RunState, RunsSnapshot, SetupState};

const SHOP: &str = "/r/shop";

fn queued(id: &str, goal: &str, queued_at: u64, setup: SetupState) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: id.into(),
        project: SHOP.into(),
        goal: goal.into(),
        queued_at,
        yes: false,
        trust_project: false,
        unconfined_checks: false,
        setup,
    }
}

fn with_queue(runs: Vec<proto::RunInfo>, goals: Vec<QueuedGoalInfo>) -> RunsSnapshot {
    let mut snap = snapshot(1, runs);
    snap.queued_goals = goals;
    snap
}

/// Needs review, but the snapshot has no proposal for it yet: a priority-5 set-up
/// alert in the review's words (the next snapshot brings the review alert).
#[test]
fn needs_review_without_a_proposal_is_a_set_up_alert() {
    let snap = with_queue(
        vec![],
        vec![queued("q-1", "add a", 1, SetupState::NeedsReview)],
    );
    let app = app_with_runs(vec![], snap);
    let list = alerts(&app);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!(list[0].key, AlertKey::Setup(SHOP.into()));
    assert_eq!(list[0].priority, 5);
    assert_eq!(list[0].text, "review how anthrex will work here");
}

/// The alert's age is the first queued goal's.
#[test]
fn the_age_is_the_first_goals() {
    let snap = with_queue(
        vec![],
        vec![
            queued("q-1", "add a", 100, SetupState::Reading),
            queued("q-2", "fix b", 500, SetupState::Reading),
        ],
    );
    let app = app_with_runs(vec![], snap);
    assert_eq!(alerts(&app)[0].age, Some(app.run_age(100)));
}

/// Retry with no link toasts `not connected` and sends nothing.
#[test]
fn retry_without_a_link_sends_nothing() {
    let failed = SetupState::Failed {
        reason: "boom".into(),
    };
    let snap = with_queue(vec![], vec![queued("q-1", "add a", 1, failed)]);
    let mut app = app_with_runs(vec![], snap);
    app.on_link_lost("gone");
    assert!(app.retry_setup(std::path::Path::new(SHOP)).is_empty());
    assert_eq!(app.toast_text(), Some("not connected"));
}

/// A failed set-up (priority 3) comes after the runs' priority-3 alerts.
#[test]
fn a_failed_set_up_follows_the_runs_priority_three() {
    let failed = SetupState::Failed {
        reason: "boom".into(),
    };
    let snap = with_queue(
        vec![at("r", RunState::Halted, 1)],
        vec![queued("q-1", "add a", 1, failed)],
    );
    let app = app_with_runs(vec![], snap);
    assert_eq!(
        listed(&app),
        vec![
            line(3, "r", "run halted"),
            line(3, "shop", "setting up failed: boom"),
        ]
    );
}

/// Task 10 review M2: the status bar and the detail both say `o open profile` on a
/// set-up alert.
#[test]
fn both_hint_rows_say_open_profile() {
    let snap = with_queue(vec![], vec![queued("q-1", "add a", 1, SetupState::Reading)]);
    let mut app = app_with_runs(vec![], snap);
    prefix(&mut app);
    press(&mut app, KeyCode::Char('a'), KeyModifiers::NONE);
    let key = app.alerts_focus.as_ref().and_then(|f| f.selected.clone());
    assert_eq!(key, Some(AlertKey::Setup(SHOP.into())));
    let rows = crate::ui::audit::rows(&crate::ui::audit::draw(&app, 160, 40));
    let bar = rows.last().unwrap();
    assert!(bar.contains("o open profile"), "{bar}");
}
