//! Milestone 9.5 decision 45 (FU-F21): the alerts a stage's tier 3 raises, at priority
//! 3, beside `alerts.rs`'s (split out to keep it focused, `AGENTS.md` hard rule 8).
//! Pure: no I/O.

use proto::{FullState, RunInfo};

/// Milestone 9.5 decision 45: which tier-3 state a stage alert is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageAlert {
    Held,
    PropagateRed,
    Red,
}

/// Milestone 9.5 decision 45: each stage's tier-3 states the user must act on — held
/// after executor failures; red at propagation (its commit, review ruling I7); a red
/// tier 3 on the stage's head, or one whose commit is unknown (a `run.json` from before
/// ruling C-18, ruling T9-2), with its latest fix task, unless the orchestrator accepted
/// it (milestone 9.9). Which are the user's is `alerts_route`'s to say.
pub(super) fn stage_alerts(run: &RunInfo) -> Vec<(u16, StageAlert, String)> {
    let mut out = Vec::new();
    for stage in &run.stages {
        let n = stage.n;
        if stage.full.held {
            let text = format!(
                "stage {n} tier 3 held after executor failures; anthrex run resume retries"
            );
            out.push((n, StageAlert::Held, text));
        }
        if let Some(sha) = &stage.propagate_red {
            let sha7: String = sha.chars().take(7).collect();
            out.push((
                n,
                StageAlert::PropagateRed,
                format!("stage {n} propagate red at {sha7}"),
            ));
        }
        // Milestone 9.7 rulings T9-1, T9-2: an open PR's stage shows a red from an
        // earlier head (decision 12); a red known to be on another commit is stale. One
        // whose commit is unknown (before ruling C-18) still alerts.
        let stale = stage
            .full
            .commit
            .as_ref()
            .is_some_and(|c| Some(c) != stage.head.as_ref());
        if stage.full.state == FullState::Red && !stale && !stage.full.accepted {
            let mut text = format!("stage {n} tier 3 red");
            if let Some(fix) = stage.fix_tasks.last() {
                text.push_str(&format!(" · fix task {fix}"));
            }
            out.push((n, StageAlert::Red, text));
        }
    }
    out
}
