//! The worker tools' and the reviewer's arguments, checked against the MCP section's
//! schemas (Interfaces, "MCP tools"). The MCP server checks them too (M8a.19); the engine does not trust a
//! caller to have. Problems read `<field>: <problem>`, and the caller prefixes
//! `invalid arguments: `. Pure (design decision 2).

use proto::{Finding, Severity, Verdict};
use serde_json::{Map, Value};

/// `task_done { summary, test?, red? }`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct DoneArgs {
    pub summary: String,
    pub test: Option<String>,
    pub red: Option<String>,
}

fn object<'a>(args: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let Some(map) = args.as_object() else {
        return Err("arguments: must be an object".into());
    };
    if let Some(key) = map.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(format!("{key}: unknown field"));
    }
    Ok(map)
}

/// A string field of 1 to `max` characters; `None` when absent and not `required`.
fn text(
    map: &Map<String, Value>,
    field: &str,
    max: usize,
    required: bool,
) -> Result<Option<String>, String> {
    match map.get(field) {
        None if required => Err(format!("{field}: required")),
        None => Ok(None),
        Some(Value::String(s)) if (1..=max).contains(&s.chars().count()) => Ok(Some(s.clone())),
        Some(Value::String(_)) => Err(format!("{field}: must be 1 to {max} characters")),
        Some(_) => Err(format!("{field}: must be a string")),
    }
}

/// `^[0-9a-f]{7,40}$`.
fn is_sha(s: &str) -> bool {
    (7..=40).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn parse_done(args: &Value) -> Result<DoneArgs, String> {
    let map = object(args, &["summary", "test", "red"])?;
    let summary = text(map, "summary", 4000, true)?.unwrap_or_default();
    let test = text(map, "test", 300, false)?;
    let red = match map.get("red") {
        None => None,
        Some(Value::String(s)) if is_sha(s) => Some(s.clone()),
        Some(_) => return Err("red: must be 7 to 40 lowercase hex digits".into()),
    };
    Ok(DoneArgs { summary, test, red })
}

/// `task_blocked { kind?, reason }`: the kind (default `question`) and the reason.
pub(super) fn parse_blocked(args: &Value) -> Result<(&'static str, String), String> {
    let map = object(args, &["kind", "reason"])?;
    let kind = match map.get("kind") {
        None => "question",
        Some(Value::String(s)) if s == "question" => "question",
        Some(Value::String(s)) if s == "mis_sized" => "mis_sized",
        Some(Value::String(s)) if s == "environment" => "environment",
        Some(_) => return Err("kind: must be one of question, mis_sized, environment".into()),
    };
    let reason = text(map, "reason", 4000, true)?.unwrap_or_default();
    Ok((kind, reason))
}

/// `submit_review { verdict, summary, findings }` (decision 35): the verdict, the
/// summary and the findings, each checked against the MCP schema. A critical or
/// important finding must carry `file` and `line`, or `input`.
pub(super) fn parse_review(args: &Value) -> Result<(Verdict, String, Vec<Finding>), String> {
    let map = object(args, &["verdict", "summary", "findings"])?;
    let verdict = match map.get("verdict") {
        None => return Err("verdict: required".into()),
        Some(Value::String(s)) if s == "approve" => Verdict::Approve,
        Some(Value::String(s)) if s == "changes" => Verdict::Changes,
        Some(_) => return Err("verdict: must be one of approve, changes".into()),
    };
    let summary = text(map, "summary", 4000, true)?.unwrap_or_default();
    let items = match map.get("findings") {
        None => return Err("findings: required".into()),
        Some(Value::Array(items)) if items.len() <= 50 => items,
        Some(Value::Array(_)) => return Err("findings: at most 50".into()),
        Some(_) => return Err("findings: must be an array".into()),
    };
    let findings = items
        .iter()
        .enumerate()
        .map(|(i, item)| parse_finding(item).map_err(|e| format!("findings[{i}]{e}")))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((verdict, summary, findings))
}

/// One finding; a problem reads `.<field>: <problem>`, or `: <problem>` for the whole.
fn parse_finding(item: &Value) -> Result<Finding, String> {
    let fields = ["severity", "file", "line", "input", "text"];
    let map = object(item, &fields).map_err(|e| format!(".{e}"))?;
    let severity = match map.get("severity") {
        None => return Err(".severity: required".into()),
        Some(Value::String(s)) if s == "critical" => Severity::Critical,
        Some(Value::String(s)) if s == "important" => Severity::Important,
        Some(Value::String(s)) if s == "minor" => Severity::Minor,
        Some(_) => return Err(".severity: must be one of critical, important, minor".into()),
    };
    let file = text(map, "file", 500, false).map_err(|e| format!(".{e}"))?;
    let line = match map.get("line") {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 && n <= u64::from(u32::MAX) => Some(n as u32),
            _ => return Err(".line: must be an integer of at least 1".into()),
        },
    };
    let input = text(map, "input", 2000, false).map_err(|e| format!(".{e}"))?;
    let text = text(map, "text", 2000, true)
        .map_err(|e| format!(".{e}"))?
        .unwrap_or_default();
    let located = (file.is_some() && line.is_some()) || input.is_some();
    if severity != Severity::Minor && !located {
        return Err(": a critical or important finding needs file and line, or input".into());
    }
    Ok(Finding {
        severity,
        file,
        line,
        input,
        text,
    })
}
