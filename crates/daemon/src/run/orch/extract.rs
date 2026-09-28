//! Decision 34's scout extract: the scout reports a planner's and a worker's first
//! prompts carry. Pure. A report is repository-derived, untrusted text, so the extract
//! keeps it inside its own section: every summary line is indented, and a file path is
//! kept on one line.

use proto::ScoutReport;

use crate::run::contract::floor_boundary;

/// Decision 34: the scout extract's cap, and a report summary's.
pub const EXTRACT_MAX_BYTES: usize = 12 * 1024;
pub const EXTRACT_SUMMARY_CHARS: usize = 4000;

/// The line [`scout_extract`] ends with when it cut the later reports.
pub const EXTRACT_CUT_MARKER: &str = "\n[anthrex] The later scout reports were cut here to fit.";

/// Decision 34: the reports the driver could read, in `scout_refs` order; one that could
/// not be read is left out with a warning.
pub fn readable_reports(
    read: Vec<(String, Result<ScoutReport, String>)>,
) -> Vec<(String, ScoutReport)> {
    read.into_iter()
        .filter_map(|(id, report)| match report {
            Ok(report) => Some((id, report)),
            Err(error) => {
                tracing::warn!(%id, %error, "a scout report named in scout_refs is left out");
                None
            }
        })
        .collect()
}

/// Decision 34: `Scout report <id>:`, the summary cut to 4000 characters and
/// `Files: <paths>` per report. Every summary line is indented two spaces, so no line of
/// a report starts at column 0 where it could pass for a section of the prompt, and
/// each `\n` or `\r` of a path becomes a space; at most [`EXTRACT_MAX_BYTES`] in all, the later reports
/// cut first, ending with [`EXTRACT_CUT_MARKER`] when anything was cut. Empty for no
/// reports.
pub fn scout_extract(reports: &[(String, ScoutReport)]) -> String {
    let blocks: Vec<String> = reports
        .iter()
        .map(|(id, report)| {
            let summary: String = report.summary.chars().take(EXTRACT_SUMMARY_CHARS).collect();
            let summary = indent(&summary);
            let files: Vec<String> = report
                .files
                .iter()
                .map(|f| f.path.replace(['\n', '\r'], " "))
                .collect();
            let files = if files.is_empty() {
                "none".to_string()
            } else {
                files.join(", ")
            };
            format!("Scout report {id}:\n{summary}\nFiles: {files}")
        })
        .collect();
    let text = blocks.join("\n");
    if text.len() <= EXTRACT_MAX_BYTES {
        return text;
    }
    let cut = floor_boundary(&text, EXTRACT_MAX_BYTES - EXTRACT_CUT_MARKER.len());
    format!("{}{EXTRACT_CUT_MARKER}", &text[..cut])
}

/// `text` with two spaces before every line; `\r\n` and a lone `\r` end a line too.
fn indent(text: &str) -> String {
    text.replace("\r\n", "\n")
        .split(['\n', '\r'])
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
