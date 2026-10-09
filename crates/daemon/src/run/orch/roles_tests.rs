//! Milestone 9 task M9.13b (decision 43): a role-routing record as dispatch makes it.

use proto::{
    AgentRole, Effort, OrchestratorChoice, RoleOutcome, RoleRoutingInput, Route, RoutingCandidate,
    RunPath, Runtime,
};

use super::*;
use crate::run::orch::launch::orchestrator_route;
use crate::run::orch::test_support::run_of;

fn route(model: &str) -> Route {
    Route {
        runtime: Runtime::Claude,
        model: model.into(),
        effort: Effort::HIGH,
    }
}

fn candidate(model: &str) -> RoutingCandidate {
    RoutingCandidate {
        route: route(model),
        skipped_reason: None,
    }
}

/// The orchestrator's record from its resolution for `choice` (milestone 9.8: the row).
fn orchestrator_of(choice: Option<&OrchestratorChoice>) -> RoleRoutingDecision {
    let mut run = run_of(1);
    run.path = Some(RunPath::Plan);
    let resolved = orchestrator_route(choice, run.limits.models());
    let mut o = crate::run::orch::OrchestratorRecord::new(resolved.route, 1_000);
    o.routing = RoleSnapshot {
        source: resolved.source,
        candidates: resolved.candidates,
    };
    run.orch.orchestrator = Some(o);
    orchestrator_record(&run, "start", 2_000).unwrap()
}

#[test]
fn record_appends_the_chosen_route_when_absent() {
    let run = run_of(1);
    let listed = vec![candidate("claude-haiku-4-5"), candidate("claude-sonnet-5")];
    let chosen = route("claude-opus-5");
    let d = record(
        Some(&run),
        AgentRole::Planner,
        "mail/1",
        "start",
        "planner_config",
        PLANNER_POLICY,
        RoleRoutingInput::default(),
        listed.clone(),
        &chosen,
        2_000,
    );
    assert_eq!(d.candidates.len(), 3, "{:#?}", d.candidates);
    assert_eq!(d.selected_index, 2);
    assert_eq!(d.candidates[2].route, chosen);
    assert_eq!(d.candidates[2].skipped_reason, None);
    // The listed ones keep their order.
    assert_eq!(d.candidates[0].route, listed[0].route);
    assert_eq!(d.candidates[1].route, listed[1].route);
    // A listed chosen route is not appended again.
    let d = record(
        Some(&run),
        AgentRole::Planner,
        "mail/1",
        "start",
        "planner_config",
        PLANNER_POLICY,
        RoleRoutingInput::default(),
        listed.clone(),
        &listed[1].route,
        2_000,
    );
    assert_eq!((d.candidates.len(), d.selected_index), (2, 1));
    assert_eq!(d.candidates[d.selected_index as usize].route, d.chosen);
    assert_eq!((d.outcome, d.result), (None, None), "open at dispatch");
}

#[test]
fn explicit_choice_is_identifiable_as_explicit() {
    let default = orchestrator_of(None);
    assert_eq!(default.source, "role_table");
    // The user's choice names the same route the default would have taken; the record
    // still says it was the user's.
    let same = OrchestratorChoice {
        runtime: Runtime::Claude,
        model: Some(default.chosen.model.clone()),
        effort: None,
    };
    let explicit = orchestrator_of(Some(&same));
    assert_eq!(explicit.chosen, default.chosen);
    assert_eq!(explicit.source, "explicit_choice");
    assert_eq!(explicit.policy_version, crate::run::routing::ROLES_POLICY);
    assert_eq!(explicit.role, AgentRole::Orchestrator);
}

#[test]
fn unchosen_candidates_have_no_failure_label() {
    let allowed = [NOT_INSTALLED, NOT_CONFIGURED, EARLIER_TAKEN];
    let failure = ["fail", "error", "reject", "bad", "lost"];
    let run = run_of(1);
    let mut records = vec![orchestrator_of(None)];
    let named = OrchestratorChoice {
        runtime: Runtime::Claude,
        model: Some("claude-sonnet-5".into()),
        effort: None,
    };
    records.push(orchestrator_of(Some(&named)));
    let reviewer = run.limits.models().choice(proto::models::Role::Reviewer);
    let ladder = row_candidates(reviewer);
    let chosen = ladder[1].route.clone();
    records.push(record(
        Some(&run),
        AgentRole::Scout,
        "3f9a-api",
        "start",
        "scout_config",
        SCOUT_POLICY,
        input_of(&run),
        ladder,
        &chosen,
        2_000,
    ));
    // A caller's candidate given before the chosen one with no reason.
    records.push(record(
        Some(&run),
        AgentRole::Planner,
        "mail/1",
        "start",
        "planner_config",
        PLANNER_POLICY,
        input_of(&run),
        vec![candidate("a"), candidate("b")],
        &route("b"),
        2_000,
    ));
    // Milestone 9.8: an orchestrator's snapshot is its chosen route alone.
    for d in &records {
        assert!(!d.candidates.is_empty(), "{d:#?}");
        for (i, c) in d.candidates.iter().enumerate() {
            if i == d.selected_index as usize {
                assert_eq!(c.skipped_reason, None);
                continue;
            }
            let reason = c.skipped_reason.as_deref().expect("a reason");
            assert!(allowed.contains(&reason), "{reason}");
            assert!(
                !failure.iter().any(|w| reason.contains(w)),
                "{reason} reads as a failure"
            );
        }
    }
}

