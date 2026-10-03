//! Milestone 9.3 task 10a (task 4b's carried item): while round 2 or later is open, the
//! action menu's `reject` and `cancel` effect lines name that round, as the requests
//! act on it (decisions 12 and 16); round 1's lines are unchanged
//! (`actions_effects.rs`, pinning). The final fix wave (I2): a live earlier-round task
//! stays editable while a later round runs, and the menu agrees.

use proto::{ActionKind, PlanEdit, RunState, TaskOrigin, TaskState};
use serde_json::json;

use super::actions_fixtures::ask;
use super::dispatch::{edit, replies};
use super::fixes::spec as fixes_spec;
use super::fixture::*;
use super::goal_rounds_end::{complete_staged, create_stages, submit_round};
use super::goal_rounds_pr::delivering;
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{iterate, reply, started};
use super::merge::window_of;
use super::orch::{answer, edit_plan};
use super::wake_notes::clear;
use crate::run::engine::EventKind;
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::fixes::add_fix;

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

/// The final fix wave (review A, I2): a `pr` run delivering stage 1's PR, in round 2
/// (`t2` in stage 2, working), with two engine-made fixes on stage 1, round 1's:
/// `fix1` blocked on a question and `fix2` working. Round 1's `t1` is merged.
pub(super) fn round_two_fixes() -> Fixture {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let now = fx.now;
    for scope in ["crates/fix/**", "crates/fix2/**"] {
        let spec = fixes_spec(TaskOrigin::Bisect, scope, Default::default());
        add_fix(fx.run_mut(), spec, now, &mut Vec::new()).expect("added");
    }
    fx.tick();
    let windows = fx.launch_all();
    ask(&mut fx, window_of(&windows, "fix1"), "fix1");
    assert_eq!((fx.task("fix1").round, fx.task("fix2").round), (1, 1));
    assert_eq!(fx.task("fix2").state, TaskState::Working);
    fx
}

/// I2: during round 2, a live round-1 fix can be answered, messaged, refreshed and
/// cancelled by the user, and its question answered by the orchestrator; the action
/// menu lists each, and a finished round-1 task stays read-only with KG's text.
#[test]
fn a_live_earlier_round_task_stays_editable() {
    use ActionKind::{Answer, CancelTask, Message, Refresh};
    let fx = round_two_fixes();
    let listed = |id: &str| -> Vec<ActionKind> {
        let node = ActionNode::Task(id);
        actions::available(fx.run(), &node)
            .into_iter()
            .filter(|a| a.refused_why.is_none())
            .map(|a| a.kind)
            .collect()
    };
    for kind in [Answer, Message, CancelTask] {
        assert!(
            listed("fix1").contains(&kind),
            "{kind:?}: {:?}",
            listed("fix1")
        );
    }
    assert!(listed("fix2").contains(&Refresh), "{:?}", listed("fix2"));
    // The user answers fix1's question; the orchestrator, a fresh fixture's.
    let mut user = round_two_fixes();
    let answer_edit = PlanEdit::Answer {
        task_id: "fix1".into(),
        text: "the users table".into(),
    };
    let got = replies(&edit(&mut user, vec![answer_edit]));
    assert!(got.len() == 1 && got[0].is_ok(), "{got:?}");
    let mut orch = round_two_fixes();
    let effects = edit_plan(
        &mut orch,
        json!({"edits": [{"op": "answer", "task_id": "fix1", "text": "the users table"}]}),
    );
    assert!(answer(&effects).0, "{effects:#?}");
    for fx in [&user, &orch] {
        assert_ne!(
            fx.task("fix1").state,
            TaskState::Blocked,
            "{:#?}",
            fx.run().log
        );
    }
    // The user cancels fix2.
    let mut cancelled = round_two_fixes();
    let cancel = PlanEdit::CancelTask {
        task_id: "fix2".into(),
    };
    assert!(replies(&edit(&mut cancelled, vec![cancel]))[0].is_ok());
    assert_eq!(cancelled.task("fix2").state, TaskState::Cancelled);
    // A finished round-1 task is still read-only, from every source.
    let mut done = round_two_fixes();
    let amend = PlanEdit::Refresh {
        task_id: "t1".into(),
    };
    let text = "stage 1 belongs to round 1, which is done; put new work in a new stage";
    assert_eq!(
        replies(&edit(&mut done, vec![amend])),
        vec![Err(text.to_string())]
    );
    // W1 fix round 2: a live earlier-round task takes answer, message, refresh and
    // cancel only; nothing that adds or moves work (D15, KG §2.4).
    let work = [
        PlanEdit::AmendTask {
            task_id: "fix1".into(),
            brief: Some("more".into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
            deps: None,
            stage: None,
        },
        PlanEdit::AddDep {
            task_id: "fix1".into(),
            dep: "t2".into(),
        },
    ];
    for refused in work {
        let mut fx = round_two_fixes();
        assert_eq!(
            replies(&edit(&mut fx, vec![refused.clone()])),
            vec![Err(text.to_string())],
            "{refused:?}"
        );
    }
}

/// W1 fix round 2 (the re-review's breakage 1): the orchestrator's `split_task` of a
/// live round-1 fix during round 2 is refused with KG §2.4's text, and no child lands
/// in round 1's stage.
#[test]
fn a_split_of_a_live_earlier_round_task_is_refused() {
    let mut fx = round_two_fixes();
    let child = super::orch::add("x1", "fix")["task"].clone();
    let split = json!({"op": "split_task", "task_id": "fix1", "into": [child]});
    let effects = edit_plan(&mut fx, json!({"edits": [split]}));
    let (ok, value) = answer(&effects);
    assert!(!ok, "{value}");
    assert_eq!(
        value["errors"][0]["message"],
        json!("stage 1 belongs to round 1, which is done; put new work in a new stage"),
        "{value}"
    );
    assert!(fx.run().task("x1").is_none());
    assert_eq!(fx.task("fix1").state, TaskState::Blocked);
}
