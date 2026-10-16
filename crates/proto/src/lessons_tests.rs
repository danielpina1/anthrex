//! Task M9.15.3: the lessons store's lines (decision 2) and the lesson wire types
//! (decision 32) round-trip through JSON and MessagePack.

use std::fmt::Debug;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::lessons::*;
use crate::lessons_store::*;
use crate::run_info::TokenUsage;

fn round_trip<T: Serialize + DeserializeOwned + PartialEq + Debug>(value: &T) {
    let json = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{e}: {json}"));
    assert_eq!(&back, value, "JSON: {json}");
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value, "MessagePack");
}

fn usage() -> TokenUsage {
    TokenUsage {
        input: 10,
        output: 20,
        cache_read: 30,
        cache_write: 40,
    }
}

fn draft(layer: LessonLayer, title: &str) -> LessonDraft {
    LessonDraft {
        layer,
        title: title.to_string(),
        body: "a body".to_string(),
        files: vec!["crates/a/**".to_string()],
    }
}

/// One line of every type, the optional fields both set and left out.
fn every_line() -> Vec<LessonLine> {
    let queued = "add-a-ff9f/t1/distill".to_string();
    vec![
        LessonLine::Queued(QueuedLine {
            v: LESSONS_VERSION,
            record_id: queued.clone(),
            at: 100,
            run_id: "add-a-ff9f".to_string(),
            task_id: Some("t1".to_string()),
            cause: DistillCause::TaskRecovered,
            dossier: "## outcome\nrecovered".to_string(),
            fps: vec!["0123456789abcdef".to_string()],
        }),
        LessonLine::Queued(QueuedLine {
            v: LESSONS_VERSION,
            record_id: "add-a-ff9f/run/distill".to_string(),
            at: 101,
            run_id: "add-a-ff9f".to_string(),
            task_id: None,
            cause: DistillCause::RunDiscarded,
            dossier: String::new(),
            fps: Vec::new(),
        }),
        LessonLine::Attempt(AttemptLine {
            v: LESSONS_VERSION,
            record_id: format!("{queued}/attempt/1"),
            at: 102,
            queued: queued.clone(),
            n: 1,
            error: Some("timed out".to_string()),
            usage: None,
        }),
        LessonLine::Attempt(AttemptLine {
            v: LESSONS_VERSION,
            record_id: format!("{queued}/attempt/2"),
            at: 103,
            queued: queued.clone(),
            n: 2,
            error: None,
            usage: Some(usage()),
        }),
        LessonLine::Distilled(DistilledLine {
            v: LESSONS_VERSION,
            record_id: format!("{queued}/distilled"),
            at: 104,
            queued: queued.clone(),
            items: vec![
                draft(LessonLayer::Episodic, "e"),
                draft(LessonLayer::Semantic, "s"),
                draft(LessonLayer::Procedural, "p"),
            ],
            dropped: 2,
            usage: Some(usage()),
        }),
        LessonLine::GaveUp(GaveUpLine {
            v: LESSONS_VERSION,
            record_id: format!("{queued}/gave_up"),
            at: 105,
            queued,
            reason: "deciders are off".to_string(),
        }),
        LessonLine::Verdict(VerdictLine {
            v: LESSONS_VERSION,
            record_id: "l-0123456789abcdef/verdict/106".to_string(),
            at: 106,
            id: "l-0123456789abcdef".to_string(),
            approved: true,
        }),
        LessonLine::Retired(RetiredLine {
            v: LESSONS_VERSION,
            record_id: "l-0123456789abcdef/retired".to_string(),
            at: 107,
            id: "l-0123456789abcdef".to_string(),
            reason: "forgotten by the user".to_string(),
        }),
        LessonLine::Dismissed(DismissedLine {
            v: LESSONS_VERSION,
            record_id: "kf-0123456789abcdef/dismissed".to_string(),
            at: 108,
            fp: "0123456789abcdef".to_string(),
        }),
    ]
}

#[test]
fn every_lesson_line_round_trips() {
    for line in every_line() {
        round_trip(&line);
    }
}

