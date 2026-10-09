//! Milestone 9.10 decisions 6 and 12: a goal with no stored profile waits in its
//! repository's queue, and a drained goal starts through the run service.
//!
//! M9.10.4 creates this file with only the starter's stub; M9.10.5 fills it.

use std::sync::Arc;

use crate::profile::service_queue::GoalStarter;
use crate::run::driver::RunService;

impl RunService {
    /// Decision 6: the callback `profile::service::wire` installs (a weak handle once
    /// M9.10.5 fills it, as `set_live_runs`). Until then every drained start is
    /// refused, so a drained goal is recorded as dropped, never lost silently.
    pub(crate) fn queued_starter(self: &Arc<Self>) -> GoalStarter {
        Arc::new(|_goal| Box::pin(async { Err("not wired until M9.10.5".to_string()) }))
    }
}
