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

/// Decision 34, the engine's half: the reports a first turn's extract is built from,
/// and where it goes. The engine builds the turn with no extract (it reads no file); the
/// driver reads the reports when it launches the session and puts
/// `sep` + [`scout_extract`] at byte `at` ([`ExtractSlot::fill`]), which gives exactly
/// the prompt built with that extract.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExtractSlot {
    /// `scout_refs` entries, in order: a run scout's full id, or `onboarding`.
    pub refs: Vec<String>,
    /// The report `onboarding` names (`Run.onboarding_report`).
    pub onboarding: Option<String>,
    pub at: usize,
    pub sep: String,
}

impl ExtractSlot {
    /// A slot for `refs`, none when there are no refs (the turn needs no extract).
    pub fn new(refs: &[String], onboarding: Option<&str>, at: usize, sep: &str) -> Option<Self> {
        (!refs.is_empty()).then(|| ExtractSlot {
            refs: refs.to_vec(),
            onboarding: onboarding.map(str::to_string),
            at,
            sep: sep.to_string(),
        })
    }

    /// `first_turn` with `extract` in its place; unchanged when `extract` is empty or
    /// the offset is not a character boundary of it.
    pub fn fill(&self, first_turn: &str, extract: &str) -> String {
        if extract.is_empty() || !first_turn.is_char_boundary(self.at) {
            return first_turn.to_string();
        }
        let (head, tail) = first_turn.split_at(self.at);
        format!("{head}{}{extract}{tail}", self.sep)
    }
}

/// Decision 34: a worker's (or a handover's) slot for its task's `scout_refs`, after
/// its acceptance criteria.
pub fn worker_slot(
    run: &crate::run::model::Run,
    task: &crate::run::model::Task,
) -> Option<ExtractSlot> {
    let at = crate::run::contract::worker_extract_at(run, task);
    let onboarding = run.onboarding_report.as_deref();
    ExtractSlot::new(&task.spec.scout_refs, onboarding, at, "\n\n")
}

#[cfg(test)]
#[path = "extract_tests.rs"]
mod tests;
