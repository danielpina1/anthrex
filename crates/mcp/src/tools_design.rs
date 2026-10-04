//! Milestone 9.6's tools (Interfaces "MCP tools", DF §9): the orchestrator's
//! `start_brainstorm`, `submit_doc` and `get_doc`, a brainstormer's `submit_doc`, and a
//! document reviewer's `get_doc` and `submit_findings`. Every schema is a closed object
//! at every level and free of `oneOf`, `anyOf` and `allOf`. No tool here approves,
//! changes, rethinks, goes back or rejects: the gates are the user's. The daemon parses
//! every call again (`daemon::run::orch::tools::parse_call`), and the engine checks
//! each document's caps and template.

use rmcp::model::Tool;
use serde_json::{Value, json};

use crate::tools::{closed, one_of, text};

pub const START_BRAINSTORM: &str = "start_brainstorm";
pub const SUBMIT_DOC: &str = "submit_doc";
pub const GET_DOC: &str = "get_doc";
pub const SUBMIT_FINDINGS: &str = "submit_findings";

/// `start_brainstorm`'s `answers`, in bytes (the schema's characters bound it as well).
pub const ANSWERS_MAX: u64 = 8 * 1024;
/// The largest document the orchestrator submits: the spec's 64 KiB cap.
pub const DOC_MAX: u64 = 64 * 1024;
/// A brainstorm draft's cap, 12 KiB (DF §3.3).
pub const DRAFT_MAX: u64 = 12 * 1024;
/// The most findings one review holds, and so the most responses.
pub const FINDINGS_MAX: u64 = 40;
/// A finding's id: the reviewer's, answered by the same id.
pub const FINDING_ID_PATTERN: &str = "^[A-Za-z0-9][A-Za-z0-9-]{0,15}$";
/// A brainstorm draft's label (`get_doc`'s `from`): `claude`, `codex`, `A`, `B`.
pub const LABEL_PATTERN: &str = "^[A-Za-z0-9_-]{1,32}$";

/// The orchestrator's three, after its milestone 9.3 seven.
pub fn orchestrator_design_tools() -> Vec<Tool> {
    vec![start_brainstorm(), submit_doc(), get_doc()]
}

/// `tools_for(Brainstormer)`: its draft, nothing else (it never reads another draft).
pub fn brainstormer_tools() -> Vec<Tool> {
    vec![submit_draft()]
}

/// `tools_for(DocReviewer)`: read the document (its review draft by `draft`, ruling
/// T5-1), then submit the findings once.
pub fn doc_reviewer_tools() -> Vec<Tool> {
    vec![reviewers_get_doc(), submit_findings()]
}

/// `[{id, answer}]`: the orchestrator's answer to each finding of the latest review,
/// `"fixed"` or `"kept: <reason>"` (`submit_doc` and `edit_plan`).
pub(crate) fn responses() -> Value {
    let response = closed(
        json!({"id": finding_id(), "answer": text(2000)}),
        &["id", "answer"],
    );
    json!({"type": "array", "maxItems": FINDINGS_MAX, "items": response})
}

/// `plan_task.covers` (decision 17): at most 32 requirement ids such as `R4`.
pub(crate) fn covers() -> Value {
    let id = json!({"type": "string", "pattern": "^R[0-9]+$", "maxLength": 8});
    json!({"type": "array", "maxItems": 32, "items": id})
}

fn start_brainstorm() -> Tool {
    Tool::new(
        START_BRAINSTORM,
        "Start the two brainstormers with the user's answers to your questions (empty if \
         they skipped). Only in the brainstorming phase. Returns at once.",
        closed(
            json!({"answers": {"type": "string", "minLength": 0, "maxLength": ANSWERS_MAX}}),
            &["answers"],
        ),
    )
}

fn submit_doc() -> Tool {
    Tool::new(
        SUBMIT_DOC,
        "Submit a design document: the merged brainstorm report (kind brainstorm) or the \
         spec (kind spec). A spec with ready false goes to review; with ready true it opens \
         the gate, and responses must answer every finding of the latest review.",
        closed(
            json!({
                "kind": one_of(&["brainstorm", "spec"]),
                "text": text(DOC_MAX),
                "ready": {"type": "boolean"},
                "amend": {"type": "boolean"},
                "responses": responses(),
            }),
            &["kind", "text"],
        ),
    )
}

fn submit_draft() -> Tool {
    Tool::new(
        SUBMIT_DOC,
        "Submit your brainstorm draft (kind brainstorm_draft) in the six-section template. \
         If it is refused, fix what the refusal names and submit again.",
        closed(
            json!({"kind": one_of(&["brainstorm_draft"]), "text": text(DRAFT_MAX)}),
            &["kind", "text"],
        ),
    )
}

fn get_doc() -> Tool {
    Tool::new(
        GET_DOC,
        "Read a design document: the latest of a kind, one version, or a brainstormer's \
         draft (from).",
        closed(
            json!({
                "kind": one_of(&["brainstorm_draft", "brainstorm", "spec", "plan"]),
                "version": {"type": "integer", "minimum": 1},
                "from": {"type": "string", "pattern": LABEL_PATTERN},
            }),
            &["kind"],
        ),
    )
}

/// The document reviewer's `get_doc` (task M9.6.10): also a spec's review draft by its
/// review's number, `draft`.
fn reviewers_get_doc() -> Tool {
    let mut schema = get_doc().input_schema.as_ref().clone();
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.insert("draft".into(), json!({"type": "integer", "minimum": 1}));
    }
    Tool::new(
        GET_DOC,
        "Read a design document: the latest of a kind, one version, or the spec's review \
         draft your first message names (draft).",
        schema,
    )
}

fn submit_findings() -> Tool {
    let finding = closed(
        json!({
            "id": finding_id(),
            "severity": one_of(&["blocking", "minor"]),
            "place": text(300),
            "text": text(2000),
        }),
        &["id", "severity", "place", "text"],
    );
    Tool::new(
        SUBMIT_FINDINGS,
        "Submit your review's findings once, then stop. Use blocking only for placeholders, \
         contradictions, untestable requirements, scope beyond the approved approach, dropped \
         brainstorm decisions, a requirement without an acceptance check, or a plan task that \
         does not deliver its covers.",
        closed(
            json!({"findings": {"type": "array", "maxItems": FINDINGS_MAX, "items": finding}}),
            &["findings"],
        ),
    )
}

fn finding_id() -> Value {
    json!({"type": "string", "pattern": FINDING_ID_PATTERN})
}

#[cfg(test)]
#[path = "tools_design_tests.rs"]
mod tests;
