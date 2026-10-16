//! Milestone 9.17 task M9.17.5 (decisions 13 to 18, pre-flight rulings C5, C6): the plan
//! frame at submission. An orchestrator's acting 9.5-way submit and every `submit_epic`
//! carry `decided` and `out_of_scope`; a design-flow submit, a user's submit and an
//! edit without `submit` never need them; the frame is stored per round and per epic.

use proto::{DecidedItem, PlanEdit, RoundDesign, RunState};
use serde_json::{Value, json};

use super::design_plan_fixture::{covering, plan_submit, planning};
use super::design_rounds_fixture::{design_complete, iterate_with, off_round_submit, round_submit};
use super::dispatch::replies;
use super::fixture::*;
use super::goal_rounds_stages::add_in;
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::orch::{add, answer, edit_plan, error, launched};
use super::planners::{
    PLANNER, epic, planner_ended, planner_started, planner_tool, planning_mail, spawn, task_in,
};
use crate::run::engine::plan_frame::{EPIC_FRAME_MISSING, FRAME_NEEDS_SUBMIT, PLAN_FRAME_MISSING};
use crate::run::engine::planners::EPIC_RECORDED;
use crate::run::engine::{EventKind, ScoutEnd};
use crate::run::orch::contract::ACTION_ALONE;
use crate::run::orch::plan_frame::{PlanFrame, frame_for};
use crate::run::orch::test_support::plan_frame;
use crate::run::validate::EditScope;

/// `args` with `decided` and `out_of_scope` set.
fn framed(mut args: Value, decided: Value, out_of_scope: Value) -> Value {
    args["decided"] = decided;
    args["out_of_scope"] = out_of_scope;
    args
}

fn store_q() -> Value {
    json!([{"question": "Which store?", "answer": "SQLite"}])
}

/// The frame stored for `round`, stamped with the fixture's clock.
fn stored(fx: &Fixture, round: u32) -> Option<PlanFrame> {
    frame_for(fx.run(), round).cloned()
}

/// The user's `run edit --submit` of `edits`: its reply.
fn user_submit(fx: &mut Fixture, edits: Vec<PlanEdit>) -> Result<String, String> {
    let reply = fx.reply();
    let effects = fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits,
        scope: EditScope::Run,
        refusals: Vec::new(),
        submit: true,
    });
    replies(&effects).remove(0)
}

#[test]
fn submit_without_frame_is_refused_and_nothing_applies() {
    let mut fx = launched(false);
    let args = json!({"edits": [add("t1", "auth"), add("t2", "mail")], "submit": true});
    assert_eq!(error(&edit_plan(&mut fx, args)), PLAN_FRAME_MISSING);
    let run = fx.run();
    assert!(run.tasks.is_empty(), "{:?}", run.tasks);
    assert_eq!(run.state, RunState::Planning);
    assert!(run.orch.plan_frames.is_empty());
    // The refusal is logged as a rejected batch (decision 40).
    let logged = run.plan_edits.last().unwrap();
    assert!(!logged.accepted, "{logged:?}");
}

#[test]
fn submit_with_one_key_is_refused() {
    for (decided, out) in [(Some(json!([])), None), (None, Some(json!([])))] {
        let mut fx = launched(false);
        let mut args = json!({"edits": [add("t1", "auth")], "submit": true});
        if let Some(d) = decided {
            args["decided"] = d;
        }
        if let Some(o) = out {
            args["out_of_scope"] = o;
        }
        assert_eq!(
            error(&edit_plan(&mut fx, args.clone())),
            PLAN_FRAME_MISSING,
            "{args}"
        );
        assert!(fx.run().tasks.is_empty());
        assert_eq!(fx.run().state, RunState::Planning);
    }
}

#[test]
fn submit_with_empty_lists_is_accepted_and_stored() {
    let mut fx = launched(false);
    let args = framed(
        json!({"edits": [add("t1", "auth")], "submit": true}),
        json!([]),
        json!([]),
    );
    let (ok, value) = answer(&edit_plan(&mut fx, args));
    assert!(ok, "{value}");
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let want = PlanFrame {
        at: fx.now,
        ..plan_frame(1, &[], &[])
    };
    assert_eq!(stored(&fx, 1), Some(want));
}

