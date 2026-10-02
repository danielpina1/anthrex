//! Milestone 9 task 2: the orchestrator, sub-planner, hold, integration, message and
//! role-history types, the new requests, and the appended enum variants.

use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::EditFile;
use crate::messages::ClientMsg;
use crate::orch::*;
use crate::run::{AgentRole, BlockReason, Effort, PlanEdit, Route, RunState, Strength, TaskState};
use crate::run_wire::RunRequest;
use crate::types::Runtime;

/// The role-routing history line (decision 43), split out to keep this file under the
/// 600-line rule.
#[path = "orch_tests_history.rs"]
mod history;

/// Request ids on run replies (decision 2), split out for the same reason.
#[path = "orch_tests_replies.rs"]
mod replies;

/// Milestone 9.0.5: the task detail request and reply.
#[path = "orch_tests_detail.rs"]
mod detail;

fn a_route(runtime: Runtime, model: &str) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    }
}

/// Through MessagePack (named, as the codec sends) and JSON, back to an equal value.
fn both_ways<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, value, "JSON: {json}");
}

#[test]
fn orch_types_round_trip() {
    for choice in [
        OrchestratorChoice {
            runtime: Runtime::Claude,
            model: Some("claude-opus-5".into()),
        },
        OrchestratorChoice {
            runtime: Runtime::Codex,
            model: None,
        },
    ] {
        both_ways(&choice);
    }
    // `model` is `#[serde(default)]`: a choice without it names the configured default.
    let bare: OrchestratorChoice = serde_json::from_str(r#"{"runtime":"codex"}"#).unwrap();
    assert_eq!(bare.model, None);

    let kinds = [
        HoldKind::Promotion,
        HoldKind::Epic {
            epic: "auth".into(),
        },
    ];
    for kind in &kinds {
        both_ways(kind);
    }
    assert_eq!(
        serde_json::to_string(&kinds[1]).unwrap(),
        r#"{"kind":"epic","epic":"auth"}"#
    );
    assert_eq!(
        serde_json::to_string(&kinds[0]).unwrap(),
        r#"{"kind":"promotion"}"#
    );

    let states = [
        (HoldState::Drafting, "drafting"),
        (HoldState::Awaiting, "awaiting"),
        (HoldState::Approved, "approved"),
        (HoldState::Rejected, "rejected"),
        (HoldState::Moot, "moot"),
    ];
    for (state, text) in states {
        both_ways(&state);
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{text}\"")
        );
        let hold = HoldInfo {
            id: "h2".into(),
            kind: HoldKind::Epic { epic: "ui".into() },
            state,
            tasks: vec!["ui1".into(), "ui2".into()],
            created_at: 1_700_000_100,
            decided_at: Some(1_700_000_200),
            decided_by: Some("user".into()),
        };
        both_ways(&hold);
    }

    let orchestrator = OrchestratorInfo {
        route: a_route(Runtime::Claude, "claude-opus-5"),
        window_id: Some(7),
        live: true,
        started_at: 1_700_000_300,
        plan_submitted: true,
        summary: Some("two epics, eleven tasks".into()),
        notes: vec!["t3 blocked: which table?".into()],
        wakes: 4,
        wake_held: false,
    };
    both_ways(&orchestrator);

    let integration_states = [
        (IntegrationState::NotYet, "not_yet"),
        (IntegrationState::Reviewing, "reviewing"),
        (IntegrationState::Approved, "approved"),
        (IntegrationState::Changes, "changes"),
        (IntegrationState::Finished, "finished"),
    ];
    assert_eq!(IntegrationState::default(), IntegrationState::NotYet);
    for (state, text) in integration_states {
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            format!("\"{text}\"")
        );
        both_ways(&IntegrationInfo {
            epic: "auth".into(),
            state,
            base: Some("abc1234".into()),
            merges: vec!["def5678".into()],
            tasks: vec!["auth-int1".into()],
        });
    }

    for (kind, text) in [
        (TaskNoteKind::Discovery, "discovery"),
        (TaskNoteKind::Risk, "risk"),
        (TaskNoteKind::Progress, "progress"),
    ] {
        assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{text}\""));
        both_ways(&TaskNoteInfo {
            task_id: "t2".into(),
            kind,
            text: "the parser is shared with t4".into(),
            at: 1_700_000_400,
        });
    }
}

fn message(to: MessageTarget, kind: MessageKind) -> PlanEdit {
    PlanEdit::Message {
        to,
        text: "the schema moved to crates/db".into(),
        kind,
    }
}

