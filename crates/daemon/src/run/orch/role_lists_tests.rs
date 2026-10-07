//! Milestone 9.6 decision 10's design agents over the role table (milestone 9.8, MR
//! §3.1): the brainstormers are the `brainstorm` row, the document reviewer the
//! `reviewer` row's pick against the orchestrator (decision 27), and ruling T8-4's
//! unsaved rule on both. Milestone 9.5's list tests went with the lists' launches
//! (task M9.8.7b).

use proto::{Effort, Runtime, Strength};

use super::*;
use crate::run::orch::test_support::{orchestrator as record_of, run_of};

const LUNA: &str = "gpt-6-luna";

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength,
        effort,
    }
}

/// Milestone 9.8: a run with an orchestrator and `brainstorm` = `first`, `second` at
/// `effort`.
fn brainstorm_row(first: &str, second: &str, effort: Option<&str>) -> Run {
    let mut run = run_of(1);
    run.orch.orchestrator = Some(record_of());
    let mut models = run.limits.models().clone();
    models.brainstorm = proto::models::BrainstormChoice {
        first: proto::models::ModelRef::parse(first).unwrap(),
        second: proto::models::ModelRef::parse(second).unwrap(),
        effort: effort.map(str::to_string),
    };
    run.limits.models = Some(models);
    run
}

fn lanes(picks: &[BrainstormPick]) -> Vec<(String, Runtime, String, String)> {
    (picks.iter())
        .map(|p| {
            let r = &p.route;
            let effort = r.effort.as_str().to_string();
            (p.label.clone(), r.runtime, r.model.clone(), effort)
        })
        .collect()
}

fn lane(
    label: &str,
    runtime: Runtime,
    model: &str,
    effort: &str,
) -> (String, Runtime, String, String) {
    (label.into(), runtime, model.into(), effort.into())
}

/// Milestone 9.8 (MR §3.1): the two brainstormers are the brainstorm row's two models
/// at its effort, labelled by runtime on two runtimes and `A`/`B` on one.
#[test]
fn brainstormers_are_the_brainstorm_row() {
    let run = brainstorm_row("claude:claude-sonnet-5", "codex:gpt-6-luna", Some("medium"));
    let picks = brainstorm_picks_with(&run, &crate::decider::DECIDER_CAPS).unwrap();
    assert_eq!(
        lanes(&picks),
        [
            lane("claude", Runtime::Claude, "claude-sonnet-5", "medium"),
            lane("codex", Runtime::Codex, LUNA, "medium"),
        ]
    );
    let run = brainstorm_row("claude:claude-sonnet-5", "claude:claude-opus-5-5", None);
    let picks = brainstorm_picks_with(&run, &crate::decider::DECIDER_CAPS).unwrap();
    assert_eq!(
        lanes(&picks),
        [
            lane("A", Runtime::Claude, "claude-sonnet-5", ""),
            lane("B", Runtime::Claude, "claude-opus-5-5", ""),
        ]
    );
}

/// Ruling T8-4 on the row: a model whose CLI cannot run unsaved is replaced by the row's
/// other model; with neither usable, no brainstormer.
#[test]
fn a_brainstormer_whose_cli_cannot_run_unsaved_is_replaced_by_the_other() {
    let caps = crate::decider::DeciderCaps {
        codex_ephemeral: false,
        ..crate::decider::DECIDER_CAPS
    };
    let run = brainstorm_row("claude:claude-opus-5-5", "codex:default", Some("high"));
    let picks = brainstorm_picks_with(&run, &caps).unwrap();
    let opus = "claude-opus-5-5";
    assert_eq!(
        lanes(&picks),
        [
            lane("A", Runtime::Claude, opus, "high"),
            lane("B", Runtime::Claude, opus, "high"),
        ]
    );
    let neither = crate::decider::DeciderCaps {
        claude_no_session_persistence: false,
        ..caps
    };
    assert_eq!(brainstorm_picks_with(&run, &neither), None);
}

/// Decision 27 for the document reviewer, whose author is the orchestrator: a reviewer
/// row on the orchestrator's own model takes its fallback.
#[test]
fn the_document_reviewer_is_never_the_orchestrators_model() {
    let mut run = run_of(1);
    let mut o = record_of();
    o.route = route(
        Runtime::Claude,
        "claude-opus-5-5",
        Strength::Standard,
        Effort::HIGH,
    );
    run.orch.orchestrator = Some(o);
    let opus = "claude:claude-opus-5-5";
    let luna = Some("codex:gpt-6-luna");
    let reviewer = proto::models::Role::Reviewer;
    crate::run::test_support::set_row(&mut run, reviewer, opus, Some("high"), luna);
    let pick = review_pick(&run, &crate::decider::DECIDER_CAPS).unwrap();
    let picked = &pick.route;
    assert_eq!(
        (
            picked.runtime,
            picked.model.as_str(),
            picked.effort.as_str()
        ),
        (Runtime::Codex, LUNA, "")
    );
    assert_eq!((pick.why, pick.warning), (None, None));
}

/// A run whose orchestrator runs Opus at high, its `reviewer` row `model` with
/// `fallback`.
fn reviewing(model: &str, fallback: Option<&str>) -> Run {
    let mut run = run_of(1);
    let mut o = record_of();
    o.route = route(
        Runtime::Claude,
        "claude-opus-5-5",
        Strength::Standard,
        Effort::HIGH,
    );
    run.orch.orchestrator = Some(o);
    let reviewer = proto::models::Role::Reviewer;
    crate::run::test_support::set_row(&mut run, reviewer, model, None, fallback);
    run
}

