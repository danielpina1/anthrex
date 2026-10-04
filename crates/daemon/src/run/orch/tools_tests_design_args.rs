//! Milestone 9.6 task M9.6.6: each design tool's arguments, parsed into its call and
//! bounded by the daemon (Interfaces "MCP tools"); in its own file beside
//! `tools_tests_design.rs` to keep each under 400 lines. Pure.

use proto::{AgentRole, DocFinding, DocKind, DocSeverity, FindingAnswer};
use serde_json::{Value, json};

use super::*;

fn orch(tool: &str, args: Value) -> Result<OrchCall, String> {
    parse_call(AgentRole::Orchestrator, tool, &args)
}

fn answer(id: &str, answer: &str) -> FindingAnswer {
    FindingAnswer {
        id: id.into(),
        answer: answer.into(),
    }
}

/// Each design tool from its own role parses into its call; `submit_doc`'s kinds are
/// its role's (the orchestrator's `brainstorm` and `spec`, a brainstormer's
/// `brainstorm_draft`), and its optional fields default.
#[test]
fn design_tools_parse_into_their_calls() {
    assert_eq!(
        orch("start_brainstorm", json!({"answers": ""})),
        Ok(OrchCall::StartBrainstorm {
            answers: String::new()
        })
    );
    let answers = "é".repeat(4096); // 8 KiB exactly
    assert_eq!(
        orch("start_brainstorm", json!({"answers": answers})),
        Ok(OrchCall::StartBrainstorm { answers })
    );
    assert_eq!(
        orch(
            "submit_doc",
            json!({"kind": "spec", "text": "# T", "ready": true, "amend": true,
                   "responses": [{"id": "F1", "answer": "fixed"}]})
        ),
        Ok(OrchCall::SubmitDoc(SubmitDoc {
            kind: DocKind::Spec,
            text: "# T".into(),
            ready: true,
            amend: true,
            responses: vec![answer("F1", "fixed")],
        }))
    );
    assert_eq!(
        orch("submit_doc", json!({"kind": "brainstorm", "text": "x"})),
        Ok(OrchCall::SubmitDoc(SubmitDoc {
            kind: DocKind::Brainstorm,
            text: "x".into(),
            ready: false,
            amend: false,
            responses: Vec::new(),
        }))
    );
    // A text's cap is the engine's template check, with its exact text (decision 15):
    // the parse bounds no document's length.
    let long = "x".repeat(200_000);
    assert!(matches!(
        orch("submit_doc", json!({"kind": "spec", "text": long})),
        Ok(OrchCall::SubmitDoc(SubmitDoc { text, .. })) if text.len() == 200_000
    ));
    assert_eq!(
        parse_call(
            AgentRole::Brainstormer,
            "submit_doc",
            &json!({"kind": "brainstorm_draft", "text": "## Understanding"})
        ),
        Ok(OrchCall::SubmitDoc(SubmitDoc {
            kind: DocKind::BrainstormDraft,
            text: "## Understanding".into(),
            ready: false,
            amend: false,
            responses: Vec::new(),
        }))
    );
    for role in [AgentRole::Orchestrator, AgentRole::DocReviewer] {
        assert_eq!(
            parse_call(role, "get_doc", &json!({"kind": "spec"})),
            Ok(OrchCall::GetDoc {
                kind: DocKind::Spec,
                version: None,
                from: None
            })
        );
    }
    assert_eq!(
        orch("get_doc", json!({"kind": "plan", "version": 3})),
        Ok(OrchCall::GetDoc {
            kind: DocKind::Plan,
            version: Some(3),
            from: None
        })
    );
    assert_eq!(
        orch(
            "get_doc",
            json!({"kind": "brainstorm_draft", "from": "codex"})
        ),
        Ok(OrchCall::GetDoc {
            kind: DocKind::BrainstormDraft,
            version: None,
            from: Some("codex".into())
        })
    );
    let finding = |id: &str, severity| DocFinding {
        id: id.into(),
        severity,
        place: "## Requirements".into(),
        text: "R2 has no acceptance check".into(),
    };
    assert_eq!(
        parse_call(
            AgentRole::DocReviewer,
            "submit_findings",
            &json!({"findings": [
                {"id": "F1", "severity": "blocking", "place": "## Requirements",
                 "text": "R2 has no acceptance check"},
                {"id": "F2", "severity": "minor", "place": "## Requirements",
                 "text": "R2 has no acceptance check"},
            ]})
        ),
        Ok(OrchCall::SubmitFindings {
            findings: vec![
                finding("F1", DocSeverity::Blocking),
                finding("F2", DocSeverity::Minor)
            ]
        })
    );
    assert_eq!(
        parse_call(
            AgentRole::DocReviewer,
            "submit_findings",
            &json!({"findings": []})
        ),
        Ok(OrchCall::SubmitFindings {
            findings: Vec::new()
        })
    );
}