#[test]
fn message_and_refresh_edits_round_trip() {
    let targets = [
        (
            MessageTarget::Tasks(vec!["t1".into(), "t2".into()]),
            r#"["t1","t2"]"#,
        ),
        (MessageTarget::Stage(3), r#""stage:3""#),
        (MessageTarget::Running, r#""running""#),
    ];
    let shown: Vec<String> = targets.iter().map(|(t, _)| t.to_string()).collect();
    assert_eq!(shown, ["t1,t2", "stage:3", "running"]);
    for (target, json) in &targets {
        assert_eq!(&serde_json::to_string(target).unwrap(), json);
        assert_eq!(
            &serde_json::from_str::<MessageTarget>(json).unwrap(),
            target
        );
        both_ways(target);
    }
    let kinds = [
        (MessageKind::Info, "info"),
        (MessageKind::Change, "change"),
        (MessageKind::StopAndWait, "stop_and_wait"),
    ];
    let mut edits = Vec::new();
    for ((target, _), (kind, text)) in targets.iter().zip(kinds) {
        assert_eq!(serde_json::to_string(&kind).unwrap(), format!("\"{text}\""));
        edits.push(message(target.clone(), kind));
    }
    edits.push(PlanEdit::Refresh {
        task_id: "t4".into(),
    });
    for edit in &edits {
        both_ways(edit);
        // Wrapped as the wire carries it.
        both_ways(&ClientMsg::Run(RunRequest::Edit {
            run_id: "run-a1b2".into(),
            edits: vec![edit.clone()],
            submit: false,
        }));
    }
    // The shape a `run edit --file` or an `edit_plan` call writes.
    let json = r#"{"op":"message","to":"running","text":"rebuild","kind":"stop_and_wait"}"#;
    assert_eq!(
        serde_json::from_str::<PlanEdit>(json).unwrap(),
        PlanEdit::Message {
            to: MessageTarget::Running,
            text: "rebuild".into(),
            kind: MessageKind::StopAndWait,
        }
    );
    let file = EditFile { edits };
    let toml_text = toml::to_string(&file).unwrap();
    assert_eq!(toml::from_str::<EditFile>(&toml_text).unwrap(), file);

    both_ways(&BlockReason::MessagePause);
    assert_eq!(
        serde_json::to_string(&BlockReason::MessagePause).unwrap(),
        "\"message_pause\""
    );

    for bad in [
        r#""stage:x""#,
        r#""stage:""#,
        r#""runnin""#,
        "[]",
        r#""t1""#,
    ] {
        assert!(
            serde_json::from_str::<MessageTarget>(bad).is_err(),
            "{bad} must not parse"
        );
    }
    let bad_edit = r#"{"op":"message","to":[],"text":"x","kind":"info"}"#;
    assert!(serde_json::from_str::<PlanEdit>(bad_edit).is_err());
}

/// Decodes the MessagePack positive fixint `index` as a variant of `T`: the variant
/// index serde's derive gives it, which is its place in the declaration.
fn variant_at<T: DeserializeOwned>(index: u8) -> Option<T> {
    assert!(index < 0x80, "a positive fixint");
    rmp_serde::from_slice(&[index]).ok()
}

fn assert_last<T>(older: &[T], appended: &[T])
where
    T: DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let all: Vec<&T> = older.iter().chain(appended).collect();
    for (index, expected) in all.iter().enumerate() {
        assert_eq!(
            variant_at::<T>(index as u8).as_ref(),
            Some(*expected),
            "variant index {index}"
        );
    }
    assert!(
        variant_at::<T>(all.len() as u8).is_none(),
        "no variant after"
    );
}

#[test]
fn appended_variants_keep_their_indices() {
    use AgentRole as A;
    assert_last(
        &[A::Orchestrator, A::Worker, A::Reviewer, A::Scout],
        &[A::Planner, A::Decider],
    );
    use RunState as R;
    assert_last(
        &[
            R::AwaitingApproval,
            R::Running,
            R::Paused,
            R::Halted,
            R::Complete,
            R::Accepted,
            R::Discarded,
            R::Failed,
        ],
        &[R::Planning],
    );
    use TaskState as T;
    assert_last(
        &[
            T::Pending,
            T::Queued,
            T::Preparing,
            T::Working,
            T::Proof,
            T::Check,
            T::Review,
            T::MergeQueue,
            T::Merged,
            T::Blocked,
            T::Cancelled,
        ],
        &[T::Reported],
    );
    use BlockReason as B;
    assert_last(
        &[
            B::MisSized,
            B::Human,
            B::Conflict,
            B::DepCancelled,
            B::Question,
            B::Environment,
        ],
        &[B::MessagePause],
    );
    // `PlanEdit` is tagged by name ("op"), so its wire carries no index; its variant
    // order is pinned by the names serde reports, M8a's nine first.
    let names = plan_edit_names();
    assert_eq!(
        names,
        [
            "add_task",
            "split_task",
            "cancel_task",
            "amend_task",
            "add_dep",
            "answer",
            "pause",
            "resume",
            "finish",
            "message",
            "refresh",
            "reply_comment",
        ]
    );
}

/// The `op` names in declaration order, read from the error serde gives for an unknown
/// one ("expected one of `add_task`, …").
fn plan_edit_names() -> Vec<String> {
    let error = serde_json::from_str::<PlanEdit>(r#"{"op":"no_such_op"}"#)
        .unwrap_err()
        .to_string();
    let list = error
        .split_once("expected one of ")
        .map(|(_, rest)| rest)
        .unwrap_or_else(|| panic!("unexpected error: {error}"));
    list.split(", ")
        .map(|name| {
            let name = name.trim_start_matches('`');
            name.split('`').next().unwrap_or(name).to_string()
        })
        .collect()
}

#[test]
fn reported_is_finished_and_planning_is_not_terminal() {
    assert!(TaskState::Reported.is_finished());
    assert_eq!(TaskState::Reported.label(), "reported");
    assert_eq!(
        serde_json::to_string(&TaskState::Reported).unwrap(),
        "\"reported\""
    );
    assert!(!RunState::Planning.is_terminal());
    assert_eq!(RunState::Planning.label(), "planning");
    assert_eq!(
        serde_json::to_string(&RunState::Planning).unwrap(),
        "\"planning\""
    );
    assert_eq!(
        serde_json::to_string(&AgentRole::Planner).unwrap(),
        "\"planner\""
    );
}

#[test]
fn new_requests_round_trip() {
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: Some("gpt-5-codex".into()),
    };
    let requests = [
        RunRequest::ApproveHold {
            run_id: "run-a1b2".into(),
            hold: "h1".into(),
        },
        RunRequest::RejectHold {
            run_id: "run-a1b2".into(),
            hold: "h2".into(),
        },
        RunRequest::StartGoal {
            goal: "Add password reset".into(),
            dir: PathBuf::from("/tmp/p"),
            yes: false,
            trust_project: true,
            unconfined_checks: false,
            orchestrator: None,
            delivery: None,
        },
        RunRequest::StartGoal {
            goal: "Add password reset".into(),
            dir: PathBuf::from("/tmp/p"),
            yes: true,
            trust_project: false,
            unconfined_checks: true,
            orchestrator: Some(choice.clone()),
            delivery: None,
        },
        RunRequest::Promote {
            run_id: "run-a1b2".into(),
            orchestrator: Some(choice.clone()),
        },
    ];
    for request in &requests {
        both_ways(&ClientMsg::Run(request.clone()));
        let tagged = ClientMsg::RunTagged {
            id: 41,
            request: request.clone(),
        };
        both_ways(&tagged);
    }
    // By name: the id and the hold land in their own fields.
    let packed = rmp_serde::to_vec_named(&ClientMsg::RunTagged {
        id: 41,
        request: requests[1].clone(),
    })
    .unwrap();
    let ClientMsg::RunTagged {
        id,
        request: RunRequest::RejectHold { run_id, hold },
    } = rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to a tagged RejectHold");
    };
    assert_eq!((id, run_id.as_str(), hold.as_str()), (41, "run-a1b2", "h2"));

    // An M8b-shaped `StartGoal` and `Promote`, without `orchestrator`, still decode.
    let old_goal = serde_json::json!({"StartGoal": {
        "goal": "g", "dir": "/tmp/p", "yes": false, "trust_project": false,
        "unconfined_checks": false}});
    let RunRequest::StartGoal { orchestrator, .. } = serde_json::from_value(old_goal).unwrap()
    else {
        panic!("a StartGoal");
    };
    assert_eq!(orchestrator, None);
    let old_promote = serde_json::json!({"Promote": {"run_id": "run-a1b2"}});
    let RunRequest::Promote { orchestrator, .. } = serde_json::from_value(old_promote).unwrap()
    else {
        panic!("a Promote");
    };
    assert_eq!(orchestrator, None);
}

/// M9.7 review fixes, ruling 5: `RunRequest::Edit.submit`, decision 13's user submit.
#[test]
fn edit_submit_round_trips_and_defaults_to_false() {
    let submit = ClientMsg::Run(RunRequest::Edit {
        run_id: "run-a1b2".into(),
        edits: Vec::new(),
        submit: true,
    });
    both_ways(&submit);
    // An M8c-shaped `Edit`, without `submit`, still decodes, as a plain batch.
    let old = serde_json::json!({"Edit": {"run_id": "run-a1b2", "edits": [{"op": "pause"}]}});
    let RunRequest::Edit { submit, edits, .. } = serde_json::from_value(old).unwrap() else {
        panic!("must decode as an Edit");
    };
    assert!(!submit);
    assert_eq!(edits, vec![PlanEdit::Pause]);
}
