//! Milestone 9.3 task 10a (task 4b's carried item): while round 2 or later is open, the
//! action menu's `reject` and `cancel` effect lines name that round, as the requests
//! act on it (decisions 12 and 16); round 1's lines are unchanged
//! (`actions_effects.rs`, pinning).

use proto::{ActionKind, RunState};
use serde_json::json;

use super::fixture::*;
use super::goal_rounds_end::{complete_staged, create_stages, submit_round};
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{iterate, reply, started};
use super::wake_notes::clear;
use crate::run::engine::EventKind;
use crate::run::engine::actions::{self, ActionNode};

fn effect(fx: &Fixture, kind: ActionKind) -> String {
    actions::available(fx.run(), &ActionNode::Run)
        .into_iter()
        .find(|a| a.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?} is not listed"))
        .effect
}

#[test]
fn reject_and_cancel_previews_name_an_open_later_round() {
    // Round 2 at its gate: its one task, the earlier round's untouched.
    let mut fx = complete_staged();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    clear(&mut fx);
    submit_round(&mut fx);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_eq!(
        effect(&fx, ActionKind::Reject),
        "reject: drop round 2's 1 task; the earlier rounds are unchanged"
    );
    // The preview says what the request does.
    let reply_id = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    });
    assert_eq!(
        reply(&effects),
        Ok("run 3f9a round 2 rejected; the earlier rounds are unchanged".into())
    );

    // Round 2 running, its worker live.
    let mut fx = complete_staged();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let windows = fx.launch_all();
    assert!(windows.iter().any(|(t, _)| t == "t2"), "{windows:?}");
    assert_eq!(
        effect(&fx, ActionKind::Cancel),
        "cancel: stop 1 worker and cancel round 2's 1 unmerged task; the earlier rounds are unchanged"
    );
}
