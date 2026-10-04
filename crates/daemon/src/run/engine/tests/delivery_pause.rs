//! The final fix wave's ruling FW-2 (b): a run's pause does not stop a pull request's
//! wait on a person (milestone 9.2 decision 43), so the stage's `human_review_secs`, as
//! the snapshot shows it and history records it, counts the pause; only the estimate
//! leaves the overlap out, once (`pause_tests.rs`).

use proto::{PlanEdit, RunState};

use super::delivery_land::{head, merged_view, stage_lines};
use super::delivery_sync::fetched;
use super::delivery_watch::{PR, poll_with, watched};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::merge::commit;
use crate::run::engine::EventKind;

#[test]
fn a_pause_does_not_stop_the_human_review_wait() {
    let mut fx = watched();
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let secs = |fx: &Fixture| fx.run().delivery.stage(1).unwrap().review_wait_secs;
    let shown = |fx: &Fixture| {
        crate::run::delivery::snapshot::stage_pr_info(fx.run(), 1)
            .unwrap()
            .human_review_secs
    };
    let opened = fx.run().delivery.pr(1).unwrap().opened_at;
    fx.send(opened + 10, EventKind::Tick);
    assert_eq!(secs(&fx), 10);
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Paused);
    fx.now = opened + 110;
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    assert_eq!(fx.run().paused_secs, 100);
    fx.send(opened + 120, EventKind::Tick);
    assert_eq!(secs(&fx), 120, "the pause is still a person's wait");
    assert_eq!(shown(&fx), 120);
    // Merged: history records the same wait, the pause in it.
    let at = head(&fx, 1);
    let (merged, _) = poll_with(&mut fx, merged_view(PR, &at, &commit(70)));
    let effects = fetched(&mut fx, &commit(71), Some(2));
    let lines = stage_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    assert_eq!(lines[0].human_review_secs, merged - opened);
    assert_eq!(lines[0].human_review_secs, secs(&fx));
}