#[test]
fn a_lesson_line_is_tagged_by_type_in_snake_case() {
    let tags: Vec<String> = every_line()
        .iter()
        .map(|line| {
            serde_json::to_value(line).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        tags,
        [
            "queued",
            "queued",
            "attempt",
            "attempt",
            "distilled",
            "gave_up",
            "verdict",
            "retired",
            "dismissed"
        ]
    );
    let queued = serde_json::to_value(&every_line()[0]).unwrap();
    assert_eq!(queued["cause"], "task_recovered");
    assert_eq!(queued["v"], 1);
    let distilled = serde_json::to_value(&every_line()[4]).unwrap();
    assert_eq!(distilled["items"][2]["layer"], "procedural");
    // An unset option is left out of the line.
    let run = serde_json::to_value(&every_line()[1]).unwrap();
    assert!(run.get("task_id").is_none(), "{run}");
}

#[test]
fn an_unknown_line_type_fails_to_decode_alone() {
    let text = [
        r#"{"type":"lesson","v":1,"record_id":"x","at":1,"title":"t"}"#,
        r#"{"type":"dismissed","v":1,"record_id":"kf-ab/dismissed","at":2,"fp":"ab"}"#,
        r#"{"type":"retired","v":1,"record_id":"l-1/retired","at":3,"id":"l-1""#,
        r#"{"type":"verdict","v":2,"record_id":"l-1/verdict/4","at":4,"id":"l-1","approved":false,"later":true}"#,
    ];
    let decoded: Vec<bool> = text
        .iter()
        .map(|line| serde_json::from_str::<LessonLine>(line).is_ok())
        .collect();
    // The unknown type and the torn line fail; the others (one with a field this
    // version does not know) still decode.
    assert_eq!(decoded, [false, true, false, true]);
}

#[test]
fn every_cause_and_layer_round_trips() {
    for cause in [
        DistillCause::TaskBlocked,
        DistillCause::TaskRecovered,
        DistillCause::ReviewBlocking,
        DistillCause::RunDiscarded,
        DistillCause::RunFailed,
    ] {
        round_trip(&cause);
    }
    assert_eq!(
        serde_json::to_string(&DistillCause::ReviewBlocking).unwrap(),
        "\"review_blocking\""
    );
    for status in [
        LessonStatus::OnTrial,
        LessonStatus::Kept,
        LessonStatus::Proposed,
        LessonStatus::Approved,
        LessonStatus::Rejected,
        LessonStatus::Retired,
        LessonStatus::Known,
    ] {
        round_trip(&status);
    }
    assert_eq!(
        serde_json::to_string(&LessonStatus::OnTrial).unwrap(),
        "\"on_trial\""
    );
}

fn info() -> LessonInfo {
    LessonInfo {
        id: "l-0123456789abcdef".to_string(),
        layer: LessonLayer::Semantic,
        status: LessonStatus::Proposed,
        origin: LessonOrigin::Distilled,
        title: "run cargo fmt before claiming done".to_string(),
        body: "CI fails on formatting.".to_string(),
        files: vec!["crates/**".to_string()],
        created_at: 1,
        last_seen: 2,
        runs: vec!["add-a-ff9f".to_string()],
        green: 3,
        outcomes: 4,
        reconfirmed: 1,
        headlines: vec!["error: x".to_string()],
        retired_reason: Some("trial ended".to_string()),
    }
}

#[test]
fn lesson_wire_types_round_trip() {
    round_trip(&info());
    let dir = PathBuf::from("/repo");
    for request in [
        LessonsRequest::List {
            dir: dir.clone(),
            all: true,
        },
        LessonsRequest::Decide {
            dir: dir.clone(),
            ids: vec!["l-012345".to_string()],
            approve: false,
        },
        LessonsRequest::Forget {
            dir: dir.clone(),
            ids: vec!["kf-0123".to_string()],
        },
        LessonsRequest::Graph { dir: dir.clone() },
    ] {
        round_trip(&request);
    }
    round_trip(&LessonGraph {
        project: dir,
        nodes: vec![
            GraphNode {
                id: "m:(root)".to_string(),
                kind: GraphNodeKind::Root,
                label: "(root)".to_string(),
                status: None,
                layer: None,
            },
            GraphNode {
                id: "l-0123456789abcdef".to_string(),
                kind: GraphNodeKind::Lesson,
                label: "run cargo fmt".to_string(),
                status: Some(LessonStatus::Approved),
                layer: Some(LessonLayer::Procedural),
            },
        ],
        edges: vec![GraphEdge {
            from: "l-0123456789abcdef".to_string(),
            to: "m:(root)".to_string(),
            kind: GraphEdgeKind::Touches,
        }],
        lessons: vec![info()],
        omitted: 7,
        module_source: ModuleSource::CargoCache,
    });
}

#[test]
fn a_minimal_lesson_info_decodes_with_defaults() {
    let text = r#"{"id":"kf-ab","layer":"episodic","status":"known","origin":"signature",
        "title":"Known failure: x","body":"b","created_at":1,"last_seen":2}"#;
    let info: LessonInfo = serde_json::from_str(text).unwrap();
    assert!(info.files.is_empty() && info.runs.is_empty() && info.headlines.is_empty());
    assert_eq!((info.green, info.outcomes, info.reconfirmed), (0, 0, 0));
    assert_eq!(info.retired_reason, None);
    let graph: LessonGraph =
        serde_json::from_str(r#"{"project":"/r","nodes":[],"edges":[]}"#).unwrap();
    assert_eq!(graph.module_source, ModuleSource::None);
}

// Task M9.15.5: protocol 24 (decision 32).

use crate::actions::InputKind;
use crate::delivery::tests::variant_names;
use crate::models::HelperKind;
use crate::run::FinishAction;
use crate::run_wire::{RunReply, RunRequest, request};
use crate::task_detail::{SummarySource, TaskDetailInfo};

fn graph() -> LessonGraph {
    LessonGraph {
        project: PathBuf::from("/repo"),
        nodes: vec![
            GraphNode {
                id: "m:core".to_string(),
                kind: GraphNodeKind::Module,
                label: "core".to_string(),
                status: None,
                layer: None,
            },
            GraphNode {
                id: "kf-0123456789abcdef".to_string(),
                kind: GraphNodeKind::Lesson,
                label: "Known failure: boom".to_string(),
                status: Some(LessonStatus::Known),
                layer: Some(LessonLayer::Episodic),
            },
        ],
        edges: vec![
            GraphEdge {
                from: "kf-0123456789abcdef".to_string(),
                to: "m:core".to_string(),
                kind: GraphEdgeKind::Touches,
            },
            GraphEdge {
                from: "l-0123456789abcdef".to_string(),
                to: "kf-0123456789abcdef".to_string(),
                kind: GraphEdgeKind::FromSignature,
            },
        ],
        lessons: vec![info()],
        omitted: 2,
        module_source: ModuleSource::Profile,
    }
}

fn every_lessons_request() -> Vec<LessonsRequest> {
    let dir = PathBuf::from("/repo");
    vec![
        LessonsRequest::List {
            dir: dir.clone(),
            all: false,
        },
        LessonsRequest::Decide {
            dir: dir.clone(),
            ids: vec!["l-012345".to_string(), "kf-0123456789abcdef".to_string()],
            approve: true,
        },
        LessonsRequest::Forget {
            dir: dir.clone(),
            ids: Vec::new(),
        },
        LessonsRequest::Graph { dir },
    ]
}

#[test]
fn every_lessons_request_round_trips_inside_run_request() {
    for lessons in every_lessons_request() {
        round_trip(&lessons);
        round_trip(&RunRequest::Lessons(lessons));
    }
}

#[test]
fn lessons_replies_round_trip() {
    for reply in [
        RunReply::Lessons {
            lessons: vec![info()],
            message: Some("approved 1".to_string()),
            request_id: Some(7),
        },
        RunReply::Lessons {
            lessons: Vec::new(),
            message: None,
            request_id: None,
        },
        RunReply::LessonsGraph {
            graph: Box::new(graph()),
            request_id: Some(9),
        },
        RunReply::LessonsGraph {
            graph: Box::default(),
            request_id: None,
        },
    ] {
        round_trip(&reply);
    }
    round_trip(&info());
    round_trip(&graph());
}

#[test]
fn lessons_replies_carry_their_request_id() {
    let list = RunReply::Lessons {
        lessons: Vec::new(),
        message: None,
        request_id: None,
    };
    assert_eq!(list.tagged(Some(4)).request_id(), Some(4));
    let graph = RunReply::LessonsGraph {
        graph: Box::default(),
        request_id: None,
    };
    assert_eq!(graph.tagged(Some(5)).request_id(), Some(5));
}

#[test]
fn a_lessons_reply_decodes_without_its_defaulted_fields() {
    let reply: RunReply = serde_json::from_str(r#"{"Lessons":{"lessons":[]}}"#).unwrap();
    assert_eq!(
        reply,
        RunReply::Lessons {
            lessons: Vec::new(),
            message: None,
            request_id: None,
        }
    );
    let text = r#"{"LessonsGraph":{"graph":{"project":"/r","nodes":[],"edges":[]}}}"#;
    let reply: RunReply = serde_json::from_str(text).unwrap();
    let RunReply::LessonsGraph { graph, request_id } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!((graph.omitted, request_id), (0, None));
}

fn finish(reason: Option<&str>) -> RunRequest {
    RunRequest::Finish {
        run_id: "add-a-ff9f".to_string(),
        action: FinishAction::Discard,
        confirm: Some("add-a-ff9f".to_string()),
        reason: reason.map(str::to_string),
    }
}

#[test]
fn finish_reason_round_trip() {
    round_trip(&finish(Some(
        "wrong approach: the cache belongs in the client",
    )));
    round_trip(&finish(None));
    // Left out while unset, so the request reads as protocol 23's.
    let json = serde_json::to_value(finish(None)).unwrap();
    assert!(json["Finish"].get("reason").is_none(), "{json}");
}

#[test]
fn old_finish_decodes_without_reason() {
    let old = serde_json::json!({"Finish": {
        "run_id": "add-a-ff9f", "action": "discard", "confirm": "add-a-ff9f"
    }});
    let from_json: RunRequest = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(from_json, finish(None));
    let packed = rmp_serde::to_vec_named(&old).unwrap();
    let from_pack: RunRequest = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(from_pack, finish(None));
}

#[test]
fn the_new_variants_are_appended_last() {
    // Milestone 9.16 appends `Hold` and `Held` after these (`brakes_tests.rs`).
    let requests = variant_names::<RunRequest>();
    assert_eq!(requests[requests.len() - 2], "Lessons");
    assert_eq!(requests[requests.len() - 3], "AnswerAsk");
    let replies = variant_names::<RunReply>();
    assert_eq!(
        &replies[replies.len() - 4..replies.len() - 1],
        ["Queued", "Lessons", "LessonsGraph"]
    );
    assert_eq!(
        HelperKind::ALL[HelperKind::ALL.len() - 2..],
        [HelperKind::Distill, HelperKind::RiskNote]
    );
    assert_eq!(HelperKind::ALL.len(), 8);
    let inputs = variant_names::<InputKind>();
    assert_eq!(inputs.last().map(String::as_str), Some("discard_reason"));
    assert_eq!(inputs[inputs.len() - 2], "promote");
    round_trip(&InputKind::DiscardReason);
}

#[test]
fn the_new_helpers_have_their_keys_and_labels() {
    assert_eq!(HelperKind::Distill.key(), "distill");
    assert_eq!(HelperKind::RiskNote.key(), "risk_note");
    assert_eq!(HelperKind::Distill.label(), "distill");
    assert_eq!(HelperKind::RiskNote.label(), "risk note");
    round_trip(&HelperKind::Distill);
    round_trip(&HelperKind::RiskNote);
}

fn detail(risk_note: Option<&str>) -> TaskDetailInfo {
    TaskDetailInfo {
        run_id: "add-a-ff9f".to_string(),
        task_id: "t1".to_string(),
        brief: "do it".to_string(),
        acceptance: vec!["it works".to_string()],
        worker_summary: None,
        summary_source: Some(SummarySource::TaskDone),
        risk_note: risk_note.map(str::to_string),
    }
}

#[test]
fn task_detail_risk_note_defaults() {
    round_trip(&detail(Some("- check the lock order first")));
    round_trip(&detail(None));
    let json = serde_json::to_value(detail(None)).unwrap();
    assert!(json.get("risk_note").is_none(), "{json}");
    let old = serde_json::json!({
        "run_id": "add-a-ff9f", "task_id": "t1", "brief": "do it",
        "acceptance": ["it works"], "worker_summary": null, "summary_source": "task_done"
    });
    let from_json: TaskDetailInfo = serde_json::from_value(old.clone()).unwrap();
    assert_eq!(from_json, detail(None));
    let packed = rmp_serde::to_vec_named(&old).unwrap();
    let from_pack: TaskDetailInfo = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(from_pack, detail(None));
}

#[test]
fn the_lessons_request_label() {
    assert_eq!(request::LESSONS, "lessons");
}

#[test]
fn status_and_layer_names_are_their_wire_names() {
    for status in [
        LessonStatus::OnTrial,
        LessonStatus::Kept,
        LessonStatus::Proposed,
        LessonStatus::Approved,
        LessonStatus::Rejected,
        LessonStatus::Retired,
        LessonStatus::Known,
    ] {
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            serde_json::json!(status.name())
        );
    }
    for layer in [
        LessonLayer::Episodic,
        LessonLayer::Semantic,
        LessonLayer::Procedural,
    ] {
        assert_eq!(
            serde_json::to_value(layer).unwrap(),
            serde_json::json!(layer.name())
        );
    }
}
