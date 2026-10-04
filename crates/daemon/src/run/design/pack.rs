//! Milestone 9.6 decision 11 (DF §3.2): the brainstormers' input pack, the same for
//! both. Its inputs are frozen when the brainstormers are queued ([`FrozenPack`],
//! ruling T8-2): the run's scout report ids and a continued goal's previous spec. The
//! driver reads them off the engine (`driver/design_ops.rs`: the stored profile and
//! those reports, as `get_context` reads them, and the previous spec, checked against
//! its frozen index entry); this module only lays it out, so it is pure like the rest
//! of `run/design/`.
//!
//! **Prompt hygiene.** Every text is cleaned by `safe_text` before it reaches the
//! prompt, and the repository-derived ones (reports, the profile, the earlier spec) sit
//! indented inside their own block, so no line of theirs passes for a section of the
//! prompt. The pack is capped at [`PACK_MAX`]; a cut keeps the head (the goal and the
//! answers come first) and ends `[cut: <n> bytes]`.

use std::path::PathBuf;

use proto::{DocGateKind, DocKind, RunState, ScoutReport, safe_text};
use serde::{Deserialize, Serialize};

use super::report;
use super::state::{DocVersion, design_dir};
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
    /// A rethink's note and the report it replaces (decision 7, task M9.6.9).
    pub rethink: Option<RethinkInput>,
}

/// What a rethink adds to its round's pack: the user's note and the previous merged
/// report (its text as stored, `None` when it could not be read back).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RethinkInput {
    pub version: u32,
    pub note: String,
    pub report: Option<String>,
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
    if let Some(rethink) = &inputs.rethink {
        blocks.push(rethink_block(rethink));
    }
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

/// A rethink's block: the user's note, then the previous report without the engine's
/// appendix of drafts (task M9.6.9).
fn rethink_block(rethink: &RethinkInput) -> String {
    let n = rethink.version;
    let note = indented(&rethink.note, 2);
    let mut block = format!("The user asked to rethink the brainstorm:\n{note}");
    match &rethink.report {
        Some(text) => {
            let report = indented(report::split(text).0.trim_end(), 2);
            block.push_str(&format!("\nThe previous merged report, v{n}:\n{report}"));
        }
        None => block.push_str(&format!(
            "\nThe previous merged report, v{n}, could not be read."
        )),
    }
    block
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

/// Ruling T8-2: what the pack reads, frozen when the brainstormers are queued, so both
/// starts, and any relaunch, read exactly the same (`DesignState::pack`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrozenPack {
    /// The run's scout reports then (`Run::scout_reports`), in order.
    pub reports: Vec<String>,
    /// A continued goal's previous approved spec then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earlier: Option<EarlierSpec>,
    /// Ruling T8-6: the brainstorm round these inputs are for (a rethink is a new one),
    /// whose pack is written once, to [`pack_path`].
    pub round: u32,
    /// Ruling T8-6: the round's pack file as its first start wrote it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<PackFile>,
    /// Task M9.6.9 (decision 7): a rethink's round also carries the user's note and the
    /// report it replaces.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rethink: Option<FrozenRethink>,
}

/// A rethink's frozen inputs: the user's note, and the previous merged report's file and
/// index entry, against which the driver reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenRethink {
    pub note: String,
    pub path: PathBuf,
    pub version: DocVersion,
}

/// Ruling T8-6: a written pack's length and SHA-256, against which every later start of
/// its round reads it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFile {
    pub bytes: u64,
    pub sha256: String,
}

/// Ruling T8-6: round `round`'s pack file, `design/brainstorm/pack-r<round>.md`.
pub fn pack_path(run: &Run, round: u32) -> PathBuf {
    design_dir(run).join(format!("brainstorm/pack-r{round}.md"))
}

/// A continued goal's previous approved spec: its run, its file, and its index entry
/// (its length and SHA-256), against which the file is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EarlierSpec {
    pub run: String,
    pub path: PathBuf,
    pub version: DocVersion,
}

/// Ruling T8-2: `run`'s pack inputs now, with `earlier` ([`previous_spec`]).
pub fn freeze(run: &Run, earlier: Option<EarlierSpec>) -> FrozenPack {
    FrozenPack {
        reports: run.scout_reports.clone(),
        earlier,
        round: (run.orch.design.as_ref()).map_or(1, |d| d.rethinks + 1),
        file: None,
        rethink: None,
    }
}

/// Decision 30: the spec a continued goal's pack carries: the run that `run_id`
/// continued (`continued_by`), its latest spec gate version when that spec was
/// approved.
pub fn previous_spec<'a>(
    runs: impl IntoIterator<Item = &'a Run>,
    run_id: &str,
) -> Option<EarlierSpec> {
    let prev = (runs.into_iter()).find(|r| r.continued_by.as_deref() == Some(run_id))?;
    let design = prev.orch.design.as_ref()?;
    let version = design.find(DocKind::Spec, None).filter(|v| v.n > 0)?;
    spec_approved(prev).then(|| EarlierSpec {
        run: prev.id.clone(),
        path: design_dir(prev).join(design.file_name(version)),
        version: version.clone(),
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
