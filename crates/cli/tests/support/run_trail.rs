//! What a run left behind, for a failure message: CI keeps nothing else once the
//! harness's directory is gone. Each task's whole history (the snapshot shows its last
//! ten events), what every `fake-agent` said on stderr, its MCP calls, and the state of
//! every script claim.

use std::path::Path;

use serde_json::Value;

use super::run_harness::RunHarness;

/// The most lines of one task's history, of the error log and of the MCP log shown.
const TRAIL_LINES: usize = 60;

impl RunHarness {
    /// `FAKE_AGENT_ERROR_LOG`: every line a `fake-agent` wrote to stderr.
    pub fn error_log(&self) -> std::path::PathBuf {
        self.io.join("fake-agent.errors")
    }

    /// The trail of every run of this harness: see the module doc.
    pub fn trail(&self) -> String {
        let mut out = String::new();
        let runs = std::fs::read_dir(self.data().join("runs"))
            .into_iter()
            .flatten();
        for run in runs.flatten() {
            let text = std::fs::read_to_string(run.path().join("run.json")).unwrap_or_default();
            let Ok(state) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for task in state["tasks"].as_array().into_iter().flatten() {
                let history: Vec<String> = (task["history"].as_array().into_iter().flatten())
                    .map(|e| format!("  {} {}", e["at"], e["text"].as_str().unwrap_or("")))
                    .collect();
                out.push_str(&format!(
                    "--- {} task {} history ({} events):\n",
                    run.file_name().to_string_lossy(),
                    task["spec"]["id"].as_str().unwrap_or("?"),
                    history.len()
                ));
                out.push_str(&head(&history.join("\n")));
            }
        }
        for (what, path) in [
            ("fake-agent stderr", self.error_log()),
            ("MCP calls", self.io.join("mcp.jsonl")),
        ] {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            out.push_str(&format!("--- {what}:\n{}", head(&text)));
        }
        out.push_str("--- script claims:\n");
        out.push_str(&claims(&self.repo.join(".git").join("fake-agent")));
        out
    }
}

/// The first [`TRAIL_LINES`] lines of `text`, newline-ended, and how many were left out.
fn head(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: String = (lines.iter().take(TRAIL_LINES))
        .map(|l| format!("{l}\n"))
        .collect();
    if lines.len() > TRAIL_LINES {
        out.push_str(&format!("  ... {} more\n", lines.len() - TRAIL_LINES));
    }
    out
}

/// Every script in `dir`, with its claim's session, position and variables.
fn claims(dir: &Path) -> String {
    let mut names: Vec<String> = (std::fs::read_dir(dir).into_iter().flatten().flatten())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".jsonl"))
        .collect();
    names.sort();
    names
        .iter()
        .map(|name| {
            let read = |suffix: &str| {
                std::fs::read_to_string(dir.join(format!("{name}{suffix}")))
                    .map(|t| t.trim().to_string())
                    .unwrap_or_else(|_| "-".into())
            };
            format!(
                "  {name}: claimed {} pos {} vars {}\n",
                read(".claimed"),
                read(".pos"),
                read(".vars")
            )
        })
        .collect()
}