const SAME_OPUS: &str = "reviewer: claude:claude-opus-5-5 reviews work by the same model; set an \"if it struggles\" model for the reviewer in C-b S";

/// MR D3, decision 27 (fix round 1, I1): a reviewer row on the orchestrator's model with
/// no fallback reviews on it, with the run-log warning.
#[test]
fn a_document_reviewer_on_the_orchestrators_model_is_warned() {
    let run = reviewing("claude:claude-opus-5-5", None);
    let pick = review_pick(&run, &crate::decider::DECIDER_CAPS).unwrap();
    assert_eq!(pick.route.model, "claude-opus-5-5");
    assert_eq!(pick.why, None);
    assert_eq!(pick.warning.as_deref(), Some(SAME_OPUS));
}

/// Fix round 1 (I1): a reviewer that falls back to the orchestrator's own route reviews
/// work by the same model, and says so too.
#[test]
fn a_document_reviewer_on_the_orchestrators_own_route_is_warned() {
    let run = reviewing("codex:default", None);
    let caps = crate::decider::DeciderCaps {
        codex_ephemeral: false,
        ..crate::decider::DECIDER_CAPS
    };
    let pick = review_pick(&run, &caps).unwrap();
    assert_eq!(pick.route.model, "claude-opus-5-5");
    let why = "the codex CLI cannot run a session without saving it";
    assert_eq!(pick.why.as_deref(), Some(why));
    assert_eq!(pick.warning.as_deref(), Some(SAME_OPUS));
}

/// Fix round 1 (M4, D3): a reviewer row's pick that cannot run is replaced by the row's
/// fallback, when it can run and is not the author's model, before the orchestrator's
/// own route.
#[test]
fn a_document_reviewer_that_cannot_run_takes_its_rows_fallback() {
    let run = reviewing("codex:default", Some("claude:claude-sonnet-5"));
    let caps = crate::decider::DeciderCaps {
        codex_ephemeral: false,
        ..crate::decider::DECIDER_CAPS
    };
    let pick = review_pick(&run, &caps).unwrap();
    assert_eq!(
        (pick.route.runtime, pick.route.model.as_str()),
        (Runtime::Claude, "claude-sonnet-5")
    );
    assert_eq!(pick.why, None);
    let line = "reviewer: codex:default cannot review: the codex CLI cannot run a session without saving it; its \"if it struggles\" model claude:claude-sonnet-5 reviews";
    assert_eq!(pick.warning.as_deref(), Some(line));
    // A fallback on the author's own model is not taken over the orchestrator's route
    // with its reason.
    let run = reviewing("codex:default", Some("claude:claude-opus-5-5"));
    let pick = review_pick(&run, &caps).unwrap();
    assert!(pick.why.is_some());
    assert_eq!(pick.warning.as_deref(), Some(SAME_OPUS));
}

/// Ruling T8-4: the row's installed runtimes whose CLI cannot run unsaved are said so;
/// an uninstalled one is not named.
#[test]
fn a_rows_runtime_that_cannot_run_unsaved_is_named() {
    let caps = crate::decider::DeciderCaps {
        codex_ephemeral: false,
        ..crate::decider::DECIDER_CAPS
    };
    let run = brainstorm_row("claude:claude-opus-5-5", "codex:default", Some("high"));
    assert_eq!(unsaved_missing(&run, &crate::decider::DECIDER_CAPS), []);
    assert_eq!(unsaved_missing(&run, &caps), [Runtime::Codex]);
    let mut gone = run.clone();
    gone.orch.installed = [("codex".to_string(), false)].into();
    assert_eq!(unsaved_missing(&gone, &caps), []);
    let claude_only = brainstorm_row("claude:claude-opus-5-5", "claude:claude-sonnet-5", None);
    assert_eq!(
        unsaved_missing(&claude_only, &caps),
        [],
        "codex is not in the row"
    );
}

/// Ruling F16: a brainstormer's record is the row's: source `role_table`, policy
/// `m9.8-roles-v1`, the row's two models at its effort the candidates.
#[test]
fn a_brainstormers_record_is_the_rows() {
    use crate::run::design::state::{DesignAgent, DesignAgentState};
    use crate::run::orch::roles::design_agent_record;
    let run = brainstorm_row("claude:claude-sonnet-5", "codex:gpt-6-luna", Some("medium"));
    let pick = brainstorm_picks_with(&run, &crate::decider::DECIDER_CAPS).unwrap()[1].clone();
    let agent = DesignAgent {
        label: pick.label.clone(),
        role: proto::AgentRole::Brainstormer,
        route: pick.route.clone(),
        session: 1,
        window_id: None,
        state: DesignAgentState::Queued,
        calls: 0,
        tokens: 0,
        started: None,
        listed: false,
        unsubmitted: false,
        round: 1,
    };
    let d = design_agent_record(&run, &agent, false, 5);
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        ("role_table", "m9.8-roles-v1")
    );
    let models: Vec<&str> = d
        .candidates
        .iter()
        .map(|c| c.route.model.as_str())
        .collect();
    assert_eq!(models, ["claude-sonnet-5", LUNA]);
    assert_eq!((d.selected_index, d.chosen.clone()), (1, pick.route));
}
