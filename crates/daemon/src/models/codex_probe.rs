//! Codex's model list: `codex app-server`'s JSON-RPC `initialize`, then `model/list`
//! page by page (MR §4.1). The requests are a few hundred bytes each, far under a
//! pipe's buffer, so writing them from this blocking thread cannot stall it.

use super::lines::{LineReader, read_deadline, spawn, wait_for_exit};
use super::{EXIT_GRACE, ProbeError, REFUSAL_MAX_CHARS, catalog_id_ok, probe_command, write_line};
use proto::{CatalogModel, Runtime};
use serde_json::{Value, json};
use std::time::Instant;
use tokio_util::sync::CancellationToken;

/// At most this many `model/list` pages are read.
const MAX_PAGES: u64 = 20;

/// The models Codex's `model/list` pages list, and every line read.
pub fn probe(
    program: &str,
    deadline: Instant,
    cancel: &CancellationToken,
    env_remove: &[&str],
) -> Result<(Vec<CatalogModel>, Vec<String>), ProbeError> {
    let mut command = probe_command(program, &["app-server"], Runtime::Codex, env_remove);
    let mut owned = spawn(&mut command, program, deadline, cancel)?;
    let mut stdin = owned.child.stdin.take().expect("stdin was piped");
    let stdout = owned.child.stdout.take().expect("stdout was piped");
    let mut reader = LineReader::new(stdout)
        .map_err(|e| ProbeError::Failed(format!("could not read codex: {e}")))?;
    let until = read_deadline(deadline);
    let initialize = json!({"id": 1, "method": "initialize",
        "params": {"clientInfo": {"name": "anthrex", "version": env!("CARGO_PKG_VERSION")}}});
    write_line(&mut stdin, &initialize.to_string(), "codex")?;
    answer(&mut reader, 1, "initialize", until, cancel)?;
    write_line(&mut stdin, r#"{"method":"initialized"}"#, "codex")?;
    let mut models = Vec::new();
    let mut cursor = Value::Null;
    for id in 2..2 + MAX_PAGES {
        let request = json!({"id": id, "method": "model/list",
            "params": {"includeHidden": false, "cursor": cursor}});
        write_line(&mut stdin, &request.to_string(), "codex")?;
        let page = answer(&mut reader, id, "model/list", until, cancel)?;
        models.extend(parse_models(&page["data"]));
        match page.get("nextCursor") {
            Some(next) if !next.is_null() => cursor = next.clone(),
            _ => break,
        }
    }
    drop(stdin);
    wait_for_exit(&mut owned, until.min(Instant::now() + EXIT_GRACE));
    if models.is_empty() {
        return Err(ProbeError::Failed("codex's reply had no models".into()));
    }
    Ok((models, reader.into_lines()))
}

/// The `result` of the reply with `id`; other lines (notifications, non-JSON) are skipped.
fn answer(
    reader: &mut LineReader,
    id: u64,
    method: &str,
    until: Instant,
    cancel: &CancellationToken,
) -> Result<Value, ProbeError> {
    loop {
        let Some(line) = reader.next(until, cancel)? else {
            return Err(ProbeError::Failed(format!(
                "codex ended without answering {method}"
            )));
        };
        let Ok(mut value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if value["id"].as_u64() != Some(id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            let message = error["message"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| error.to_string());
            let cut: String = message.chars().take(REFUSAL_MAX_CHARS).collect();
            return Err(ProbeError::Failed(format!("codex refused {method}: {cut}")));
        }
        return Ok(value["result"].take());
    }
}

pub(crate) fn parse_models(data: &Value) -> Vec<CatalogModel> {
    let text = |v: &Value| v.as_str().map(str::to_string);
    data.as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = text(&m["id"]).or_else(|| text(&m["model"]))?;
            if !catalog_id_ok(Runtime::Codex, &id) {
                return None;
            }
            let efforts = m["supportedReasoningEfforts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| text(e).or_else(|| text(&e["reasoningEffort"])))
                .collect();
            Some(CatalogModel {
                resolved: None,
                label: text(&m["displayName"]).unwrap_or_else(|| id.clone()),
                description: text(&m["description"]).unwrap_or_default(),
                efforts,
                default_effort: text(&m["defaultReasoningEffort"]),
                is_default: m["isDefault"].as_bool() == Some(true),
                id,
            })
        })
        .collect()
}
