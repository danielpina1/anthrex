//! Decisions 16 and 17, the pure parts: schemas, parsing, answer extraction,
//! and fallbacks. The prompts are in `tests_prompt.rs`, the argv in `tests_argv.rs`.

use super::fallback::{fallback, fallback_decision};
use super::parse::{answer_from_events, parse, parse_for};
use super::schema::schema;
use super::*;
use crate::headless::SessionEvent;
use crate::headless::claude_stream::ClaudeStream;
use proto::{DeciderSource, Scale, Size, TaskKind, TestMode};
use serde_json::{Value, json};

const TRIAGE_FIXTURE: &str =
    include_str!("../../tests/fixtures/deciders/claude-2.1.280-decider-triage.jsonl");
const BLOCKED_FIXTURE: &str =
    include_str!("../../tests/fixtures/deciders/claude-2.1.280-decider.jsonl");
const BRIEF: &str = include_str!("../../../../docs/milestones/M8b-adaptation.md");

fn stream_events(fixture: &str) -> Vec<SessionEvent> {
    let mut stream = ClaudeStream::default();
    fixture
        .lines()
        .filter(|l| !l.trim().is_empty())
        .flat_map(|l| stream.parse_line(l))
        .collect()
}

// ---- schemas ------------------------------------------------------------------------

fn assert_closed(value: &Value, path: &str) {
    match value {
        Value::Object(map) => {
            if map.get("type") == Some(&json!("object")) {
                assert_eq!(
                    map.get("additionalProperties"),
                    Some(&json!(false)),
                    "{path}: not closed"
                );
                let props = map["properties"].as_object().unwrap();
                let required: Vec<&str> = map["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                let mut names: Vec<&str> = props.keys().map(String::as_str).collect();
                let mut sorted = required.clone();
                names.sort();
                sorted.sort();
                assert_eq!(names, sorted, "{path}: not every property is required");
            }
            for (key, child) in map {
                assert_closed(child, &format!("{path}.{key}"));
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                assert_closed(child, &format!("{path}[{i}]"));
            }
        }
        _ => {}
    }
}

#[test]
fn every_schema_is_closed_and_strict() {
    for kind in DeciderKind::ALL {
        let s = schema(kind);
        assert_eq!(s["type"], "object", "{}", kind.label());
        assert_closed(&s, kind.label());
    }
}

#[test]
fn schemas_match_the_brief() {
    let block = BRIEF
        .split("### Decider schemas (exact)")
        .nth(1)
        .unwrap()
        .split("```json\n")
        .nth(1)
        .unwrap()
        .split("\n```")
        .next()
        .unwrap();
    let brief: Value = serde_json::from_str(block).expect("the brief's schema JSON parses");
    // M8b's four kinds; milestone 9.2's `ci_summary` is pinned against its own brief
    // (`tests_ci.rs`).
    let m8b = &DeciderKind::ALL[..4];
    for kind in m8b {
        assert_eq!(schema(*kind), brief[kind.label()], "{}", kind.label());
    }
    assert_eq!(brief.as_object().unwrap().len(), m8b.len());
}

// ---- parsing ------------------------------------------------------------------------

fn triage_value() -> Value {
    json!({"kinds":["docs"],"scale":"single","reason":"Single file","task":{
        "title":"Fix typo","brief":"Fix it.","acceptance":["fixed"],"owns":["README.md"],
        "size":"S","interface_change":false,"test_mode":"none",
        "test_mode_reason":"Docs only","test_to_write":null}})
}

#[test]
fn parse_accepts_each_valid_answer() {
    assert_eq!(
        parse(DeciderKind::Triage, &triage_value()).unwrap(),
        DeciderAnswer::Triage(TriageAnswer {
            kinds: vec![TaskKind::Docs],
            scale: Scale::Single,
            reason: "Single file".into(),
            task: Some(TriageTask {
                title: "Fix typo".into(),
                brief: "Fix it.".into(),
                acceptance: vec!["fixed".into()],
                owns: vec!["README.md".into()],
                size: Size::S,
                interface_change: false,
                test_mode: TestMode::None,
                test_mode_reason: Some("Docs only".into()),
                test_to_write: None,
            }),
        })
    );
    // A plan-scale answer's task is ignored.
    let plan = json!({"kinds":["code","docs"],"scale":"plan","reason":"r","task":{"bogus":1}});
    assert_eq!(
        parse(DeciderKind::Triage, &plan).unwrap(),
        DeciderAnswer::Triage(TriageAnswer {
            kinds: vec![TaskKind::Code, TaskKind::Docs],
            scale: Scale::Plan,
            reason: "r".into(),
            task: None,
        })
    );
    // The real triage answer M8b.1 recorded.
    let events = stream_events(TRIAGE_FIXTURE);
    let value = answer_from_events(&events).unwrap();
    assert!(matches!(
        parse(DeciderKind::Triage, &value).unwrap(),
        DeciderAnswer::Triage(TriageAnswer {
            scale: Scale::Single,
            task: Some(_),
            ..
        })
    ));
    assert_eq!(
        parse(
            DeciderKind::SizeCheck,
            &json!({"tasks":[{"id":"t1","size":"M","reason":"two files"}]})
        )
        .unwrap(),
        DeciderAnswer::SizeCheck(vec![SizeVerdict {
            id: "t1".into(),
            size: Size::M,
            reason: "two files".into()
        }])
    );
    assert_eq!(
        parse(DeciderKind::CheckSummary, &json!({"lines":["a","b"]})).unwrap(),
        DeciderAnswer::CheckSummary {
            lines: vec!["a".into(), "b".into()]
        }
    );
    // The real blocked_reason answer M8b.1 recorded.
    let value = answer_from_events(&stream_events(BLOCKED_FIXTURE)).unwrap();
    assert!(matches!(
        parse(DeciderKind::BlockedReason, &value).unwrap(),
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Environment,
            ..
        }
    ));
    assert_eq!(
        parse(
            DeciderKind::BlockedReason,
            &json!({"kind":"mis_sized","reason":"too big"})
        )
        .unwrap(),
        DeciderAnswer::BlockedReason {
            kind: BlockKind::MisSized,
            reason: "too big".into()
        }
    );
}