/// Every refusal of a design tool's arguments, in the parse's `invalid arguments:
/// <field>: <problem>` wording.
#[test]
fn design_tool_arguments_are_bounded() {
    let cases: Vec<(AgentRole, &str, Value, &str)> = vec![
        (
            AgentRole::Orchestrator,
            "start_brainstorm",
            json!({}),
            "answers: required",
        ),
        (
            AgentRole::Orchestrator,
            "start_brainstorm",
            json!({"answers": 3}),
            "answers: must be a string",
        ),
        (
            AgentRole::Orchestrator,
            "start_brainstorm",
            json!({"answers": "x".repeat(8193)}),
            "answers: at most 8192 bytes",
        ),
        (
            AgentRole::Orchestrator,
            "start_brainstorm",
            json!({"answers": "", "x": 1}),
            "x: unknown field",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"text": "t"}),
            "kind: required",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"kind": "brainstorm_draft", "text": "t"}),
            "kind: must be one of brainstorm, spec",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"kind": "plan", "text": "t"}),
            "kind: must be one of brainstorm, spec",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"kind": "spec"}),
            "text: required",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"kind": "spec", "text": ""}),
            "text: must not be empty",
        ),
        (
            AgentRole::Orchestrator,
            "submit_doc",
            json!({"kind": "spec", "text": "t", "ready": "yes"}),
            "ready: must be a boolean",
        ),
        (
            AgentRole::Brainstormer,
            "submit_doc",
            json!({"kind": "spec", "text": "t"}),
            "kind: must be brainstorm_draft",
        ),
        (
            AgentRole::Brainstormer,
            "submit_doc",
            json!({"kind": "brainstorm_draft", "text": "t", "ready": true}),
            "ready: unknown field",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({}),
            "kind: required",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "report"}),
            "kind: must be one of brainstorm_draft, brainstorm, spec, plan",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "spec", "version": 0}),
            "version: must be a positive integer",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "spec", "version": 4_294_967_296_u64}),
            "version: must be a positive integer",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "spec", "from": "codex"}),
            "from: only with kind brainstorm_draft",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "brainstorm_draft", "from": "../x"}),
            "from: must match ^[A-Za-z0-9_-]{1,32}$",
        ),
        (
            AgentRole::Orchestrator,
            "get_doc",
            json!({"kind": "brainstorm_draft", "from": "codex", "version": 2}),
            "from: not with version",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({}),
            "findings: required",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": (0..41).map(|n| json!({"id": format!("F{n}"), "severity": "minor",
                "place": "p", "text": "t"})).collect::<Vec<_>>()}),
            "findings: at most 40 items",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "F1", "severity": "critical", "place": "p", "text": "t"}]}),
            "findings[0].severity: must be one of blocking, minor",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "F1", "severity": "minor", "text": "t"}]}),
            "findings[0].place: required",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "F1", "severity": "minor", "place": "p", "text": "x".repeat(2001)}]}),
            "findings[0].text: must be 1 to 2000 characters",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "F1", "severity": "minor", "place": "p".repeat(301), "text": "t"}]}),
            "findings[0].place: must be 1 to 300 characters",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "", "severity": "minor", "place": "p", "text": "t"}]}),
            "findings[0].id: must match ^[A-Za-z0-9][A-Za-z0-9-]{0,15}$",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [{"id": "F1", "severity": "minor", "place": "p", "text": "t", "where": "w"}]}),
            "findings[0].where: unknown field",
        ),
        (
            AgentRole::DocReviewer,
            "submit_findings",
            json!({"findings": [
                {"id": "F1", "severity": "minor", "place": "p", "text": "t"},
                {"id": "F1", "severity": "blocking", "place": "p", "text": "t"},
            ]}),
            "findings[1].id: F1 repeats",
        ),
    ];
    for (role, tool, args, problem) in cases {
        assert_eq!(
            parse_call(role, tool, &args),
            Err(format!("invalid arguments: {problem}")),
            "{tool} {args}"
        );
    }
}
