use serde_json::{Value, json};

use crate::delivery::tests::variant_names;
use crate::*;

fn both<T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(v: &T) {
    let back: T = serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    assert_eq!(&back, v);
    let back: T = rmp_serde::from_slice(&rmp_serde::to_vec_named(v).unwrap()).unwrap();
    assert_eq!(&back, v);
}

#[test]
fn the_five_ops_have_their_wire_shapes() {
    let cases = [
        (
            json!({"op": "retry", "task_id": "t3", "reason": "the lock was transient"}),
            PlanEdit::Retry {
                task_id: "t3".into(),
                reason: "the lock was transient".into(),
            },
        ),
        (
            json!({"op": "override", "task_id": "t3", "reason": "the reviewer looped"}),
            PlanEdit::Override {
                task_id: "t3".into(),
                reason: "the reviewer looped".into(),
            },
        ),
        (
            json!({"op": "resume_run", "reason": "the ref read again"}),
            PlanEdit::ResumeRun {
                reason: "the ref read again".into(),
                stage: None,
            },
        ),
        (
            json!({"op": "resume_run", "reason": "push again", "stage": 2}),
            PlanEdit::ResumeRun {
                reason: "push again".into(),
                stage: Some(2),
            },
        ),
        (
            json!({"op": "approve_hold", "hold": "epic:ui", "reason": "planned"}),
            PlanEdit::ApproveHold {
                hold: "epic:ui".into(),
                reason: "planned".into(),
            },
        ),
        (
            json!({"op": "accept_red", "stage": 2, "reason": "red on main too"}),
            PlanEdit::AcceptRed {
                stage: 2,
                reason: "red on main too".into(),
            },
        ),
    ];
    for (wire, edit) in cases {
        assert_eq!(
            serde_json::from_value::<PlanEdit>(wire.clone()).unwrap(),
            edit
        );
        both(&edit);
    }
    // The pause edit keeps its name and shape.
    assert_eq!(
        serde_json::from_value::<PlanEdit>(json!({"op": "resume"})).unwrap(),
        PlanEdit::Resume
    );
}

#[test]
fn an_op_without_its_reason_or_with_an_unknown_key_is_refused() {
    for wire in [
        json!({"op": "retry", "task_id": "t3"}),
        json!({"op": "accept_red", "stage": 2}),
        json!({"op": "retry", "task_id": "t3", "reason": "r", "force": true}),
    ] {
        assert!(
            serde_json::from_value::<PlanEdit>(wire.clone()).is_err(),
            "{wire}"
        );
    }
}

#[test]
fn the_five_ops_are_appended_last() {
    let error = serde_json::from_value::<PlanEdit>(json!({"op": "no_such"}))
        .unwrap_err()
        .to_string();
    let names = error.split_once("expected one of ").unwrap().1;
    assert!(
        names.ends_with(
            "`iterate`, `retry`, `override`, `resume_run`, `approve_hold`, `accept_red`"
        ),
        "{names}"
    );
    assert_eq!(
        variant_names::<RunRequest>().last().map(String::as_str),
        Some("AnswerAsk")
    );
}

fn route() -> Route {
    serde_json::from_value(json!({"runtime": "claude", "model": "", "effort": "high"})).unwrap()
}

fn info() -> OrchestratorInfo {
    OrchestratorInfo {
        route: route(),
        window_id: Some(9),
        live: true,
        started_at: 1,
        plan_submitted: true,
        summary: None,
        notes: vec![],
        wakes: 2,
        wake_held: false,
        stuck: None,
        ask: None,
        handled: vec![],
        handled_total: 0,
    }
}

#[test]
fn orchestrator_info_new_fields_round_trip_and_default() {
    let mut full = info();
    full.stuck = Some(OrchestratorStuck::Stalled { since: 40 });
    full.ask = Some(AskInfo {
        id: 3,
        question: "tabs or spaces?".into(),
        options: vec!["tabs".into(), "spaces".into()],
        context: "the style guide is silent".into(),
        asked_at: 41,
    });
    full.handled = vec![HandledInfo {
        at: 42,
        op: "retry".into(),
        target: "t3".into(),
        reason: "transient".into(),
    }];
    full.handled_total = 7;
    both(&full);
    both(&OrchestratorStuck::Dead { since: None });
    // A protocol-19 value has none of the four keys and decodes to their defaults.
    let mut old = serde_json::to_value(info()).unwrap();
    for key in ["stuck", "ask", "handled", "handled_total"] {
        assert!(old.get(key).is_none(), "{key} is left out while empty");
        old.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(
        serde_json::from_value::<OrchestratorInfo>(old).unwrap(),
        info()
    );
    assert_eq!(
        serde_json::to_value(OrchestratorStuck::Stalled { since: 4 }).unwrap(),
        json!({"kind": "stalled", "since": 4})
    );
}

#[test]
fn user_only_and_accepted_default_false_and_are_left_out() {
    let old: BlockInfo =
        serde_json::from_value(json!({"reason": "environment", "text": "x"})).unwrap();
    assert_eq!(old, BlockInfo::new(BlockReason::Environment, "x"));
    assert!(!old.user_only);
    assert!(
        serde_json::to_value(&old)
            .unwrap()
            .get("user_only")
            .is_none()
    );
    let tagged = BlockInfo {
        user_only: true,
        ..old
    };
    both(&tagged);
    let alert: DeliveryAlert =
        serde_json::from_value(json!({"kind": "host_op_held", "stage": 1, "text": "t"})).unwrap();
    assert!(!alert.user_only);
    both(&DeliveryAlert {
        user_only: true,
        ..alert
    });
    let full: FullInfo =
        serde_json::from_value(serde_json::to_value(FullInfo::default()).unwrap()).unwrap();
    assert!(!full.accepted);
    both(&FullInfo {
        accepted: true,
        ..FullInfo::default()
    });
}

#[test]
fn answer_ask_round_trips_and_its_choice_defaults_to_none() {
    both(&RunRequest::AnswerAsk {
        run_id: "r".into(),
        ask: 3,
        choice: Some(1),
    });
    both(&RunRequest::AnswerAsk {
        run_id: "r".into(),
        ask: 3,
        choice: None,
    });
    let wire: Value = json!({"AnswerAsk": {"run_id": "r", "ask": 3}});
    assert_eq!(
        serde_json::from_value::<RunRequest>(wire).unwrap(),
        RunRequest::AnswerAsk {
            run_id: "r".into(),
            ask: 3,
            choice: None
        }
    );
    assert_eq!(crate::run_wire::request::ANSWER_ASK, "run answer-ask");
}
