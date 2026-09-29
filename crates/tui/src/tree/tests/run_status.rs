//! M9.15 review: a run's roll-up (decision 9) with milestone 9's states. A hold
//! awaiting approval asks for the user; a fresh `paused(message)` task does not, until
//! the daemon lists it in the run's attention after 600 s (decision 42c).

use crate::tree::orch_fixtures::{held_fixture, hold};
use crate::tree::run_status;
use proto::{HoldState, Status};

#[test]
fn a_hold_awaiting_approval_rolls_up_attention() {
    let (mut snapshot, _) = held_fixture();
    let run = &mut snapshot.runs[0];
    // No paused task, so the hold alone decides.
    run.tasks[1].state = proto::TaskState::Working;
    run.tasks[1].block = None;
    assert_eq!(run_status(run), Status::Attention);
    run.holds = vec![hold("epic:ui", HoldState::Approved, &["t2"])];
    assert_eq!(run_status(run), Status::Working);
}

#[test]
fn a_fresh_pause_alone_does_not_roll_up_attention_but_a_listed_one_does() {
    let (mut snapshot, _) = held_fixture();
    let run = &mut snapshot.runs[0];
    run.holds.clear();
    run.tasks[2].hold = None;
    assert!(run.attention.is_empty());
    assert_eq!(run_status(run), Status::Working);
    run.attention = vec!["t1 paused(message) for 10 min".into()];
    assert_eq!(run_status(run), Status::Attention);
}
