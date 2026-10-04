//! Milestone 9.6 decision 11 (DF §3.2): the brainstormers' input pack, the same for
//! both. The driver reads what it holds off the engine (`driver/design_ops.rs`: the
//! stored profile and the run's scout reports, as `get_context` reads them, and a
//! continued goal's previous spec, checked against its index); this module only lays it
//! out, so it is pure like the rest of `run/design/`.
//!
//! **Prompt hygiene.** Every text is cleaned by `safe_text` before it reaches the
//! prompt, and the repository-derived ones (reports, the profile, the earlier spec) sit
//! indented inside their own block, so no line of theirs passes for a section of the
//! prompt. The pack is capped at [`PACK_MAX`]; a cut keeps the head (the goal and the
//! answers come first) and ends `[cut: <n> bytes]`.

use std::path::PathBuf;

use proto::{DocGateKind, DocKind, RunState, ScoutReport, safe_text};

use super::state::design_dir;
use super::template::{lines, section};
use crate::run::contract::floor_boundary;
use crate::run::model::Run;

/// Decision 11's cap.
pub const PACK_MAX: usize = 48 * 1024;
/// Room kept for the cut marker, `\n[cut: <n> bytes]`.
const MARKER_ROOM: usize = 32;
/// The earlier spec's sections the pack carries (decision 11).
const EARLIER_SECTIONS: [&str; 2] = ["## Goal and success criteria", "## Requirements"];

/// What the pack holds, read by the driver.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PackInputs {
    pub goal: String,
    /// The user's answers (`start_brainstorm`).
    pub answers: Option<String>,
    /// The stored repository profile's summary (`profile::summary`).
    pub profile: Option<String>,
    /// The run's scout reports, the onboarding one first.
    pub reports: Vec<ScoutReport>,
    /// A continued goal's previous run's approved spec (decision 30).
    pub earlier: Option<Earlier>,
}

/// The previous run's approved spec: where it is, and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Earlier {
    pub path: PathBuf,
    pub text: String,
}

/// The pack, at most [`PACK_MAX`] bytes.
pub fn pack(inputs: &PackInputs) -> String {
    let full = pack_uncapped(inputs);
    if full.len() <= PACK_MAX {
        return full;
    }
    let kept = floor_boundary(&full, PACK_MAX - MARKER_ROOM);
    format!("{}\n[cut: {} bytes]", &full[..kept], full.len() - kept)
}

fn pack_uncapped(inputs: &PackInputs) -> String {
    let mut blocks = vec![format!("Goal:\n{}", indented(&inputs.goal, 2))];
    let answers = inputs.answers.as_deref().filter(|a| !a.trim().is_empty());
    blocks.push(format!(
        "The user's answers:\n{}",
        indented(answers.unwrap_or("none"), 2)
    ));
    if let Some(earlier) = &inputs.earlier {
        let path = safe_text::one_line(&earlier.path.display().to_string());
        let mut block = format!("Related earlier work: the previous run's approved spec, {path}");
        let text = safe_text::multi_line(&earlier.text);
        let lines = lines(&text);
        for heading in EARLIER_SECTIONS {
            let body = section(&lines, heading).unwrap_or_default();
            let body: Vec<&str> = body.iter().map(|l| l.text).collect();
            block.push_str(&format!("\n  {heading}\n{}", indented(&body.join("\n"), 2)));
        }
        blocks.push(block);
    }
    let profile = inputs.profile.as_deref().unwrap_or("none stored");
    blocks.push(format!("Repository profile:\n{}", indented(profile, 2)));
    for r in &inputs.reports {
        blocks.push(report(r));
    }
    blocks.join("\n\n")
}

/// One scout report: its summary and findings (`files`, `interfaces`, `risks`; the
/// brief's C-19).
fn report(r: &ScoutReport) -> String {
    let id = safe_text::one_line(&r.id);
    let question = safe_text::one_line(&r.question);
    let mut out = format!(
        "Scout report {id} ({question}):\n{}",
        indented(&r.summary, 2)
    );
    let files: Vec<String> = (r.files.iter())
        .map(|f| format!("{}: {}", f.path, f.why))
        .collect();
    for (title, list) in [
        ("Files", &files),
        ("Interfaces", &r.interfaces),
        ("Risks", &r.risks),
    ] {
        if list.is_empty() {
            continue;
        }
        out.push_str(&format!("\n  {title}:"));
        for item in list {
            out.push_str(&format!("\n    {}", safe_text::one_line(item)));
        }
    }
    out
}

/// `text` cleaned (`safe_text::multi_line`), every line indented by `n` spaces.
fn indented(text: &str, n: usize) -> String {
    let pad = " ".repeat(n);
    let text = safe_text::multi_line(text);
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Decision 30: the spec a continued goal's pack carries: the run that `run_id`
/// continued (`continued_by`), its latest spec gate version when that spec was
/// approved, as `(run id, version, path)`.
pub fn previous_spec<'a>(
    runs: impl IntoIterator<Item = &'a Run>,
    run_id: &str,
) -> Option<(String, u32, PathBuf)> {
    let prev = (runs.into_iter()).find(|r| r.continued_by.as_deref() == Some(run_id))?;
    let design = prev.orch.design.as_ref()?;
    let version = design.find(DocKind::Spec, None).filter(|v| v.n > 0)?;
    spec_approved(prev).then(|| {
        let path = design_dir(prev).join(design.file_name(version));
        (prev.id.clone(), version.n, path)
    })
}

/// Whether `run`'s spec was approved: the run is past brainstorming and specifying (not
/// in them, paused or halted there, nor waiting at the brainstorm or spec gate), and
/// was not discarded.
fn spec_approved(run: &Run) -> bool {
    let Some(design) = run.orch.design.as_ref() else {
        return false;
    };
    let doc_phase =
        |s: Option<RunState>| matches!(s, Some(RunState::Brainstorming | RunState::Specifying));
    let at_doc_gate = run.state == RunState::AwaitingApproval
        && (design.gate.as_ref()).is_some_and(|g| g.kind != DocGateKind::Plan);
    let before = doc_phase(Some(run.state))
        || doc_phase(run.paused_from)
        || doc_phase(design.halted_from)
        || at_doc_gate;
    !before && run.state != RunState::Discarded
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
