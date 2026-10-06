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
    s2.full.commit = Some("bbbb".into());
    s2.fix_tasks = vec!["fix1".into()];
    let mut s3 = stage(3, Some("cccc"), 1, 1);
    s3.full.state = FullState::Red;
    s3.full.commit = Some("cccc".into());
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

/// Review minor 2: a held stage's alert opens its run's menu with `Resume`
/// preselected (`anthrex run resume` retries it); a red one opens its stage's menu with
/// no preselection. The detail reads the phase, the stage of the run's stages and the
/// run.
#[test]
fn stage_alerts_open_their_node_and_name_their_stage() {
    use crate::actions_request::ActionTarget;
    use crate::app::alerts_view::{alert_node, preselected};
    use proto::ActionKind;
    let app = app_with_runs(vec![], snapshot(1, 100, runs(false)));
    let all = alerts(&app);
    let node = |i: usize| alert_node(&all[i].key);
    assert_eq!(node(1), Some(("h-held".into(), ActionTarget::Run)));
    assert_eq!(node(2), Some(("r-reds".into(), ActionTarget::Stage(1))));
    assert_eq!(node(3), Some(("r-reds".into(), ActionTarget::Stage(2))));
    assert_eq!(preselected(&app, &all[1].key), Some(ActionKind::Resume));
    for alert in &all[2..] {
        assert_eq!(preselected(&app, &alert.key), None, "{:?}", alert.key);
    }
    let rows = |i: usize| -> Vec<String> {
        crate::ui::alerts_view::detail_lines(&app, &all[i], 80)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|text| {
                ["phase ", "stage ", "run "]
                    .iter()
                    .any(|l| text.starts_with(l))
            })
            .collect()
    };
    assert_eq!(
        rows(1),
        [
            "phase tier 3 held",
            "stage 1 of 1",
            "run Add password reset · held"
        ]
    );
    assert_eq!(rows(2)[..2], ["phase propagate red", "stage 1 of 3"]);
    assert_eq!(rows(4)[..2], ["phase tier 3 red", "stage 3 of 3"]);
}

/// Milestone 9.7 ruling T9-1: an open PR's stage shows its latest tier-3 verdict even
/// when that job ran on an earlier head (decision 12). Such a red is stale: it raises
/// no alert. A red job on the stage's current head, or one whose commit is unknown
/// (ruling T9-2), still does.
#[test]
fn only_a_red_on_the_stages_head_or_of_an_unknown_commit_raises_an_alert() {
    let app_for = |commit: Option<&str>| {
        let mut run = at("r-pr", RunState::Running, 1);
        let mut s1 = stage(1, Some("bbbb"), 1, 1);
        s1.full.state = FullState::Red;
        s1.full.commit = commit.map(str::to_string);
        run.stages = vec![s1];
        app_with_runs(vec![], snapshot(1, 100, vec![run]))
    };
    let stage_reds = |app: &crate::app::App| {
        alerts(app)
            .into_iter()
            .filter(|a| matches!(a.key, AlertKey::Stage { .. }))
            .count()
    };
    assert_eq!(
        stage_reds(&app_for(Some("aaaa"))),
        0,
        "a red on an earlier head"
    );
    assert_eq!(stage_reds(&app_for(Some("bbbb"))), 1, "a red on the head");
    // Ruling T9-2: a red whose commit is unknown (a `run.json` from before ruling
    // C-18) is not known to be stale, so it still alerts.
    assert_eq!(stage_reds(&app_for(None)), 1, "a red with no commit");
    assert_eq!(
        listed(&app_for(Some("bbbb"))),
        vec![line(3, "r-pr", "stage 1 tier 3 red")]
    );
}
