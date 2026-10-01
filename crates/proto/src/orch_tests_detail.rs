//! Milestone 9.0.5 task 1: the task detail request and reply, and where their variants
//! sit on the wire.

use serde::de::DeserializeOwned;

use super::both_ways;
use crate::messages::{ClientMsg, DaemonMsg};
use crate::run_wire::{RunReply, RunRequest, request};
use crate::task_detail::{ACTIVITY_MAX, SummarySource, TaskDetailInfo, WORKER_SUMMARY_MAX};

fn a_detail(source: Option<SummarySource>) -> TaskDetailInfo {
    TaskDetailInfo {
        run_id: "run-a1b2".into(),
        task_id: "t2".into(),
        brief: "Add the stats command.\nKeep the parser shared.".into(),
        acceptance: vec!["mean and median print".into(), "tests pass".into()],
        worker_summary: source.map(|_| "stats: mean, median; tests added".into()),
        summary_source: source,
    }
}

#[test]
fn task_detail_round_trips() {
    assert_eq!((WORKER_SUMMARY_MAX, ACTIVITY_MAX), (1500, 160));
    let request = RunRequest::TaskDetail {
        run_id: "run-a1b2".into(),
        task_id: "t2".into(),
    };
    both_ways(&ClientMsg::Run(request.clone()));
    both_ways(&ClientMsg::RunTagged { id: 9, request });

    for (source, text) in [
        (SummarySource::TaskDone, "task_done"),
        (SummarySource::LastMessage, "last_message"),
    ] {
        assert_eq!(
            serde_json::to_string(&source).unwrap(),
            format!("\"{text}\"")
        );
        both_ways(&source);
    }
    for source in [
        None,
        Some(SummarySource::TaskDone),
        Some(SummarySource::LastMessage),
    ] {
        both_ways(&a_detail(source));
        for request_id in [None, Some(12)] {
            both_ways(&DaemonMsg::Run(RunReply::TaskDetail {
                detail: Box::new(a_detail(source)),
                request_id,
            }));
        }
    }

    // By name: the fields land in their own places.
    let reply = DaemonMsg::Run(RunReply::TaskDetail {
        detail: Box::new(a_detail(Some(SummarySource::LastMessage))),
        request_id: Some(12),
    });
    let packed = rmp_serde::to_vec_named(&reply).unwrap();
    let DaemonMsg::Run(RunReply::TaskDetail { detail, request_id }) =
        rmp_serde::from_slice(&packed).unwrap()
    else {
        panic!("must decode back to RunReply::TaskDetail");
    };
    assert_eq!(request_id, Some(12));
    assert_eq!(
        (detail.run_id.as_str(), detail.task_id.as_str()),
        ("run-a1b2", "t2")
    );
    assert_eq!(detail.acceptance[1], "tests pass");
    assert_eq!(
        detail.worker_summary.as_deref(),
        Some("stats: mean, median; tests added")
    );
    assert_eq!(detail.summary_source, Some(SummarySource::LastMessage));

    // A reply without `request_id` (an untagged request's) decodes as `None`.
    let mut json = serde_json::to_value(RunReply::TaskDetail {
        detail: Box::new(a_detail(None)),
        request_id: Some(3),
    })
    .unwrap();
    json["TaskDetail"]
        .as_object_mut()
        .unwrap()
        .remove("request_id")
        .unwrap();
    let reply: RunReply = serde_json::from_value(json).unwrap();
    assert_eq!(reply.request_id(), None);
    assert_eq!(request::TASK_DETAIL, "run task-detail");
}

#[test]
fn tagged_task_detail_echoes_its_id() {
    let reply = RunReply::TaskDetail {
        detail: Box::new(a_detail(Some(SummarySource::TaskDone))),
        request_id: None,
    };
    assert_eq!(reply.request_id(), None);
    let tagged = reply.clone().tagged(Some(41));
    assert_eq!(tagged.request_id(), Some(41));
    both_ways(&DaemonMsg::Run(tagged.clone()));
    assert_eq!(tagged.tagged(None), reply);
}

/// The variant names serde reports for an unknown one ("expected one of `Start`, …"),
/// which is their declaration order.
fn variant_names<T: DeserializeOwned + std::fmt::Debug>() -> Vec<String> {
    let error = serde_json::from_str::<T>(r#""NoSuchVariant""#)
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

/// `{index: payload}` in MessagePack: a struct variant addressed by its index, which a
/// peer that encodes variants by index would send.
fn variant_at<T: DeserializeOwned>(index: u8, payload: &serde_json::Value) -> Option<T> {
    assert!(index < 0x80, "a positive fixint");
    let mut bytes = vec![0x81, index];
    bytes.extend(rmp_serde::to_vec_named(payload).unwrap());
    rmp_serde::from_slice(&bytes).ok()
}

#[test]
fn appended_variants_keep_their_indices() {
    let requests = [
        "Start",
        "Approve",
        "Reject",
        "Edit",
        "Retry",
        "Override",
        "Cancel",
        "Resume",
        "Finish",
        "List",
        "Subscribe",
        "Unsubscribe",
        "Tool",
        "StartGoal",
        "Promote",
        "Stats",
        "Profile",
        "ApproveHold",
        "RejectHold",
    ];
    let mut expected: Vec<&str> = requests.to_vec();
    expected.push("TaskDetail");
    expected.push("Settings");
    // Milestone 9.2 appends `Deliver` and `Watch` after 9.0.6's `Settings` (`delivery_tests.rs`).
    expected.extend(["Deliver", "Watch"]);
    assert_eq!(variant_names::<RunRequest>(), expected);
    assert_eq!(
        variant_at::<RunRequest>(
            requests.len() as u8,
            &serde_json::json!({"run_id": "r1", "task_id": "t1"})
        ),
        Some(RunRequest::TaskDetail {
            run_id: "r1".into(),
            task_id: "t1".into(),
        })
    );

    let replies = [
        "Started",
        "Done",
        "Refused",
        "ConfirmNeeded",
        "Snapshot",
        "ToolResult",
        "Triaged",
        "Profile",
        "Stats",
    ];
    let mut expected: Vec<&str> = replies.to_vec();
    expected.push("TaskDetail");
    expected.push("Settings");
    assert_eq!(variant_names::<RunReply>(), expected);
    let detail = a_detail(None);
    assert_eq!(
        variant_at::<RunReply>(
            replies.len() as u8,
            &serde_json::json!({"detail": detail, "request_id": 5})
        ),
        Some(RunReply::TaskDetail {
            detail: Box::new(detail),
            request_id: Some(5),
        })
    );
}
