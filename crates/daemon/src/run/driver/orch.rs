//! Milestone 9's driver side. Task M9.6: decision 20's overlay of each run's scouts
//! onto the pure snapshot. Task M9.7: the user's hold verdicts and `run promote`,
//! handed to the engine. Later tasks add the orchestrator's tool routing here.

use proto::run_wire::request;
use proto::{OrchestratorChoice, RunReply, RunsSnapshot};

use super::RunService;
use crate::run::engine::{EventKind, OrchEvent};

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

#[cfg(test)]
#[path = "orch_tests.rs"]
mod tests;
