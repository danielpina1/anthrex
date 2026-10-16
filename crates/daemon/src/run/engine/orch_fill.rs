//! Milestone 9.16 decision 29 (NA §5 A5): a live orchestrator's context fill, measured
//! by the driver every minute (`driver/handoff.rs::check_live`), is shown and never acted
//! on: at [`FILL_ATTENTION_PERCENT`] of a known window its run's attention says so. No
//! alert, no handoff mid-run, the window untouched. Pure.

use super::EngineState;
use crate::run::model::Run;
use crate::run::orch::contract_rounds::{FILL_ATTENTION_PERCENT, fill_attention_text};
use crate::transcript::fill::{Fill, fill_percent};
use proto::RunState;

/// `orchestrator context <n>% full` for `fill`, at or past [`FILL_ATTENTION_PERCENT`] of
/// its known window.
fn line_of(fill: Option<&Fill>) -> Option<String> {
    let fill = fill?;
    let percent = fill_percent(fill.tokens, fill.window?);
    (percent >= FILL_ATTENTION_PERCENT).then(|| fill_attention_text(percent))
}

/// Decision 29's attention line of `run`, while it is running.
pub(crate) fn attention(run: &Run) -> Option<String> {
    (run.state == RunState::Running)
        .then(|| line_of(run.orch.fill.as_ref()))
        .flatten()
}

/// `EventKind::OrchestratorFill`. Ruling C17: the fill is written only when what it
/// shows moves (its line, or whether its window is known), so a steady fill changes no
/// run and bumps no revision.
pub(super) fn record(state: &mut EngineState, run_id: &str, fill: Option<Fill>) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let shown = |fill: Option<&Fill>| (line_of(fill), fill.and_then(|f| f.window).is_some());
    if shown(run.orch.fill.as_ref()) != shown(fill.as_ref()) {
        run.orch.fill = fill;
    }
}
