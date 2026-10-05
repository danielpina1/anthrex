//! Milestone 9.6 task M9.6.9 (decision 7, DF §2.1), part of `design_agents.rs`: a
//! rethink starts brainstorm round k+1. Its pack's inputs are frozen anew (ruling T8-2):
//! the round's scout reports as frozen before, a continued goal's earlier spec, and now
//! the user's note and the report it replaces; the round writes its own pack file
//! (`pack-r<k+1>.md`, ruling T8-6). The drafts settle again (ruling T8-5), and both
//! brainstormers relaunch as their next sessions, their drafts held until both have
//! ended (ruling T8-1). Pure (design decision 2).

use proto::DocKind;

use super::super::requests::log;
use crate::run::design::pack::{FrozenPack, FrozenRethink, freeze};
use crate::run::design::state::{DesignAgentState, design_dir};
use crate::run::model::Run;

/// The user asked to rethink the brainstorm with `note` (the run is back in
/// brainstorming, `rethinks` counted): both brainstormers are queued again for round
/// `rethinks + 1`, with the round's inputs frozen.
pub(in crate::run::engine) fn rethink(run: &mut Run, note: &str, now: u64) {
    let dir = design_dir(run);
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    let previous = (design.find(DocKind::Brainstorm, None)).map(|v| FrozenRethink {
        note: note.to_string(),
        path: dir.join(design.file_name(v)),
        version: v.clone(),
    });
    let before = design.pack.clone().unwrap_or_else(|| freeze(run, None));
    let round = design.rethinks + 1;
    // Never empty here: the brainstorm gate opens only on a report taken once the
    // drafts settled (`report_doc`), which `settle` does only with brainstormers, and
    // the list is only ever replaced by `start_brainstorm`, refused after the first.
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    design.pack = Some(FrozenPack {
        reports: before.reports,
        earlier: before.earlier,
        round,
        file: None,
        rethink: previous,
    });
    design.drafts_settled = false;
    design.held.clear();
    for agent in design.brainstormers.iter_mut() {
        agent.state = DesignAgentState::Queued;
        agent.window_id = None;
        agent.started = None;
        agent.unsubmitted = false;
    }
    let labels: Vec<String> = (design.brainstormers.iter())
        .map(|a| a.label.clone())
        .collect();
    // Ruling T13-1: each one's next session is routed `rethink`.
    design.rethink_starts = labels.clone();
    let text = format!(
        "brainstormers queued again for round {round}: {}",
        labels.join(", ")
    );
    log(run, now, text);
}
