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
/// tier 3 on the stage's head (its latest fix task). The two reds are the
/// orchestrator's while it lives.
pub(super) fn stage_alerts(run: &RunInfo) -> Vec<(u16, StageAlert, String)> {
    let mut out = Vec::new();
    let orchestrated = super::alerts::orchestrator_lives(run);
    for stage in &run.stages {
        let n = stage.n;
        if stage.full.held {
            let text = format!(
                "stage {n} tier 3 held after executor failures; anthrex run resume retries"
            );
            out.push((n, StageAlert::Held, text));
        }
        if orchestrated {
            continue;
        }
        if let Some(sha) = &stage.propagate_red {
            let sha7: String = sha.chars().take(7).collect();
            out.push((
                n,
                StageAlert::PropagateRed,
                format!("stage {n} propagate red at {sha7}"),
            ));
        }
        // Milestone 9.7 ruling T9-1: only a red job on the stage's current head. An
        // open PR's stage shows a red from an earlier head (decision 12); it is stale.
        let on_head = stage.full.commit.is_some() && stage.full.commit == stage.head;
        if stage.full.state == FullState::Red && on_head {
            let mut text = format!("stage {n} tier 3 red");
            if let Some(fix) = stage.fix_tasks.last() {
                text.push_str(&format!(" · fix task {fix}"));
            }
            out.push((n, StageAlert::Red, text));
        }
    }
    out
}
