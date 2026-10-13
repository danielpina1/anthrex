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
