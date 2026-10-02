//! Milestone 9.3 decision 19, the driver's half of chains: the idle orchestrators'
//! windows (KG §3.2). I/O; the engine lock is taken only to look the chains up, never
//! across an await or a blocking call.

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
}
