//! Claude's model list: the stream-json `initialize` control request (MR §4.1). One
//! line is written, never a user message, so no turn runs and nothing is billed.

use super::lines::{LineReader, read_deadline, spawn, wait_for_exit};
use super::{EXIT_GRACE, ProbeError, REFUSAL_MAX_CHARS, catalog_id_ok, probe_command, write_line};
use proto::{CatalogModel, Runtime};
use serde_json::Value;
use std::time::Instant;
use tokio_util::sync::CancellationToken;

const REQUEST_ID: &str = "anthrex-models-1";
const INITIALIZE: &str = r#"{"type":"control_request","request_id":"anthrex-models-1","request":{"subtype":"initialize"}}"#;

/// The models Claude's `initialize` reply lists, and every line read.
pub fn probe(
    program: &str,
    deadline: Instant,
    cancel: &CancellationToken,
    env_remove: &[&str],
) -> Result<(Vec<CatalogModel>, Vec<String>), ProbeError> {
    let args = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
    ];
    let mut command = probe_command(program, &args, Runtime::Claude, env_remove);
    let mut owned = spawn(&mut command, program, deadline, cancel)?;
    let mut stdin = owned.child.stdin.take().expect("stdin was piped");
    let stdout = owned.child.stdout.take().expect("stdout was piped");
    let mut reader = LineReader::new(stdout)
        .map_err(|e| ProbeError::Failed(format!("could not read claude: {e}")))?;
    write_line(&mut stdin, INITIALIZE, "claude")?;
    let until = read_deadline(deadline);
    let models = loop {
        let Some(line) = reader.next(until, cancel)? else {
            return Err(ProbeError::Failed(
                "claude ended without answering initialize".into(),
            ));
        };
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let response = &value["response"];
        if value["type"] != "control_response" || response["request_id"] != REQUEST_ID {
            continue;
        }
        if response["subtype"] == "error" {
            let message = response["error"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| response["error"].to_string());
            let cut: String = message.chars().take(REFUSAL_MAX_CHARS).collect();
            return Err(ProbeError::Failed(format!(
                "claude refused initialize: {cut}"
            )));
        }
        break parse_models(&response["response"]["models"]);
    };
    // Closing stdin ends the session; a CLI still running after the grace is killed
    // and reaped by `ProbeChild`.
    drop(stdin);
    wait_for_exit(&mut owned, until.min(Instant::now() + EXIT_GRACE));
    if models.is_empty() {
        return Err(ProbeError::Failed("claude's reply had no models".into()));
    }
    Ok((models, reader.into_lines()))
}

fn parse_models(models: &Value) -> Vec<CatalogModel> {
    let text = |v: &Value| v.as_str().map(str::to_string);
    models
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let id = text(&m["value"])?;
            if !catalog_id_ok(Runtime::Claude, &id) {
                return None;
            }
            let efforts = if m["supportsEffort"].as_bool() == Some(true) {
                m["supportedEffortLevels"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(text)
                    .collect()
            } else {
                Vec::new()
            };
            Some(CatalogModel {
                label: text(&m["displayName"]).unwrap_or_else(|| id.clone()),
                description: text(&m["description"]).unwrap_or_default(),
                efforts,
                default_effort: None,
                is_default: id == "default",
                id,
            })
        })
        .collect()
}
