//! Milestone 9.6 task M9.6.6: the design flow's tool calls (Interfaces "MCP tools"),
//! parsed as `tools.rs` parses the others: `start_brainstorm`, `submit_doc` (the
//! orchestrator's `brainstorm` and `spec`, a brainstormer's `brainstorm_draft`),
//! `get_doc`, `submit_findings`, and the `responses` that `submit_doc` and `edit_plan`
//! carry. Which role may call which tool is `parse_call`'s; which phase admits it is the
//! engine's (ruling T1-O4, task M9.6.7). A document's length is not bounded here: the
//! engine's template check caps its raw bytes with its own exact text (decision 15,
//! Review focus 4). Pure.

use std::collections::BTreeSet;

use proto::{AgentRole, DocFinding, DocKind, DocSeverity, FindingAnswer};
use serde_json::{Map, Value};

use super::{OrchCall, array, flag, object, one_text, required};

/// `start_brainstorm`'s `answers`, in bytes; it may be empty.
const ANSWERS_MAX: usize = 8 * 1024;
/// The most findings one review holds, and so the most responses.
const FINDINGS_MAX: usize = 40;
const FINDING_ID: &str = "^[A-Za-z0-9][A-Za-z0-9-]{0,15}$";
const LABEL: &str = "^[A-Za-z0-9_-]{1,32}$";

/// `submit_doc`'s arguments (Interfaces: `design::submit_doc(run, caller, args, now)`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmitDoc {
    pub kind: DocKind,
    pub text: String,
    /// The spec's: `false` sends it to review, `true` opens the gate (decision 15).
    pub ready: bool,
    /// A round's amendment (decision 14).
    pub amend: bool,
    pub responses: Vec<FindingAnswer>,
}

/// One design tool's call from `role`, which `parse_call` has already admitted.
pub(super) fn parse(role: AgentRole, tool: &str, args: &Value) -> Result<OrchCall, String> {
    match tool {
        "start_brainstorm" => {
            let map = object(args, &["answers"])?;
            let answers = match required(map.get("answers"), "answers")? {
                Value::String(s) if s.len() <= ANSWERS_MAX => s.clone(),
                Value::String(_) => return Err(format!("answers: at most {ANSWERS_MAX} bytes")),
                _ => return Err("answers: must be a string".into()),
            };
            Ok(OrchCall::StartBrainstorm { answers })
        }
        "submit_doc" if role == AgentRole::Brainstormer => {
            let map = object(args, &["kind", "text"])?;
            let kind = match kind(map)? {
                DocKind::BrainstormDraft => DocKind::BrainstormDraft,
                _ => return Err("kind: must be brainstorm_draft".into()),
            };
            Ok(OrchCall::SubmitDoc(SubmitDoc {
                kind,
                text: document(map)?,
                ready: false,
                amend: false,
                responses: Vec::new(),
            }))
        }
        "submit_doc" => {
            let map = object(args, &["kind", "text", "ready", "amend", "responses"])?;
            let kind = match kind(map)? {
                kind @ (DocKind::Brainstorm | DocKind::Spec) => kind,
                _ => return Err("kind: must be one of brainstorm, spec".into()),
            };
            Ok(OrchCall::SubmitDoc(SubmitDoc {
                kind,
                text: document(map)?,
                ready: flag(map, "ready")?,
                amend: flag(map, "amend")?,
                responses: responses(map)?,
            }))
        }
        "get_doc" => {
            let map = object(args, &["kind", "version", "from"])?;
            let kind = kind(map)?;
            let version = match map.get("version") {
                None => None,
                Some(v) => Some(
                    (v.as_u64().filter(|n| *n >= 1))
                        .and_then(|n| u32::try_from(n).ok())
                        .ok_or("version: must be a positive integer")?,
                ),
            };
            let from = match map.get("from") {
                None => None,
                Some(_) if kind != DocKind::BrainstormDraft => {
                    return Err("from: only with kind brainstorm_draft".into());
                }
                Some(_) if version.is_some() => return Err("from: not with version".into()),
                Some(Value::String(s)) if label(s) => Some(s.clone()),
                Some(_) => return Err(format!("from: must match {LABEL}")),
            };
            Ok(OrchCall::GetDoc {
                kind,
                version,
                from,
            })
        }
        "submit_findings" => {
            let map = object(args, &["findings"])?;
            let list = array(
                required(map.get("findings"), "findings")?,
                "findings",
                0,
                FINDINGS_MAX,
            )?;
            let mut seen = BTreeSet::new();
            let mut findings = Vec::new();
            for (i, item) in list.iter().enumerate() {
                let at = format!("findings[{i}]");
                let item = entry(item, &at, &["id", "severity", "place", "text"])?;
                let id = finding_id(item, &at)?;
                if !seen.insert(id.clone()) {
                    return Err(format!("{at}.id: {id} repeats"));
                }
                let severity = match item.get("severity").and_then(Value::as_str) {
                    Some("blocking") => DocSeverity::Blocking,
                    Some("minor") => DocSeverity::Minor,
                    None if !item.contains_key("severity") => {
                        return Err(format!("{at}.severity: required"));
                    }
                    _ => return Err(format!("{at}.severity: must be one of blocking, minor")),
                };
                findings.push(DocFinding {
                    id,
                    severity,
                    place: item_text(item, &at, "place", 300)?,
                    text: item_text(item, &at, "text", 2000)?,
                });
            }
            Ok(OrchCall::SubmitFindings { findings })
        }
        other => Err(format!("{other}: unknown tool")),
    }
}

