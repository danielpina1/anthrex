//! Milestone 9.5 decision 45 (FU-F21): three tier-3 states raise alerts at priority 3 —
//! a stage held after executor failures, a propagate red (named by its commit, review
//! ruling I7) and a red tier 3 — the two reds only while no orchestrator lives.

use super::alerts::{line, listed};
use super::runs::{app_with_runs, snapshot};
use crate::app::{AlertKey, StageAlert, alerts};
use crate::tree::alert_fixtures::{at, with_orch};
use crate::tree::stage_fixtures::stage;
use proto::{FullState, RunInfo, RunState};

const RED: &str = "bbbb2222cccc3333dddd4444eeee5555ffff6666";

/// `h-held`: halted, stage 1 held after executor failures. `r-reds`: running, stage 1
/// red at propagation, stage 2's tier 3 red with fix task `fix1`, stage 3's red with
/// none. Each with a live orchestrator when `orch` is set (its window not listed).
fn runs(orch: bool) -> Vec<RunInfo> {
    let mut held = at("h-held", RunState::Halted, 1);
    held.halted_reason = Some("stage 1: tier 3 held".into());
    let mut s1 = stage(1, Some("aaaa"), 1, 1);
    s1.full.held = true;
    held.stages = vec![s1];

    let mut reds = at("r-reds", RunState::Running, 2);
    let mut s1 = stage(1, Some("aaaa"), 1, 1);
    s1.propagate_red = Some(RED.into());
    let mut s2 = stage(2, Some("bbbb"), 1, 1);
    s2.full.state = FullState::Red;
    s2.fix_tasks = vec!["fix1".into()];
    let mut s3 = stage(3, Some("cccc"), 1, 1);
    s3.full.state = FullState::Red;
    reds.stages = vec![s1, s2, s3];
    if orch {
        vec![with_orch(held, 91), with_orch(reds, 92)]
    } else {
        vec![held, reds]
    }
}

#[test]
fn tier_3_states_raise_alerts() {
    let app = app_with_runs(vec![], snapshot(1, 100, runs(false)));
    assert_eq!(
        listed(&app),
        vec![
            line(3, "h-held", "run halted: stage 1: tier 3 held"),
            line(
                3,
                "h-held",
                "stage 1 tier 3 held after executor failures; anthrex run resume retries"
            ),
            line(3, "r-reds", "stage 1 propagate red at bbbb222"),
            line(3, "r-reds", "stage 2 tier 3 red · fix task fix1"),
            line(3, "r-reds", "stage 3 tier 3 red"),
        ]
    );
    let keys: Vec<AlertKey> = alerts(&app).into_iter().map(|a| a.key).collect();
    let stage_key = |run: &str, stage: u16, kind| AlertKey::Stage {
        run: run.into(),
        stage,
        kind,
    };
    assert_eq!(
        keys[1..],
        [
            stage_key("h-held", 1, StageAlert::Held),
            stage_key("r-reds", 1, StageAlert::PropagateRed),
            stage_key("r-reds", 2, StageAlert::Red),
            stage_key("r-reds", 3, StageAlert::Red),
        ]
    );

    // A live orchestrator is woken for the two reds; the held stage still alerts.
    let app = app_with_runs(vec![], snapshot(1, 100, runs(true)));
    assert_eq!(
        listed(&app),
        vec![
            line(3, "h-held", "run halted: stage 1: tier 3 held"),
            line(
                3,
                "h-held",
                "stage 1 tier 3 held after executor failures; anthrex run resume retries"
            ),
        ]
    );
}
