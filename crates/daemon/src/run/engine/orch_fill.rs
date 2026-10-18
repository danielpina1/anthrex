//! Milestone 9.16 decision 29 (NA §5 A5): a live orchestrator's context fill, measured
//! by the driver every minute (`driver/handoff.rs::check_live`), is shown and never acted
//! on: at [`FILL_ATTENTION_PERCENT`] of a known window its run's attention says so. No
//! alert, no handoff mid-run, the window untouched. Pure.

use super::EngineState;
use crate::run::model::Run;
use crate::run::orch::contract_rounds::{FILL_ATTENTION_PERCENT, fill_attention_text};
use crate::transcript::fill::{Fill, fill_percent};
use common::numbers;
use proto::RunState;

/// `orchestrator context <n>% full (<tokens> / <window>)` for `fill` (or `… over 100%`
/// past the window), at or past [`FILL_ATTENTION_PERCENT`] of its known window. The
/// threshold is `fill_percent`'s, unchanged; only the text is milestone 9.19's.
fn line_of(fill: Option<&Fill>) -> Option<String> {
    let fill = fill?;
    let window = fill.window?;
    let percent = fill_percent(fill.tokens, window);
    (percent >= FILL_ATTENTION_PERCENT).then(|| fill_attention_text(fill.tokens, window))
}

/// Decision 29's attention line of `run`, while it is running.
pub(crate) fn attention(run: &Run) -> Option<String> {
    (run.state == RunState::Running)
        .then(|| line_of(run.orch.fill.as_ref()))
        .flatten()
}

/// The attention's step for `fill`: its floored percent at or past
/// [`FILL_ATTENTION_PERCENT`] of a known window, and `Over` past the window.
fn step_of(fill: Option<&Fill>) -> Option<numbers::Fill> {
    let fill = fill?;
    let window = fill.window?;
    (fill_percent(fill.tokens, window) >= FILL_ATTENTION_PERCENT)
        .then(|| numbers::fill(fill.tokens, window))
}

/// `EventKind::OrchestratorFill`. Ruling C17: the fill is written only when the
/// attention's step moves (its percent, `over 100%` past the window, or whether the
/// window is known), so a steady fill changes no run and bumps no revision. The line
/// shows tokens too, but they move every 1k: keyed on the text, each step would bump
/// the run and the digest and wake the orchestrator (fix round 1, review I2). The cost:
/// the shown tokens lag by under 1% of the window, and past the window they stay at
/// the first value past it until the fill drops back.
pub(super) fn record(state: &mut EngineState, run_id: &str, fill: Option<Fill>) {
    let Some(run) = state.runs.get_mut(run_id) else {
        return;
    };
    let shown = |fill: Option<&Fill>| (step_of(fill), fill.and_then(|f| f.window).is_some());
    if shown(run.orch.fill.as_ref()) != shown(fill.as_ref()) {
        run.orch.fill = fill;
    }
}
