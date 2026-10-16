//! Milestone 9.16 task M9.16.8 (U12, decision 18): the hold stops new sessions only. A
//! held run's pull request is still polled on its schedule and its delivery goes on.
//! Pure: `now` moves by hand.

use proto::brakes::DispatchHold;

use super::delivery_watch::{next_poll, polls, watched};
use crate::run::engine::EventKind;
use crate::run::engine::brakes::HOLD_TEXT;

#[test]
fn a_held_run_still_polls_its_pull_request() {
    let mut fx = watched();
    let opened = fx.now;
    let state = fx.run().state;
    let reply = fx.reply();
    // At `opened`, so the hold's own step is not yet the poll's due time.
    fx.send(
        opened,
        EventKind::Hold {
            reply,
            hold: Some(DispatchHold {
                since: opened,
                by: "user".into(),
            }),
        },
    );
    assert!(fx.state.brakes.hold.is_some());
    assert_eq!(
        next_poll(&fx),
        opened + 1,
        "the next poll is still scheduled"
    );
    assert!(
        polls(&mut fx, opened + 1),
        "the due view goes out while held"
    );
    assert_eq!(fx.run().state, state, "the hold changes no run state");
    let attention = crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0]
        .attention
        .clone();
    assert!(attention.iter().any(|l| l == HOLD_TEXT), "{attention:?}");
}