#[test]
fn record_ids_are_stable() {
    let run = run_of(1);
    let make = |role, session: &str| {
        record(
            Some(&run),
            role,
            session,
            "start",
            "planner_config",
            PLANNER_POLICY,
            input_of(&run),
            Vec::new(),
            &route("m"),
            2_000,
        )
    };
    let a = make(AgentRole::Planner, "mail/2");
    assert_eq!(a.record_id, format!("{}/planner/mail/2", run.id));
    assert_eq!(a, make(AgentRole::Planner, "mail/2"), "the same dispatch");
    assert_eq!(
        a.record_id,
        record_id(&run.id, AgentRole::Planner, "mail/2")
    );
    let orchestrator = make(AgentRole::Orchestrator, "1").record_id;
    assert_eq!(orchestrator, format!("{}/orchestrator/1", run.id));
    // Pre-run triage has no run: its id is namespaced so no run's or task's record
    // (`<run>/<task>`) can take it.
    let triage = decider_record(
        None,
        ("17/3", "triage"),
        &[],
        (&route("m"), Vec::new()),
        RoleRoutingInput::default(),
        2_000,
    );
    assert_eq!(triage.record_id, "triage/17/3");
    assert_eq!(triage.run_id, None);
    assert_eq!(triage.input.question_kind.as_deref(), Some("triage"));
    let task = crate::run::history::task_record_id(&run.id, "t0");
    assert_ne!(task, orchestrator);
    // Finishing is once.
    let mut done = triage.clone();
    finish(&mut done, RoleOutcome::Fallback, Some("timed out".into()));
    finish(&mut done, RoleOutcome::Completed, Some("answered".into()));
    assert_eq!(done.outcome, Some(RoleOutcome::Fallback));
    assert_eq!(done.result.as_deref(), Some("timed out"));
}

/// Review M-4: a decider call about one task names it; a call about several (a batch
/// size check) is run-level and names none rather than only the first.
#[test]
fn a_decider_record_names_its_task_only_when_it_has_one() {
    let run = run_of(2);
    let make = |tasks: &[String]| {
        let chosen = (&route("m"), Vec::new());
        decider_record(
            Some(&run),
            ("9", "size_check"),
            tasks,
            chosen,
            input_of(&run),
            1,
        )
    };
    assert_eq!(make(&["t0".into()]).task_id.as_deref(), Some("t0"));
    assert_eq!(make(&["t0".into(), "t1".into()]).task_id, None);
    assert_eq!(make(&[]).task_id, None);
    assert_eq!(make(&["t0".into(), "t1".into()]).trigger, "size_check");
}

/// Review M-5 (b): which reason goes to which unchosen candidate. Before the chosen
/// one: not in the configured list; after it: an earlier candidate was taken; a
/// caller's own reason is kept; the chosen one has none.
#[test]
fn each_unchosen_candidate_gets_the_reason_for_its_place() {
    let run = run_of(1);
    let mut listed = vec![
        candidate("a"),
        candidate("b"),
        candidate("c"),
        candidate("d"),
    ];
    listed[3].skipped_reason = Some(NOT_INSTALLED.into());
    let reasons = |chosen: &Route| {
        let d = record(
            Some(&run),
            AgentRole::Planner,
            "e/1",
            "start",
            "planner_config",
            PLANNER_POLICY,
            RoleRoutingInput::default(),
            listed.clone(),
            chosen,
            1,
        );
        d.candidates
            .into_iter()
            .map(|c| c.skipped_reason)
            .collect::<Vec<_>>()
    };
    let s = |r: &str| Some(r.to_string());
    assert_eq!(
        reasons(&route("b")),
        vec![s(NOT_CONFIGURED), None, s(EARLIER_TAKEN), s(NOT_INSTALLED)]
    );
    // A chosen route absent from the list is appended; every listed one came first.
    assert_eq!(
        reasons(&route("z")),
        vec![
            s(NOT_CONFIGURED),
            s(NOT_CONFIGURED),
            s(NOT_CONFIGURED),
            s(NOT_INSTALLED),
            None
        ]
    );
}
