//! Milestone 9's driver side. Task M9.6: decision 20's overlay of each run's scouts
//! onto the pure snapshot. Task M9.7: the user's hold verdicts and `run promote`,
//! handed to the engine. Later tasks add the orchestrator's tool routing here.

use proto::run_wire::request;
use proto::{OrchestratorChoice, RunReply, RunsSnapshot};

use super::RunService;
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::orch::extract::ExtractSlot;

fn answer(label: &str, result: Result<String, String>) -> RunReply {
    match result {
        Ok(message) => RunReply::done(label, message),
        Err(message) => RunReply::refused(label, message),
    }
}

impl RunService {
    /// Decision 20: each run's `RunInfo.scouts` is M8b's `ScoutService::run_scouts`,
    /// with its live counters, which the pure snapshot cannot call. Called with the
    /// engine's lock released; `run_scouts` takes only the scout table's.
    pub(super) fn with_scouts(&self, mut snap: RunsSnapshot) -> RunsSnapshot {
        if let Some(adaptation) = self.adaptation.get() {
            for run in snap.runs.iter_mut() {
                run.scouts = adaptation.scouts.run_scouts(&run.run_id);
            }
        }
        snap
    }

    /// Decision 28: `run approve|reject --hold`, the only way a hold is decided.
    pub(super) async fn hold_verdict(
        &self,
        run_id: String,
        hold: String,
        approve: bool,
    ) -> RunReply {
        let (label, result) = if approve {
            let event = |reply| OrchEvent::ApproveHold {
                reply,
                run_id,
                hold,
            };
            (
                request::APPROVE,
                self.ask(|r| EventKind::Orch(event(r))).await,
            )
        } else {
            let event = |reply| OrchEvent::RejectHold {
                reply,
                run_id,
                hold,
            };
            (
                request::REJECT,
                self.ask(|r| EventKind::Orch(event(r))).await,
            )
        };
        answer(label, result)
    }

    /// Decision 29: `run promote`, performed by the engine.
    pub(super) async fn promote(
        &self,
        run_id: String,
        orchestrator: Option<OrchestratorChoice>,
    ) -> RunReply {
        let event = |reply| EventKind::Promote {
            reply,
            run_id,
            orchestrator,
        };
        answer(request::PROMOTE, self.ask(event).await)
    }
}

/// Decision 34, the driver's half: a first turn with the scout extract of its slot's
/// reports, read on a blocking thread (each resolved only to a report anthrex stored,
/// through `read_report`'s guards); a report that cannot be read is left out with a
/// warning (`readable_reports`). With no slot, the turn as the engine built it.
pub(super) async fn fill_extract(
    ctx: &super::OpCtx,
    project: &std::path::Path,
    slot: Option<ExtractSlot>,
    first_turn: String,
) -> String {
    let Some(slot) = slot else {
        return first_turn;
    };
    let run_dir = crate::run::journal::runs_dir(&ctx.data_dir).join(&ctx.run_id);
    let repo_dir = crate::profile::repo_dir(&ctx.data_dir, project);
    let fallback = first_turn.clone();
    tokio::task::spawn_blocking(move || filled(&slot, &run_dir, &repo_dir, &first_turn))
        .await
        .unwrap_or(fallback)
}

/// [`fill_extract`]'s blocking body.
pub(super) fn filled(
    slot: &ExtractSlot,
    run_dir: &std::path::Path,
    repo_dir: &std::path::Path,
    first_turn: &str,
) -> String {
    use crate::run::orch::extract::{readable_reports, scout_extract};
    use crate::scout::report::resolve_ref;
    use crate::scout::spec::valid_id;
    let read = slot
        .refs
        .iter()
        .map(|reference| {
            let report = if valid_id(reference) {
                let onboarding = slot.onboarding.as_deref().filter(|id| valid_id(id));
                let path = resolve_ref(reference, run_dir, repo_dir, onboarding);
                super::adapt::read_report(&path)
            } else {
                Err("not a stored report".to_string())
            };
            (reference.clone(), report)
        })
        .collect();
    slot.fill(first_turn, &scout_extract(&readable_reports(read)))
}

#[cfg(test)]
#[path = "orch_tests.rs"]
mod tests;
