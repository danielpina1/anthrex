//! Milestone 9's driver side. Task M9.6: decision 20's overlay of each run's scouts
//! onto the pure snapshot. Later tasks add the orchestrator's tool routing here.

use proto::RunsSnapshot;

use super::RunService;

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
}

#[cfg(test)]
#[path = "orch_tests.rs"]
mod tests;
