//! Milestone 9 task 2 (decision 43): the role-routing history line.

use super::{a_route, both_ways};
use crate::RunPath;
use crate::history::{
    HISTORY_VERSION, HistoryLine, RoleOutcome, RoleRoutingDecision, RoleRoutingInput,
    RoutingCandidate,
};
use crate::run::AgentRole;
use crate::types::Runtime;

fn a_role_decision(role: AgentRole, run_id: Option<&str>) -> RoleRoutingDecision {
    let chosen = a_route(Runtime::Claude, "claude-opus-5");
    RoleRoutingDecision {
        v: HISTORY_VERSION,
        record_id: match run_id {
            Some(run) => format!("{run}/{role:?}/s1"),
            None => "triage/req-9/1".into(),
        },
        at: 1_700_000_500,
        run_id: run_id.map(str::to_string),
        task_id: None,
        role,
        session_id: "0f0e-session".into(),
        trigger: "start".into(),
        source: "agent_config".into(),
        policy_version: "m9-orchestrator-v1".into(),
        pick_policy: None,
        input: RoleRoutingInput {
            run_path: Some(RunPath::Plan),
            goal: Some("Add password reset".into()),
            languages: vec!["rust".into()],
            epic: Some("auth".into()),
            area: vec!["crates/auth/**".into()],
            question_kind: None,
        },
        chosen: chosen.clone(),
        selected_index: 1,
        candidates: vec![
            RoutingCandidate {
                route: a_route(Runtime::Codex, "gpt-5-codex"),
                skipped_reason: Some("not installed".into()),
            },
            RoutingCandidate {
                route: chosen,
                skipped_reason: None,
            },
            RoutingCandidate {
                route: a_route(Runtime::Claude, "claude-sonnet-5"),
                skipped_reason: Some("an earlier candidate was taken".into()),
            },
        ],
        outcome: Some(RoleOutcome::Completed),
        result: Some("submitted".into()),
    }
}

#[test]
fn role_routing_history_round_trip() {
    assert_eq!(HISTORY_VERSION, 4);
    let roles = [
        (AgentRole::Orchestrator, Some("run-a1b2")),
        (AgentRole::Planner, Some("run-a1b2")),
        (AgentRole::Scout, Some("run-a1b2")),
        // Pre-run triage: a decider session, and no run.
        (AgentRole::Decider, None),
    ];
    for (role, run_id) in roles {
        let line = HistoryLine::RoleRoute(a_role_decision(role, run_id));
        let json = serde_json::to_string(&line).unwrap();
        assert!(json.starts_with(r#"{"type":"role_route""#), "{json}");
        both_ways(&line);
    }
    // Candidate order and skip reasons survive; the chosen route is the selected one.
    let decision = a_role_decision(AgentRole::Planner, Some("run-a1b2"));
    let json = serde_json::to_string(&HistoryLine::RoleRoute(decision.clone())).unwrap();
    let HistoryLine::RoleRoute(back) = serde_json::from_str(&json).unwrap() else {
        panic!("a role_route line");
    };
    let reasons: Vec<Option<&str>> = back
        .candidates
        .iter()
        .map(|c| c.skipped_reason.as_deref())
        .collect();
    assert_eq!(
        reasons,
        [
            Some("not installed"),
            None,
            Some("an earlier candidate was taken")
        ]
    );
    assert_eq!(
        back.candidates[back.selected_index as usize].route,
        back.chosen
    );

    // Pre-run triage: no `run_id`, `task_id`, `pick_policy`, `outcome` or `result`
    // keys at all, and the optional input fields absent.
    let mut v = serde_json::to_value(HistoryLine::RoleRoute(a_role_decision(
        AgentRole::Decider,
        None,
    )))
    .unwrap();
    let map = v.as_object_mut().unwrap();
    assert_eq!(map["role"], "decider");
    for key in ["run_id", "task_id", "pick_policy", "outcome", "result"] {
        map.remove(key);
    }
    map.insert("input".into(), serde_json::json!({}));
    let HistoryLine::RoleRoute(sparse) = serde_json::from_value(v).unwrap() else {
        panic!("a role_route line");
    };
    assert_eq!(sparse.run_id, None);
    assert_eq!(sparse.outcome, None);
    assert_eq!(sparse.input, RoleRoutingInput::default());

    for (outcome, text) in [
        (RoleOutcome::Completed, "completed"),
        (RoleOutcome::Failed, "failed"),
        (RoleOutcome::Interrupted, "interrupted"),
        (RoleOutcome::Fallback, "fallback"),
    ] {
        assert_eq!(
            serde_json::to_string(&outcome).unwrap(),
            format!("\"{text}\"")
        );
    }

    // A version-1 line, as milestone 8b wrote it, still decodes.
    let v1 = r#"{"type":"revert","v":1,"record_id":"revert-ffff","at":1700002000,
        "run_id":"run-a1b2","task_id":"t1","reverted":"dddd4444","revert_commit":"ffff6666"}"#;
    let HistoryLine::Revert(old) = serde_json::from_str(v1).unwrap() else {
        panic!("a revert line");
    };
    assert_eq!(old.v, 1);
}

/// Task M9.9: a reported research or review task's outcome, appended last.
#[test]
fn reported_task_outcome_round_trips() {
    use crate::history::TaskOutcome;
    both_ways(&TaskOutcome::Reported);
    assert_eq!(
        serde_json::to_string(&TaskOutcome::Reported).unwrap(),
        "\"reported\""
    );
}