#[test]
fn parse_errors_name_the_path() {
    assert_eq!(
        parse(DeciderKind::Triage, &json!({})).unwrap_err(),
        "kinds: missing"
    );
    assert_eq!(
        parse(DeciderKind::Triage, &json!([])).unwrap_err(),
        "expected an object"
    );
    let mut v = triage_value();
    v["extra"] = json!(1);
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "extra: not in the schema"
    );
    let mut v = triage_value();
    v["kinds"] = json!([]);
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "kinds: must have at least 1 item"
    );
    let mut v = triage_value();
    v["kinds"] = json!(["code", "art"]);
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "kinds[1]: expected code, docs, research or review"
    );
    let mut v = triage_value();
    v["reason"] = json!("");
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "reason: must not be empty"
    );
    let mut v = triage_value();
    v["task"]["acceptance"] = json!([1]);
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "task.acceptance[0]: expected a string"
    );
    let mut v = triage_value();
    v["task"]["interface_change"] = json!("no");
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "task.interface_change: expected a boolean"
    );
}

#[test]
fn triage_single_without_task_is_an_error() {
    let mut v = triage_value();
    v["task"] = Value::Null;
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "task: required when scale is single"
    );
}

#[test]
fn triage_task_size_l_is_an_error() {
    let mut v = triage_value();
    v["task"]["size"] = json!("L");
    assert_eq!(
        parse(DeciderKind::Triage, &v).unwrap_err(),
        "task.size: expected S or M"
    );
}

#[test]
fn size_check_ignores_unknown_ids_and_reports_bad_sizes() {
    let request = DeciderRequest::SizeCheck(SizeCheckInput {
        tasks: ["t1", "t2"]
            .iter()
            .map(|id| SizeCheckTask {
                id: (*id).into(),
                title: "T".into(),
                brief: "B".into(),
                acceptance: vec![],
                owns: vec![],
                deps: vec![],
                size: Size::S,
                interface_change: false,
                hub: false,
            })
            .collect(),
        evidence_refs: vec![],
        evidence: vec![],
        modules: vec![],
        hub: vec![],
    });
    let answer = json!({"tasks":[
        {"id":"t2","size":"L","reason":"many modules"},
        {"id":"t9","size":"M","reason":"not asked"},
        {"id":"t2","size":"S","reason":"a repeat"}]});
    assert_eq!(
        parse_for(&request, &answer).unwrap(),
        DeciderAnswer::SizeCheck(vec![SizeVerdict {
            id: "t2".into(),
            size: Size::L,
            reason: "many modules".into()
        }])
    );
    let bad = json!({"tasks":[
        {"id":"t1","size":"S","reason":"r"},
        {"id":"t2","size":"M","reason":"r"},
        {"id":"t1","size":"XL","reason":"r"}]});
    assert_eq!(
        parse_for(&request, &bad).unwrap_err(),
        "tasks[2].size: expected S, M or L"
    );
    let long_id = json!({"tasks":[{"id":"t".repeat(17),"size":"S","reason":"r"}]});
    assert_eq!(
        parse(DeciderKind::SizeCheck, &long_id).unwrap_err(),
        "tasks[0].id: must be at most 16 characters"
    );
}