/// Decision 18: one frame per round; a submit with the keys replaces it, one without
/// keeps it.
#[test]
fn resubmit_in_the_same_round_keeps_or_replaces_the_frame() {
    let mut fx = launched(false);
    let first = framed(
        json!({"edits": [add("t1", "auth")], "submit": true}),
        store_q(),
        json!(["no UI"]),
    );
    assert!(answer(&edit_plan(&mut fx, first)).0);
    let want = PlanFrame {
        at: fx.now,
        ..plan_frame(1, &[("Which store?", "SQLite")], &["no UI"])
    };
    assert_eq!(stored(&fx, 1), Some(want.clone()));
    // Without the keys (at the gate, a submit that changes nothing): kept.
    fx.now += 5;
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"submit": true})));
    assert!(ok, "{value}");
    assert_eq!(stored(&fx, 1), Some(want.clone()));
    // With the same lists: kept too, its time included.
    let same = framed(json!({"submit": true}), store_q(), json!(["no UI"]));
    assert!(answer(&edit_plan(&mut fx, same)).0);
    assert_eq!(stored(&fx, 1), Some(want));
    // With other lists: replaced, and still one frame for the round.
    let again = framed(json!({"submit": true}), json!([]), json!(["no docs"]));
    assert!(answer(&edit_plan(&mut fx, again)).0);
    let want = PlanFrame {
        at: fx.now,
        ..plan_frame(1, &[], &["no docs"])
    };
    assert_eq!(fx.run().orch.plan_frames, vec![want]);
}

#[test]
fn a_new_round_needs_a_new_frame() {
    let mut fx = complete();
    assert!(stored(&fx, 1).is_some(), "round 1's submit carried a frame");
    assert_eq!(reply(&iterate(&mut fx, "Add the app")), started(2));
    assert_eq!(fx.run().state, RunState::Planning);
    let args = json!({"edits": [add_in("t2", "mail", 2, &[])], "submit": true});
    assert_eq!(error(&edit_plan(&mut fx, args.clone())), PLAN_FRAME_MISSING);
    assert!(fx.run().task("t2").is_none());
    let args = framed(args, json!([]), json!(["no app store"]));
    let (ok, value) = answer(&edit_plan(&mut fx, args));
    assert!(ok, "{value}");
    assert_eq!(stored(&fx, 2).unwrap().out_of_scope, ["no app store"]);
    assert_eq!(stored(&fx, 1).unwrap().out_of_scope, Vec::<String>::new());
}

#[test]
fn keys_without_submit_are_refused() {
    for args in [
        framed(json!({"edits": [add("t1", "auth")]}), json!([]), json!([])),
        json!({"edits": [add("t1", "auth")], "decided": []}),
        json!({"out_of_scope": ["no UI"]}),
        json!({"edits": [add("t1", "auth")], "submit": false, "decided": [], "out_of_scope": []}),
    ] {
        let mut fx = launched(false);
        assert_eq!(
            error(&edit_plan(&mut fx, args.clone())),
            FRAME_NEEDS_SUBMIT,
            "{args}"
        );
        assert!(fx.run().tasks.is_empty(), "{args}");
        assert!(fx.run().orch.plan_frames.is_empty());
    }
}

/// Decision 16: a design run's planning submit needs no frame; a round with the flow
/// off is 9.5's and does.
#[test]
fn design_flow_submit_is_exempt() {
    let mut fx = planning();
    let answer = plan_submit(&mut fx, json!([covering("t1", &["R1", "R2"])]), Value::Null);
    let answer = answer.unwrap();
    assert_eq!(answer["awaiting_review"], true, "{answer}");
    assert!(fx.run().orch.plan_frames.is_empty());

    let mut fx = design_complete();
    iterate_with(&mut fx, Some(RoundDesign::Off)).unwrap();
    let mut t2 = add("t2", "mail");
    t2["task"]["stage"] = json!(2);
    let refused = round_submit(&mut fx, json!([t2.clone()]));
    assert_eq!(refused.unwrap_err(), PLAN_FRAME_MISSING);
    assert!(fx.run().task("t2").is_none());
    let answer = off_round_submit(&mut fx, json!([t2])).unwrap();
    assert_eq!(answer["awaiting_approval"], true, "{answer}");
    assert!(stored(&fx, 2).is_some());
}