/// `responses`: at most 40 `{id, answer}`, each id once, each answer `"fixed"` or
/// `"kept: <reason>"` (decision 15); empty when absent.
pub(super) fn responses(map: &Map<String, Value>) -> Result<Vec<FindingAnswer>, String> {
    let Some(value) = map.get("responses") else {
        return Ok(Vec::new());
    };
    let mut seen = BTreeSet::new();
    let mut answers = Vec::new();
    for (i, item) in array(value, "responses", 0, FINDINGS_MAX)?
        .iter()
        .enumerate()
    {
        let at = format!("responses[{i}]");
        let item = entry(item, &at, &["id", "answer"])?;
        let id = finding_id(item, &at)?;
        if !seen.insert(id.clone()) {
            return Err(format!("{at}.id: {id} is answered twice"));
        }
        let answer = item_text(item, &at, "answer", 2000)?;
        // Ruling T6-1 (m1): exactly `fixed`, or `kept: ` with its space and a reason.
        let kept = (answer.strip_prefix("kept: ")).is_some_and(|reason| !reason.trim().is_empty());
        if answer != "fixed" && !kept {
            return Err(format!(
                "{at}.answer: must be \"fixed\" or \"kept: <reason>\""
            ));
        }
        answers.push(FindingAnswer { id, answer });
    }
    Ok(answers)
}

fn kind(map: &Map<String, Value>) -> Result<DocKind, String> {
    match required(map.get("kind"), "kind")?.as_str() {
        Some("brainstorm_draft") => Ok(DocKind::BrainstormDraft),
        Some("brainstorm") => Ok(DocKind::Brainstorm),
        Some("spec") => Ok(DocKind::Spec),
        Some("plan") => Ok(DocKind::Plan),
        _ => Err("kind: must be one of brainstorm_draft, brainstorm, spec, plan".into()),
    }
}

/// A document's `text`: required and not empty; its cap is the engine's.
fn document(map: &Map<String, Value>) -> Result<String, String> {
    match required(map.get("text"), "text")? {
        Value::String(s) if s.is_empty() => Err("text: must not be empty".into()),
        Value::String(s) => Ok(s.clone()),
        _ => Err("text: must be a string".into()),
    }
}

fn finding_id(item: &Map<String, Value>, at: &str) -> Result<String, String> {
    let id = required(item.get("id"), &format!("{at}.id"))?;
    let valid = id.as_str().filter(|s| {
        let mut bytes = s.bytes();
        bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
            && s.len() <= 16
            && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'-')
    });
    valid
        .map(str::to_string)
        .ok_or_else(|| format!("{at}.id: must match {FINDING_ID}"))
}

fn item_text(
    item: &Map<String, Value>,
    at: &str,
    field: &str,
    max: usize,
) -> Result<String, String> {
    let path = format!("{at}.{field}");
    one_text(required(item.get(field), &path)?, &path, max)
}

/// One item of a list: an object with only `fields`.
fn entry<'a>(
    value: &'a Value,
    at: &str,
    fields: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    if !value.is_object() {
        return Err(format!("{at}: must be an object"));
    }
    object(value, fields).map_err(|e| format!("{at}.{e}"))
}

/// A brainstorm draft's label as a file name takes it (`state.rs`'s `file_safe`).
fn label(s: &str) -> bool {
    (1..=32).contains(&s.len())
        && (s.bytes()).all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
