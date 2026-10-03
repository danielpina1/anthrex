//! Milestone 9.5 task 10a: in the reducer, a worker launched on a model list's pick,
//! rung 2 and the reviewer follow the run's frozen lists (decision 9a), and each
//! records the list in its routing decision.

use config::{Candidate, Pick, RouteList, RouteLists};
use proto::{Effort, ModelEntry, Route, Runtime, Strength, TaskState};

use super::fixture::*;
use super::gates::{CHECK_MODE, accepted, check_result, only_op};
use super::gates_review::in_review;
use crate::run::engine::OpResult;
use crate::run::refit::Tuned;
use crate::run::route_pick::{
    AUTHOR_RUNTIME, BELOW_STRENGTH, CURRENT_ROUTE, FIRST_QUALIFYING, LIST_POLICY,
};

const SOL: &str = "gpt-6.1-sol";

fn cand(runtime: Runtime, model: &str, effort: Option<Effort>) -> Candidate {
    Candidate {
        runtime,
        model: model.into(),
        effort,
    }
}

/// `s` = [codex/gpt-6.1-sol low, claude/claude-opus-5-5 medium]; `review` =
/// [claude/claude-haiku-4-5, claude/claude-opus-5-5 high, codex/gpt-6.1-sol].
fn lists() -> RouteLists {
    RouteLists {
        s: RouteList {
            candidates: vec![
                cand(Runtime::Codex, SOL, Some(Effort::Low)),
                cand(Runtime::Claude, "claude-opus-5-5", Some(Effort::Medium)),
            ],
            pick: Pick::First,
        },
        review: RouteList {
            candidates: vec![
                cand(Runtime::Claude, "claude-haiku-4-5", None),
                cand(Runtime::Claude, "claude-opus-5-5", Some(Effort::High)),
                cand(Runtime::Codex, SOL, None),
            ],
            pick: Pick::First,
        },
        ..Default::default()
    }
}

/// A working check-mode `t1` (S, `crates/a/**`) routed by [`lists`]; its window.
fn working() -> (Fixture, u32) {
    let mut config = config::Orchestrator::default();
    config.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: SOL.into(),
        strength: Strength::Frontier,
        note: String::new(),
    });
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut fx = Fixture::with_config(&plan, config);
    fx.tuning = Tuned {
        lists: lists(),
        ..Tuned::default()
    };
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    (fx, window)
}

fn sol_low() -> Route {
    Route {
        runtime: Runtime::Codex,
        model: SOL.into(),
        strength: Strength::Frontier,
        effort: Effort::Low,
    }
}

#[test]
fn a_listed_worker_records_its_list_and_rung_2_takes_the_next_candidate() {
    let (mut fx, window) = working();
    let t1 = fx.task("t1");
    assert_eq!(t1.route, sol_low());
    let d = &t1.routing_decisions[0];
    assert_eq!(
        (
            d.trigger.as_str(),
            d.source.as_str(),
            d.policy_version.as_str()
        ),
        ("initial", "configured_list", LIST_POLICY)
    );
    assert_eq!(
        (d.selected_index, d.pick_policy.as_deref()),
        (0, Some("first"))
    );

    // Two failed checks: rung 2, a fresh session on the list's next candidate.
    for _ in 0..2 {
        let effects = accepted(&mut fx, window, serde_json::json!({"summary": "s"}));
        let (op, _) = only_op(&effects, "Check");
        fx.done(op, check_result(false));
    }
    assert_eq!(fx.task("t1").rung, 2);
    let next = Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        strength: Strength::Frontier,
        effort: Effort::Medium,
    };
    assert_eq!(fx.task("t1").route, next);
    let effects = super::turns::killed_exit(&mut fx, window);
    let (op, _) = only_op(&effects, "DiffSoFar");
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(t1.list_escalation, None, "the launch took it");
    let d = &t1.routing_decisions[1];
    assert_eq!(
        (
            d.trigger.as_str(),
            d.source.as_str(),
            d.policy_version.as_str()
        ),
        ("escalation", "configured_list", LIST_POLICY)
    );
    assert_eq!((d.selected_index, &d.chosen), (1, &next));
    assert_eq!(
        d.candidates[0].skipped_reason.as_deref(),
        Some(CURRENT_ROUTE)
    );
}

#[test]
fn the_reviewer_comes_from_the_review_list_and_records_it() {
    let (mut fx, window) = working();
    let (op, _) = in_review(&mut fx, window);
    let effects = fx.done(
        op,
        OpResult::Review {
            base: BASE.into(),
            head: HEAD.into(),
            patch: "diff".into(),
        },
    );
    let opus = Route {
        runtime: Runtime::Claude,
        model: "claude-opus-5-5".into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    };
    let (_, kind) = only_op(&effects, "CreateWindow");
    assert!(format!("{kind:?}").contains("claude-opus-5-5"), "{kind:?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.review_route.as_ref(), Some(&opus));
    let d = t1.routing_decisions.last().expect("the reviewer's");
    assert_eq!(
        (
            d.trigger.as_str(),
            d.source.as_str(),
            d.policy_version.as_str()
        ),
        ("review", "configured_list", LIST_POLICY)
    );
    assert_eq!(d.pick_policy.as_deref(), Some(FIRST_QUALIFYING));
    assert_eq!((d.selected_index, &d.chosen), (1, &opus));
    let reasons: Vec<Option<&str>> = (d.candidates.iter())
        .map(|c| c.skipped_reason.as_deref())
        .collect();
    assert_eq!(reasons, [Some(BELOW_STRENGTH), None, Some(AUTHOR_RUNTIME)]);

    // A later edit never changes an earlier decision.
    let decisions = t1.routing_decisions.clone();
    let add = proto::PlanEdit::AddTask {
        task: crate::run::plan::parse_plan(&plan_with(PROFILE, &[task("t2", "S", "b", "")]))
            .expect("parses")
            .tasks
            .remove(0),
    };
    super::dispatch::edit(&mut fx, vec![add]);
    assert_eq!(fx.task("t1").routing_decisions, decisions);
}

/// Review m1: a decider's S → M raise keeps a list candidate's own effort, so the
/// launch still records the list's choice.
#[test]
fn a_decider_raise_keeps_a_listed_effort() {
    let lists = RouteLists {
        s: RouteList {
            candidates: vec![cand(
                Runtime::Claude,
                "claude-haiku-4-5",
                Some(Effort::High),
            )],
            pick: Pick::First,
        },
        ..Default::default()
    };
    let tuning = Tuned {
        lists,
        ..Tuned::default()
    };
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", CHECK_MODE)]);
    let mut run = build_tuned(&plan, &config::Orchestrator::default(), true, tuning);
    let listed = run.tasks[0].route.clone();
    assert_eq!(
        (listed.model.as_str(), listed.effort),
        ("claude-haiku-4-5", Effort::High)
    );
    super::super::deciders_size::apply_raise(&mut run, "t1", proto::Size::M, "evidence");
    let t1 = &run.tasks[0];
    assert_eq!(t1.size, proto::Size::M);
    assert_eq!(t1.route, listed);
    let pick = t1.list_pick.as_ref().expect("the list's pick");
    assert_eq!(pick.chosen_route(), Some(&t1.route));
}