/// Decision 17: `run edit --submit` never needs a frame, on a plan-file run or an
/// orchestrator run.
#[test]
fn user_submit_never_needs_a_frame() {
    let mut fx = super::actions_fixtures::gate();
    fx.run_mut().state = RunState::Planning;
    assert_eq!(
        user_submit(&mut fx, Vec::new()),
        Ok(format!(
            "the plan of run {RUN_ID} was submitted: it awaits approval"
        ))
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);

    let mut fx = launched(false);
    let t1: PlanEdit = serde_json::from_value(add("t1", "auth")).unwrap();
    let got = user_submit(&mut fx, vec![t1]);
    assert!(
        got.as_ref()
            .is_ok_and(|t| t.ends_with("it awaits approval")),
        "{got:?}"
    );
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert!(fx.run().orch.plan_frames.is_empty());
}

/// Decision 14 with ruling C5: a counted rejection, before any other check.
#[test]
fn submit_epic_without_frame_is_refused_and_recorded() {
    let mut fx = planning_mail(false);
    let args = json!({"edits": [task_in("t2", "mail")], "out_of_scope": []});
    let effects = planner_tool(&mut fx, (PLANNER, "mail"), "submit_epic", args);
    let (ok, value) = answer(&effects);
    assert!(!ok, "{value}");
    assert_eq!(
        value,
        json!({"accepted": false, "errors": [
            {"task": null, "field": "decided", "rule": "frame", "message": EPIC_FRAME_MISSING}
        ]})
    );
    let e = epic(&fx, "mail");
    assert_eq!(e.edits_rejected, 1);
    assert_eq!(e.last_rejection.as_deref(), Some(EPIC_FRAME_MISSING));
    assert_eq!(e.sessions.last().unwrap().rejections, 1);
    assert_eq!(e.frame, None);
    assert!(fx.run().task("t2").is_none());
    let logged = fx.run().plan_edits.last().unwrap();
    assert!(!logged.accepted, "{logged:?}");
    assert_eq!(logged.source, "planner:mail");
}

#[test]
fn submit_epic_with_frame_stores_it_on_the_epic() {
    let mut fx = planning_mail(false);
    let args = framed(
        json!({"edits": [task_in("t2", "mail")]}),
        store_q(),
        json!([]),
    );
    let effects = planner_tool(&mut fx, (PLANNER, "mail"), "submit_epic", args);
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    let want = PlanFrame {
        at: fx.now,
        ..plan_frame(1, &[("Which store?", "SQLite")], &[])
    };
    assert_eq!(epic(&fx, "mail").frame, Some(want));
    // A re-plan's accepted submit replaces it.
    planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER + 1);
    fx.now += 7;
    let args = framed(
        json!({"edits": [task_in("t3", "mail")]}),
        json!([]),
        json!(["no retries"]),
    );
    let effects = planner_tool(&mut fx, (PLANNER + 1, "mail"), "submit_epic", args);
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    let want = PlanFrame {
        at: fx.now,
        ..plan_frame(1, &[], &["no retries"])
    };
    assert_eq!(epic(&fx, "mail").frame, Some(want));
}

/// Decision 18: every text is stored on one line, its control characters dropped.
#[test]
fn frame_texts_are_one_lined_on_storage() {
    let mut fx = launched(false);
    let args = framed(
        json!({"edits": [add("t1", "auth")], "submit": true}),
        json!([{"question": "Which\nstore?\u{1b}[31m", "answer": "SQLite\r\n"}]),
        json!(["no\tUI\u{7}"]),
    );
    assert!(answer(&edit_plan(&mut fx, args)).0);
    let frame = stored(&fx, 1).unwrap();
    assert_eq!(
        frame.decided,
        [DecidedItem {
            question: "Which store?[31m".into(),
            answer: "SQLite".into(),
        }]
    );
    assert_eq!(frame.out_of_scope, ["no UI"]);
}

