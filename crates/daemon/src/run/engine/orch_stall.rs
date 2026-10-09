//! Milestone 9.9 decisions 19 and 20 (OFA §4.1): a stalled or dead orchestrator. A wake
//! note that has waited `stall_after_secs` with no anthrex tool call or digest read since
//! stalls it; an exited window on a run that still has work is dead. Either way the
//! orchestrator no longer counts as living and its alerts go to the user. Pure.

use proto::{OrchestratorStuck, RunState};

use super::requests::log;
use crate::run::model::Run;

/// The orchestrator's note seq now: what `acted` marks as answered.
fn last_seq(run: &Run) -> u64 {
    run.orch
        .orchestrator
        .as_ref()
        .map_or(0, |o| o.last_note_seq)
}

/// Post-step: a note beyond `acted_seq` starts the clock, once.
pub(super) fn pass(run: &mut Run, now: u64) {
    if last_seq(run) > run.orch.acted_seq && run.orch.waiting_since.is_none() {
        run.orch.waiting_since = Some(now);
    }
}

/// A tick: a wait that reached `stall_after_secs` stalls a living orchestrator.
pub(super) fn tick(run: &mut Run, now: u64) {
    let Some(since) = run.orch.waiting_since else {
        return;
    };
    let live = run.orch.orchestrator.as_ref().is_some_and(|o| o.live);
    let stall = run.limits.stall_after_secs;
    if run.orch.stalled_at.is_some() || !live || run.state.is_terminal() || now < since + stall {
        return;
    }
    run.orch.stalled_at = Some(now);
    log(
        run,
        now,
        format!(
            "the orchestrator has not acted for {} min; its alerts go to you",
            stall / 60
        ),
    );
}

/// The orchestrator called a tool or read the digest: every note so far is answered,
/// and the clock stops until a newer one arrives.
pub(super) fn acted(run: &mut Run, now: u64) {
    run.orch.acted_seq = last_seq(run);
    run.orch.waiting_since = None;
    if run.orch.stalled_at.take().is_some() {
        log(run, now, "the orchestrator is acting again");
    }
}

/// Decisions 19 and 20: why the orchestrator does not count as living, if it does not.
pub(crate) fn stuck(run: &Run) -> Option<OrchestratorStuck> {
    let o = run.orch.orchestrator.as_ref()?;
    if !o.live && o.window_id.is_some() && matches!(run.state, RunState::Running | RunState::Halted)
    {
        return Some(OrchestratorStuck::Dead { since: o.exited_at });
    }
    let at = run.orch.stalled_at?;
    o.live.then_some(OrchestratorStuck::Stalled {
        since: run.orch.waiting_since.unwrap_or(at),
    })
}
