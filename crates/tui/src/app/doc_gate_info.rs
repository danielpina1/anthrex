//! The document gate's texts and lookups, read off a run's snapshot (milestone 9.6,
//! DF §6): which gate waits for the user, its Alerts row, the status bar's mark, the
//! confirm pages, and the round's drafts behind a brainstorm version. Pure.

use crate::actions_request::short_id;
use crate::app::App;
use crate::app::screens::Screen;
use crate::safe_text::one_line;
use proto::{DocAuthor, DocGateAction, DocGateInfo, DocGateKind, DocKind, RunInfo, RunState};

/// The document a gate of `kind` shows.
pub fn gate_doc(kind: DocGateKind) -> DocKind {
    match kind {
        DocGateKind::Brainstorm => DocKind::Brainstorm,
        DocGateKind::Spec => DocKind::Spec,
        DocGateKind::Plan => DocKind::Plan,
    }
}

/// `Brainstorm`, `Spec` or `Plan`: the header's first word.
pub fn kind_title(kind: DocGateKind) -> &'static str {
    match kind {
        DocGateKind::Brainstorm => "Brainstorm",
        DocGateKind::Spec => "Spec",
        DocGateKind::Plan => "Plan",
    }
}

/// The run's gate while it waits for the user: awaiting approval, and not revising.
pub fn open_gate(run: &RunInfo) -> Option<&DocGateInfo> {
    let gate = run.doc_gate.as_ref()?;
    (run.state == RunState::AwaitingApproval && gate.revising.is_none()).then_some(gate)
}

/// The Alerts row of a design gate waiting for the user (exact): `<kind> v<n> ready for
/// review · run <id4>`, from round 2 on `round <k> <kind> v<n> ready for review · run
/// <id4>`.
pub fn alert_text(run: &RunInfo) -> Option<String> {
    let gate = open_gate(run)?;
    let id = short_id(&one_line(&run.run_id)).to_owned();
    let what = format!(
        "{} v{} ready for review · run {id}",
        gate.kind.label(),
        gate.version
    );
    Some(match run.round {
        0 | 1 => what,
        k => format!("round {k} {what}"),
    })
}

/// The status bar's `⏸ <kind> v<n>` (DF §6.2): the gate waiting for the user on the
/// run in view (the gate screen's, the run view's), else on the first shown run whose
/// gate waits.
pub fn waiting_gate(app: &App) -> Option<(DocGateKind, u32)> {
    let in_view = match &app.screen {
        Some(Screen::DocGate(s)) => Some(s.run_id.as_str()),
        _ => app.run_view.as_ref().map(|v| v.run_id.as_str()),
    };
    let first = |run: &RunInfo| open_gate(run).map(|g| (g.kind, g.version));
    let runs = &app.runs.runs;
    in_view
        .and_then(|id| runs.iter().find(|r| r.run_id == id))
        .and_then(first)
        .or_else(|| crate::tree::shown_runs(runs).find_map(first))
}

/// The run's brainstorm or spec gate, open or revising: what the screen is for.
pub fn doc_gate_of(run: &RunInfo) -> Option<&DocGateInfo> {
    let gate = run.doc_gate.as_ref()?;
    (run.state == RunState::AwaitingApproval && gate.kind != DocGateKind::Plan).then_some(gate)
}

/// The confirm pages' texts (exact, brief "TUI texts").
pub fn confirm_text(
    run_id: &str,
    kind: DocGateKind,
    version: u32,
    action: &DocGateAction,
) -> String {
    let word = kind.label();
    match action {
        DocGateAction::Approve { .. } => {
            let next = match kind {
                DocGateKind::Brainstorm => "Spec",
                DocGateKind::Spec => "Planning",
                DocGateKind::Plan => "The run",
            };
            format!("Approve {word} v{version}? {next} starts next.")
        }
        DocGateAction::Reject => format!(
            "Reject the {word}? This discards run {}.",
            short_id(&one_line(run_id))
        ),
        DocGateAction::Rethink { .. } => {
            "Rethink the brainstorm? Both brainstormers run again.".to_owned()
        }
        DocGateAction::Back { .. } => {
            let prev = match kind {
                DocGateKind::Plan => "spec",
                _ => "brainstorm",
            };
            format!("Go back to the {prev}? The {word} is set aside.")
        }
        // Changes and edit have no page: their editor's save is their confirmation.
        DocGateAction::Changes { .. } | DocGateAction::Edit { .. } => String::new(),
    }
}

/// The round's drafts behind brainstorm gate version `version`: the brainstormers'
/// drafts stored since the brainstorm version before the last batch of drafts, up to
/// that version. A user's edit (a version with no new drafts) shows the drafts the
/// report it edited was merged from. Each with its brainstormer's label.
pub fn round_drafts(run: &RunInfo, version: u32) -> Vec<(String, u32)> {
    let mut current: Vec<(String, u32)> = Vec::new();
    let mut batch: Vec<(String, u32)> = Vec::new();
    for doc in &run.docs {
        match doc.kind {
            DocKind::BrainstormDraft => {
                let label = match &doc.author {
                    DocAuthor::Brainstormer { label } => one_line(label),
                    _ => "draft".to_owned(),
                };
                batch.push((label, doc.version));
            }
            DocKind::Brainstorm => {
                if !batch.is_empty() {
                    current = std::mem::take(&mut batch);
                }
                if doc.version == version {
                    return current;
                }
            }
            _ => {}
        }
    }
    current
}