/// Milestone 9.9 decision 11 (ruling C6): the keys are "others", so an op with them is
/// not alone.
#[test]
fn nine_nine_intercept_treats_the_keys_as_others() {
    for extra in [json!({"decided": []}), json!({"out_of_scope": []})] {
        let mut fx = launched(false);
        let mut args = json!({"edits": [{"op": "retry", "task_id": "t1", "reason": "again"}]});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            error(&edit_plan(&mut fx, args.clone())),
            ACTION_ALONE,
            "{args}"
        );
    }
}

/// Ruling C6: `iterate` with a key is not alone; a complete run's summary with a key
/// is refused with the complete run's text.
#[test]
fn iterate_and_a_complete_runs_summary_refuse_the_keys() {
    let alone = "iterate must be the only thing in its edit_plan call";
    for extra in [json!({"decided": []}), json!({"out_of_scope": ["x"]})] {
        let mut fx = complete();
        let mut args = json!({"iterate": "more"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(error(&edit_plan(&mut fx, args.clone())), alone, "{args}");
        assert_eq!(fx.run().rounds.len(), 1);
        let mut args = json!({"summary": "done"});
        args.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert_eq!(
            error(&edit_plan(&mut fx, args.clone())),
            "edits are not accepted on a complete run; only a summary is",
            "{args}"
        );
    }
}

/// Task 5 review M1 (ruling C6): a promoted run's submit acts while its promotion is
/// open, so it needs the frame; once the promotion is approved a running submit acts
/// on nothing and needs none.
#[test]
fn a_promoted_runs_submit_needs_the_frame() {
    use super::promote::{hold_verdict, promoted};
    let mut fx = promoted();
    let bare = json!({"edits": [add("t2", "mail")], "submit": true});
    assert_eq!(error(&edit_plan(&mut fx, bare.clone())), PLAN_FRAME_MISSING);
    assert!(
        fx.run().tasks.iter().all(|t| t.id() != "t2"),
        "nothing applied"
    );
    assert!(fx.run().orch.plan_frames.is_empty());
    let (ok, value) = answer(&edit_plan(&mut fx, framed(bare, json!([]), json!([]))));
    assert!(ok, "{value}");
    hold_verdict(&mut fx, "promotion", true);
    // The stored frame is not what lets the next one through.
    fx.run_mut().orch.plan_frames.clear();
    let later = json!({"edits": [add("t3", "sms")], "submit": true});
    let (ok, value) = answer(&edit_plan(&mut fx, later));
    assert!(ok, "{value}");
}

/// Task 5 review M2: an item that is empty once cleaned (spaces or control characters
/// only) is refused before anything applies (one parser serves `edit_plan` and
/// `submit_epic`).
#[test]
fn a_blank_frame_item_is_refused() {
    let blanks = [
        (json!([{"question": " \u{7} ", "answer": "a"}]), json!([])),
        (json!([{"question": "q", "answer": "\u{1b}\t"}]), json!([])),
        (
            json!([{"question": "q", "answer": "\u{200B}\u{202E}"}]),
            json!([]),
        ),
        (json!([]), json!(["   "])),
    ];
    for (decided, out) in blanks {
        let mut fx = launched(false);
        let args = framed(
            json!({"edits": [add("t1", "auth")], "submit": true}),
            decided.clone(),
            out.clone(),
        );
        let text = error(&edit_plan(&mut fx, args));
        assert!(text.contains("must have text"), "{text}");
        assert!(fx.run().tasks.is_empty());
        assert!(fx.run().orch.plan_frames.is_empty());
    }
}

/// Task 5 review M3 (security): format characters (bidi overrides, zero-width
/// characters) are dropped on storage, as control characters are.
#[test]
fn frame_texts_drop_format_characters_on_storage() {
    let mut fx = launched(false);
    let args = framed(
        json!({"edits": [add("t1", "auth")], "submit": true}),
        json!([{"question": "Which\u{202E} store?", "answer": "SQ\u{200B}Lite\u{2066}"}]),
        json!(["no\u{FEFF} UI\u{600}"]),
    );
    assert!(answer(&edit_plan(&mut fx, args)).0);
    let frame = stored(&fx, 1).unwrap();
    assert_eq!(
        frame.decided,
        [DecidedItem {
            question: "Which store?".into(),
            answer: "SQLite".into(),
        }]
    );
    assert_eq!(frame.out_of_scope, ["no UI"]);
}
