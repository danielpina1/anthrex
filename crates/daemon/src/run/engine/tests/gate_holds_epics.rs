//! M9.7 second review, item 10: an epic's rounds come first for every task naming it,
//! split children included; a re-plan opens a new round; a `Drafting` round with no
//! live task and no live planner is dropped.

use proto::{HoldState, PlanEdit, TaskState};
use serde_json::json;

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::gate_holds::{awaiting, held, hold_state};
use super::orch::{ORCH, add, answer, edit_plan, orch_tool};
use super::promote::{add_to_epic, hold_verdict, prepared, promoted};
use crate::run::engine::gate_holds::submitted;

fn spawn(fx: &mut Fixture, epic: &str, brief: &str) -> serde_json::Value {
    let args = json!({"epic": epic, "title": format!("Epic {epic}"),
        "area": [format!("crates/{epic}/**")], "brief": brief});
    let (ok, value) = answer(&orch_tool(fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    value
}

fn submit(fx: &mut Fixture, hold: &str) {
    let now = fx.now;
    submitted(fx.run_mut(), hold, now);
}

/// Item 10.1, the reviewer's sequence: `epic:web` rejected, then a split of the
/// promotion-held `t2` gives `t2a` epic `web`. `t2a` follows web's rounds, not the
/// promotion it was split from, so approving the promotion does not release it.
#[test]
fn a_split_child_naming_a_rejected_epic_opens_its_next_round() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    assert_eq!(spawn(&mut fx, "web", "Plan web")["hold"], "epic:web");
    assert_eq!(add_to_epic(&mut fx, "t5", "web")["held"], "epic:web");
    submit(&mut fx, "epic:web");
    hold_verdict(&mut fx, "epic:web", false);
    let mut t2a = add("t2a", "web")["task"].clone();
    t2a["epic"] = json!("web");
    let t2b = add("t2b", "mail_b")["task"].clone();
    let split = json!({"op": "split_task", "task_id": "t2", "into": [t2a, t2b]});
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [split]})));
    assert!(ok, "{value}");
    assert_eq!(fx.task("t2a").orch.gate_hold.as_deref(), Some("epic:web.2"));
    assert_eq!(fx.task("t2b").orch.gate_hold.as_deref(), Some("promotion"));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    hold_verdict(&mut fx, "promotion", true);
    fx.tick();
    assert!(prepared(&fx, "t2b"), "the promotion's own work runs");
    assert!(!prepared(&fx, "t2a"), "web's work waits for its round");
}

/// Item 10.2: a `Drafting` round whose tasks the user all cancelled, with its planner
/// ended, is dropped, so nothing is left for completion to wait on (decision 38).
#[test]
fn a_drafting_round_whose_tasks_were_all_cancelled_is_dropped() {
    let mut fx = held(false);
    assert_eq!(hold_state(&fx, "epic:mail"), HoldState::Drafting);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t2".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    let open = fx
        .run()
        .orch
        .gate_holds
        .iter()
        .filter(|h| matches!(h.state, HoldState::Drafting | HoldState::Awaiting))
        .count();
    assert_eq!(open, 0, "{:?}", fx.run().orch.gate_holds);
}

/// Item 10.3, the controller's decision: a re-plan of an approved epic is new work, so
/// it opens a new round, and the planner's tasks wait for the user again.
#[test]
fn a_replan_of_an_approved_epic_opens_a_new_round() {
    let mut fx = held(false);
    awaiting(&mut fx);
    hold_verdict(&mut fx, "epic:mail", true);
    assert_eq!(
        spawn(&mut fx, "mail", "Plan mail again")["hold"],
        "epic:mail.2"
    );
    assert_eq!(add_to_epic(&mut fx, "t3", "mail")["held"], "epic:mail.2");
    fx.tick();
    assert!(!prepared(&fx, "t3"), "the re-plan's task waits");
    submit(&mut fx, "epic:mail.2");
    hold_verdict(&mut fx, "epic:mail.2", true);
    assert!(
        prepared(&fx, "t3"),
        "dispatched on the new round's approval"
    );
}