#[test]
fn check_summary_over_40_lines_or_300_chars_is_an_error() {
    let lines: Vec<String> = (0..41).map(|i| i.to_string()).collect();
    assert_eq!(
        parse(DeciderKind::CheckSummary, &json!({ "lines": lines })).unwrap_err(),
        "lines: must have at most 40 items"
    );
    let forty = &lines[..40];
    assert!(parse(DeciderKind::CheckSummary, &json!({ "lines": forty })).is_ok());
    let wide = "世".repeat(300);
    assert_eq!(wide.len(), 900);
    assert!(parse(DeciderKind::CheckSummary, &json!({"lines": [wide]})).is_ok());
    let wider = "世".repeat(301);
    assert_eq!(
        parse(DeciderKind::CheckSummary, &json!({"lines": ["ok", wider]})).unwrap_err(),
        "lines[1]: must be at most 300 characters"
    );
    assert_eq!(
        parse(DeciderKind::CheckSummary, &json!({"lines": []})).unwrap_err(),
        "lines: must have at least 1 item"
    );
}

#[test]
fn blocked_reason_unknown_kind_is_an_error() {
    assert_eq!(
        parse(
            DeciderKind::BlockedReason,
            &json!({"kind":"bored","reason":"r"})
        )
        .unwrap_err(),
        "kind: expected question, mis_sized or environment"
    );
}

// ---- answer extraction --------------------------------------------------------------

fn text(t: &str, parent: Option<&str>) -> SessionEvent {
    SessionEvent::AssistantText {
        text: t.into(),
        parent: parent.map(String::from),
    }
}

fn tool_use(input: Value, parent: Option<&str>) -> SessionEvent {
    SessionEvent::ToolUse {
        id: "toolu_1".into(),
        name: "StructuredOutput".into(),
        input,
        parent: parent.map(String::from),
    }
}

#[test]
fn structured_output_event_wins() {
    let events = vec![
        text("{\"from\":\"text\"}", None),
        tool_use(json!({"from":"tool"}), None),
        SessionEvent::StructuredOutput {
            value: json!({"from":"result"}),
        },
    ];
    assert_eq!(
        answer_from_events(&events).unwrap(),
        json!({"from":"result"})
    );
}

#[test]
fn structured_output_tool_use_is_read() {
    let events = vec![
        text("{\"from\":\"text\"}", None),
        tool_use(json!({"from":"subagent"}), Some("toolu_0")),
        tool_use(json!({"from":"tool"}), None),
    ];
    assert_eq!(answer_from_events(&events).unwrap(), json!({"from":"tool"}));
    // A sub-agent's StructuredOutput after the top-level one is still not the answer,
    // so position alone cannot pick the right one.
    let events = vec![
        tool_use(json!({"from":"tool"}), None),
        tool_use(json!({"from":"subagent"}), Some("toolu_0")),
    ];
    assert_eq!(answer_from_events(&events).unwrap(), json!({"from":"tool"}));
    let only_subagent = vec![tool_use(json!({"from":"subagent"}), Some("toolu_0"))];
    assert_eq!(
        answer_from_events(&only_subagent).unwrap_err(),
        "the session gave no answer"
    );
    // The recorded triage call: without its result's structured output (M8b.7), its
    // StructuredOutput tool use is the answer.
    let events: Vec<SessionEvent> = stream_events(TRIAGE_FIXTURE)
        .into_iter()
        .filter(|e| !matches!(e, SessionEvent::StructuredOutput { .. }))
        .collect();
    assert!(
        events
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolUse { .. }))
    );
    let answer = answer_from_events(&events).unwrap();
    assert_eq!(answer["scale"], "single");
    assert_eq!(answer["task"]["owns"], json!(["README.md"]));
}

