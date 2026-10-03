//! Milestone 9.3 decision 19, the driver's half of chains: the idle orchestrators'
//! windows (KG §3.2), and decision 23's adoption of an idle chain's window by its next
//! run (`Effect::AdoptOrchestrator`). I/O; the engine lock is taken only to look the
//! chains up, never across an await or a blocking call, and the adoption's manager
//! calls run on `spawn_blocking`, holding no lock of the engine's (AGENTS.md rules 2
//! and 10).

use std::sync::Arc;

use proto::{AgentRole, RunRef};

use super::super::RunService;
use crate::run::chain::ChainState;
use crate::run::engine::{EventKind, OrchEvent};

impl RunService {
    /// Decision 19: an idle chain whose session has not ended, and whose window `gone`
    /// says is gone from the manager's list or `Exited` for `EXIT_CONFIRM`, is reported
    /// to the engine (`OrchEvent::ChainWindowGone`), which ends it. Called by
    /// `check_orchestrators` with the window list it read.
    pub(in crate::run::driver) fn idle_windows(&self, gone: impl Fn(u32) -> bool) {
        let idle: Vec<(String, u32)> = crate::lock(&self.state) // lookup
            .chains
            .values()
            .filter(|chain| chain.state == ChainState::Idle && !chain.ended)
            .map(|chain| (chain.id.clone(), chain.window_id))
            .collect();
        for (chain, window_id) in idle {
            if gone(window_id) {
                self.send(EventKind::Orch(OrchEvent::ChainWindowGone {
                    chain,
                    window_id,
                }));
            }
        }
    }

    /// Decision 23: `Effect::AdoptOrchestrator`, on a task of its own.
    pub(in crate::run::driver) fn adopt(
        self: &Arc<Self>,
        run_id: String,
        window_id: u32,
        name: String,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            service.adopt_window(&run_id, window_id, name, || {}).await;
        });
    }

    /// Decision 23: window `window_id` is renamed `name` and rebound to run `run_id`
    /// (`WindowManager::rename`, then `rebind_run_window`, each taking the manager's
    /// lock for one change) on `spawn_blocking`, after `hold` (a test's; production
    /// passes nothing), with no engine lock held; then its live flag is set for the run
    /// if it has not ended (`mark_live`). The session is never restarted. A failure is
    /// logged: the window's own check (`check_orchestrators`) reports a window gone.
    pub(in crate::run::driver) async fn adopt_window(
        &self,
        run_id: &str,
        window_id: u32,
        name: String,
        hold: impl FnOnce() + Send + 'static,
    ) {
        let session = crate::lock(&self.state) // lookup
            .runs
            .get(run_id)
            .and_then(|run| run.orch.orchestrator.as_ref())
            .map_or(1, |o| o.session);
        let run_ref = RunRef {
            run_id: run_id.to_string(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session,
        };
        let manager = self.manager.clone();
        let done = tokio::task::spawn_blocking(move || {
            hold();
            if let Err(error) = manager.rename(window_id, name) {
                tracing::warn!(window_id, %error, "the adopted orchestrator kept its name");
            }
            manager.rebind_run_window(window_id, run_ref)
        })
        .await;
        match done {
            Ok(Ok(())) => self.mark_live(run_id, window_id),
            Ok(Err(error)) => {
                tracing::warn!(run = %run_id, window_id, %error, "the orchestrator was not adopted");
            }
            Err(error) => {
                tracing::warn!(run = %run_id, window_id, %error, "adopting the orchestrator panicked");
            }
        }
    }
}

#[cfg(test)]
#[path = "chain_ops_tests.rs"]
mod tests;
