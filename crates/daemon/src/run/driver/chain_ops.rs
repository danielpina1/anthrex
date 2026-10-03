//! Milestone 9.3 decision 19, the driver's half of chains: the idle orchestrators'
//! windows (KG §3.2), and decision 23's adoption of an idle chain's window by its next
//! run (`Effect::AdoptOrchestrator`). I/O; the engine lock is taken only to look the
//! chains up, never across an await or a blocking call, and the adoption's manager
//! calls run on `spawn_blocking`, holding no lock of the engine's (AGENTS.md rules 2
//! and 10).

use std::sync::Arc;
use std::time::Duration;

use proto::{AgentRole, RunRef};

use super::super::RunService;
use super::chain_goal::Handoff;
use crate::run::chain::{ChainState, newest};
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::orch::contract::orchestrator_first_prompt;

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
    /// if it has not ended (`mark_live`). The session is never restarted. Fix round 1
    /// (m1): a rebind that fails (the window gone, or not a run window) is an adoption
    /// lost ([`Self::adopt_lost`]).
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
                self.adopt_lost(run_id, window_id).await;
            }
            Err(error) => {
                tracing::warn!(run = %run_id, window_id, %error, "adopting the orchestrator panicked");
            }
        }
    }

    /// Task 6b fix round 1 (m1): run `run_id` never took window `window_id`. Its
    /// handoff (decision 24, after the chain's previous run, the history read within
    /// the run's `git_timeout_secs`, holding no lock) goes to the engine with
    /// `OrchEvent::AdoptLost`, which leaves the run no window, so `run resume` launches
    /// a fresh session with it.
    pub(in crate::run::driver) async fn adopt_lost(&self, run_id: &str, window_id: u32) {
        let found = {
            let state = crate::lock(&self.state); // lookup
            state.runs.get(run_id).and_then(|run| {
                let chain = run.chain.as_deref()?;
                let prev = newest(&state.runs, chain, Some(run_id))?;
                let runs: Vec<&crate::run::model::Run> = state
                    .runs
                    .values()
                    .filter(|r| r.chain.as_deref() == Some(chain))
                    .collect();
                // The final fix wave (A-M2): the continue order, as `rebuild`'s.
                let runs = crate::run::chain::in_order(runs);
                let ids = runs.iter().map(|r| r.id.clone()).collect();
                let bound = Duration::from_secs(run.limits.git_timeout_secs);
                Some((
                    Handoff::of(chain, ids, prev),
                    orchestrator_first_prompt(run),
                    bound,
                ))
            })
        };
        let Some((handoff, first, bound)) = found else {
            return;
        };
        let first_prompt = handoff.prompt(&first, Some(bound)).await;
        self.send(EventKind::Orch(OrchEvent::AdoptLost {
            run_id: run_id.to_string(),
            window_id,
            first_prompt,
        }));
    }

    /// Task 6b fix round 1 (m1), after a restart: a chained run that has not ended and
    /// starts no later goal, whose orchestrator record names a window the manager's
    /// record gives to another run, never took that window (the daemon stopped between
    /// the step that adopted it and the adoption): an adoption lost.
    pub(in crate::run::driver) async fn lost_adoptions(&self) {
        let named: Vec<(String, u32)> = crate::lock(&self.state) // lookup
            .runs
            .values()
            .filter(|run| run.chain.is_some() && run.continued_by.is_none())
            .filter(|run| !run.state.is_terminal())
            .filter_map(|run| Some((run.id.clone(), run.orch.orchestrator.as_ref()?.window_id?)))
            .collect();
        for (run_id, window_id) in named {
            let other = self.manager.run_window_run(window_id);
            if other.is_some_and(|r| r.run_id != run_id) {
                self.adopt_lost(&run_id, window_id).await;
            }
        }
    }
}

#[cfg(test)]
#[path = "chain_ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "chain_adopt_tests.rs"]
mod adopt_tests;