#[test]
fn assistant_text_in_a_fence_is_parsed() {
    // The recorded triage call answered first in fenced text; without its tool use the
    // fenced text is the answer (ruling R-T1-4's case, a turn cut off at --max-turns).
    let events: Vec<SessionEvent> = stream_events(TRIAGE_FIXTURE)
        .into_iter()
        .filter(|e| {
            !matches!(
                e,
                SessionEvent::ToolUse { .. } | SessionEvent::StructuredOutput { .. }
            )
        })
        .collect();
    let answer = answer_from_events(&events).unwrap();
    assert_eq!(answer["kinds"], json!(["docs"]));
    for body in [
        "```json\n{\"a\":1}\n```",
        "  ```\n{\"a\":1}\n```  \n",
        "{\"a\":1}",
        "\n```JSON\n{\"a\":1}```",
    ] {
        assert_eq!(
            answer_from_events(&[text("ignored", None), text(body, None)]).unwrap(),
            json!({"a":1}),
            "{body:?}"
        );
    }
    let err = answer_from_events(&[text("not json", None)]).unwrap_err();
    assert!(err.starts_with("expected"), "{err}");
    assert!(super::parse::json_from_text("```\n[1]\n```").is_ok());
}

#[test]
fn subagent_text_is_ignored() {
    let events = vec![text("{\"a\":1}", None), text("{\"b\":2}", Some("toolu_9"))];
    assert_eq!(answer_from_events(&events).unwrap(), json!({"a":1}));
    assert!(answer_from_events(&[text("{\"b\":2}", Some("toolu_9"))]).is_err());
}

#[test]
fn no_answer_is_an_error() {
    assert_eq!(
        answer_from_events(&[SessionEvent::TurnStarted]).unwrap_err(),
        "the session gave no answer"
    );
    assert!(answer_from_events(&[]).is_err());
}

// ---- fallbacks ----------------------------------------------------------------------

#[test]
fn fallback_table() {
    let triage = DeciderRequest::Triage(TriageInput {
        goal: "G".into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec![],
        files_total: 0,
        planner_task_cap: 12,
    });
    let DeciderAnswer::Triage(t) = fallback(&triage) else {
        panic!("not triage")
    };
    assert_eq!(
        (t.kinds, t.scale, t.task),
        (vec![TaskKind::Code], Scale::Plan, None)
    );
    assert!(!t.reason.is_empty());

    let task = |id: &str, size| SizeCheckTask {
        id: id.into(),
        title: "T".into(),
        brief: "B".into(),
        acceptance: vec![],
        owns: vec![],
        deps: vec![],
        size,
        interface_change: false,
        hub: false,
    };
    let sc = DeciderRequest::SizeCheck(SizeCheckInput {
        tasks: vec![task("t1", Size::S), task("t2", Size::M)],
        evidence_refs: vec![],
        evidence: vec![],
        modules: vec![],
        hub: vec![],
    });
    let DeciderAnswer::SizeCheck(verdicts) = fallback(&sc) else {
        panic!("not size_check")
    };
    let sizes: Vec<(&str, Size)> = verdicts.iter().map(|v| (v.id.as_str(), v.size)).collect();
    assert_eq!(sizes, [("t1", Size::S), ("t2", Size::M)]);

    let tail: String = (0..100).map(|i| format!("line {i}\n")).collect();
    let cs = DeciderRequest::CheckSummary(CheckSummaryInput {
        task_id: "t1".into(),
        command: "make".into(),
        code: Some(2),
        timed_out: false,
        tail: tail.clone(),
    });
    let DeciderAnswer::CheckSummary { lines } = fallback(&cs) else {
        panic!("not check_summary")
    };
    assert_eq!(lines.join("\n"), crate::run::exec::summary(&tail));

    let br = DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "T".into(),
        reason: "why".into(),
    });
    assert!(matches!(
        fallback(&br),
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Question,
            ..
        }
    ));
}

#[test]
fn fallback_decision_carries_the_reason() {
    let br = DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "T".into(),
        reason: "why".into(),
    });
    let d = fallback_decision(&br, "deciders are off".into());
    assert_eq!(d.kind, DeciderKind::BlockedReason);
    assert_eq!(d.source, DeciderSource::Fallback);
    assert_eq!(d.fallback_reason.as_deref(), Some("deciders are off"));
    assert_eq!(d.answer, fallback(&br));
    assert_eq!((d.usage, d.secs), (None, 0));
    // A decision survives serde, as `OpKind::Decide`'s result is persisted.
    let back: Decision = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(back, d);
}
