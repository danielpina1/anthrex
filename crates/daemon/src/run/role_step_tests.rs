use proto::Route;
use proto::models::{ModelRef, ModelTable, Role, RoleChoice};

use super::escalate;
use crate::run::model_roles::{ModelEfforts, RunModels};

fn m(s: &str) -> ModelRef {
    ModelRef::parse(s).unwrap()
}

fn models(
    model: &str,
    effort: Option<&str>,
    fallback: Option<&str>,
    lists: &[(&str, &[&str], Option<&str>)],
) -> RunModels {
    let row = RoleChoice {
        model: m(model),
        effort: effort.map(Into::into),
        fallback: fallback.map(m),
    };
    let table = ModelTable {
        rows: [(Role::ImplementerMedium, row)].into(),
        brainstorm: None,
    };
    let mut r = RunModels::resolve(&table, None);
    for (name, efforts, default) in lists {
        r.efforts.insert(
            m(name),
            ModelEfforts {
                efforts: efforts.iter().map(|e| e.to_string()).collect(),
                default: default.map(Into::into),
            },
        );
    }
    r
}

fn at(model: &str, effort: &str) -> Route {
    RunModels::route_of(&m(model), (!effort.is_empty()).then_some(effort))
}

const SOL: &str = "codex:gpt-6-sol";
const OPUS: &str = "claude:claude-opus-5-5";

#[test]
fn effort_climbs_the_models_list_first() {
    let r = models(
        SOL,
        Some("medium"),
        Some(OPUS),
        &[(SOL, &["low", "medium", "high", "max"], Some("medium"))],
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, "medium"), &[]),
        Some(at(SOL, "high"))
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, "high"), &[]),
        Some(at(SOL, "max"))
    );
}

#[test]
fn at_the_top_it_switches_to_the_fallback_at_its_default_effort() {
    let r = models(
        SOL,
        Some("medium"),
        Some(OPUS),
        &[(SOL, &["low", "medium", "high", "max"], Some("medium"))],
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, "max"), &[]),
        Some(at(OPUS, ""))
    );
}

#[test]
fn on_the_fallback_it_climbs_the_fallbacks_list_then_stays() {
    let r = models(
        SOL,
        None,
        Some(OPUS),
        &[
            (SOL, &["low"], None),
            (OPUS, &["low", "medium", "high"], Some("high")),
        ],
    );
    // DEFAULT counts as the model's default effort (`high`), the top: nothing left.
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(OPUS, ""), &[]),
        None
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(OPUS, "medium"), &[]),
        Some(at(OPUS, "high"))
    );
}

#[test]
fn without_a_fallback_it_stays_at_the_top() {
    let r = models(
        SOL,
        Some("high"),
        None,
        &[(SOL, &["low", "medium", "high"], Some("medium"))],
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, "high"), &[]),
        None
    );
}

#[test]
fn a_model_with_no_reported_efforts_goes_straight_to_the_fallback() {
    let r = models(SOL, Some("medium"), Some(OPUS), &[]);
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, "medium"), &[]),
        Some(at(OPUS, ""))
    );
}

#[test]
fn a_default_effort_with_no_model_default_starts_below_the_first() {
    let r = models(SOL, None, None, &[(SOL, &["low", "high"], None)]);
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &at(SOL, ""), &[]),
        Some(at(SOL, "low"))
    );
}

#[test]
fn a_route_that_failed_in_the_task_is_stepped_over() {
    let r = models(
        SOL,
        Some("high"),
        Some(OPUS),
        &[(SOL, &["low", "high"], None)],
    );
    // The model failed for an environment reason: no effort step helps it (RL-1).
    assert_eq!(
        escalate(
            &r,
            Role::ImplementerMedium,
            &at(SOL, "low"),
            &[at(SOL, "low")]
        ),
        Some(at(OPUS, ""))
    );
}

#[test]
fn escalation_never_names_a_model_outside_the_row() {
    let r = models(
        SOL,
        Some("low"),
        Some(OPUS),
        &[
            (SOL, &["low", "high"], None),
            (OPUS, &["low", "high"], None),
        ],
    );
    let mut route = at(SOL, "low");
    let mut seen = vec![route.clone()];
    while let Some(next) = escalate(&r, Role::ImplementerMedium, &route, &[]) {
        assert!(
            next.model == "gpt-6-sol" || next.model == "claude-opus-5-5",
            "{next:?}"
        );
        seen.push(next.clone());
        route = next;
    }
    // Opus reports no default effort, so its DEFAULT sits below `low` (decision 29).
    assert_eq!(
        seen,
        [
            at(SOL, "low"),
            at(SOL, "high"),
            at(OPUS, ""),
            at(OPUS, "low"),
            at(OPUS, "high")
        ]
    );
}

#[test]
fn a_route_outside_the_row_climbs_its_own_list_then_takes_the_rows_fallback() {
    // A user's route (decision 10) on Sonnet, which the row names nowhere (Opus is its
    // fallback).
    let r = models(
        SOL,
        Some("medium"),
        Some(OPUS),
        &[("claude:claude-sonnet-5", &["low", "high"], None)],
    );
    let sonnet = |e: &str| at("claude:claude-sonnet-5", e);
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &sonnet("low"), &[]),
        Some(sonnet("high"))
    );
    assert_eq!(
        escalate(&r, Role::ImplementerMedium, &sonnet("high"), &[]),
        Some(at(OPUS, ""))
    );
}
