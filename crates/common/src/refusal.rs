//! The daemon's refusals of a client's control request that the client also shows
//! before asking (decision 10). Pure.

use proto::RunRef;

/// Decision 49's answer to a client's `Input`, `Kill`, `Remove` or `Restart` for a
/// headless window of `run`, or of no run: a repository-level scout (M8b decision 12).
pub fn control_refusal(id: u32, run: Option<&RunRef>) -> String {
    let Some(run) = run else {
        return format!(
            "window {id} is a headless scout session; only the daemon drives it. Use anthrex profile reject to stop it"
        );
    };
    format!(
        "window {id} is a headless session of run {}; only the engine drives it. Use anthrex run cancel to stop it",
        run.run_id
    )
}

/// Decision 11's refusal of `kill` and `remove` for a live run's orchestrator.
pub fn orchestrator_refusal(id: u32, run_id: &str) -> String {
    format!(
        "window {id} is the orchestrator of run {run_id}; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart {id}"
    )
}
