//! Task M9.5.10a fix round 1: rung 2 finds the current candidate by runtime and model
//! and never steps down (ruling T10a-1), a runtime-only route under `spread` takes its
//! runtime's first candidate and no slot (T10a-2), a class change on an amend re-picks
//! (review m2), and a worker's decision records `not installed` (m4). Fix round 2: a
//! route that failed in this task is substituted, whatever the strength (T10a-3).

use super::*;
use crate::run::route_pick::{every_route_failed, rung2_route};

fn sonnet(effort: Effort) -> Route {
    route(Runtime::Claude, SONNET, Strength::Standard, effort)
}

/// Rung 2 is an escalation: never a weaker strength, nor a lower effort at the same one.
fn assert_not_down(from: &Route, to: &Route) {
    let up =
        to.strength > from.strength || (to.strength == from.strength && to.effort >= from.effort);
    assert!(up, "{from:?} stepped down to {to:?}");
}

#[test]
fn rung_2_never_steps_down() {
    // A listed route whose plan names an effort: found by runtime and model.
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Codex, SOL, None),
                cand(Runtime::Claude, OPUS, None),
            ],
        ),
        ..Default::default()
    };
    let run = built(
        &[m(
            "t1",
            "[\"crates/a/**\"]",
            "[task.route]\neffort = \"high\"",
        )],
        lists,
    );
    assert_eq!(task(&run, "t1").route, sol(Effort::High));
    let (next, step) = rung2_route(&run, 0);
    assert_eq!(next, opus(Effort::High));
    assert_not_down(&sol(Effort::High), &next);
    let step = step.expect("the list's step");
    assert_eq!(reasons(&step.candidates)[0], Some(CURRENT_ROUTE));

    // A listed route at the end of its list does not cycle to a weaker candidate.
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, SONNET, None),
                cand(Runtime::Claude, OPUS, None),
            ],
        ),
        ..Default::default()
    };
    let mut run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists);
    run.tasks[0].route = opus(Effort::Medium);
    let none = Installed::new();
    assert_eq!(next_candidate(&run.limits, &run.tasks, 0, &none), None);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!(
        (next.clone(), step),
        (opus(Effort::High), None),
        "roster::escalate"
    );
    assert_not_down(&opus(Effort::Medium), &next);

    // An explicit route the list does not hold: `roster::escalate`, never the first
    // candidate.
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, SONNET, None),
                cand(Runtime::Codex, SOL, None),
            ],
        ),
        ..Default::default()
    };
    let explicit = "[task.route]\nruntime = \"claude\"\nmodel = \"claude-opus-5-5\"";
    let run = built(&[m("t1", "[\"crates/a/**\"]", explicit)], lists);
    assert_eq!(task(&run, "t1").route, opus(Effort::Medium));
    assert_eq!(next_candidate(&run.limits, &run.tasks, 0, &none), None);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (opus(Effort::High), None));
    assert_not_down(&opus(Effort::Medium), &next);

    // A one-candidate list: `roster::escalate`.
    let lists = RouteLists {
        m: list(Pick::First, vec![cand(Runtime::Codex, SOL, None)]),
        ..Default::default()
    };
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (sol(Effort::High), None));
    assert_not_down(&sol(Effort::Medium), &next);
}

#[test]
fn spread_gives_a_runtime_only_route_its_runtimes_first_candidate_and_no_slot() {
    let lists = RouteLists {
        m: list(
            Pick::Spread,
            vec![
                cand(Runtime::Codex, SOL, None),
                cand(Runtime::Claude, OPUS, None),
                cand(Runtime::Claude, SONNET, None),
            ],
        ),
        ..Default::default()
    };
    let run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
            m(
                "t3",
                "[\"crates/c/**\"]",
                "[task.route]\nruntime = \"claude\"",
            ),
            m("t4", "[\"crates/d/**\"]", ""),
        ],
        lists,
    );
    let got: Vec<(Route, Option<u32>)> = ["t1", "t2", "t3", "t4"]
        .iter()
        .map(|id| {
            let t = task(&run, id);
            (t.route.clone(), t.list_pick.as_ref().and_then(|p| p.slot))
        })
        .collect();
    let medium = Effort::Medium;
    assert_eq!(
        got,
        [
            (sol(medium), Some(0)),
            (opus(medium), Some(1)),
            (opus(medium), None),
            (sonnet(medium), Some(2)),
        ]
    );
}

#[test]
fn an_amend_that_changes_the_class_picks_from_the_new_classs_list() {
    let lists = RouteLists {
        s: list(Pick::First, vec![cand(Runtime::Codex, LUNA, None)]),
        m: list(Pick::First, vec![cand(Runtime::Claude, OPUS, None)]),
        ..Default::default()
    };
    let run = built(
        &[task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", "")],
        lists,
    );
    assert_eq!(task(&run, "t1").route.model, LUNA);
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: Some(proto::Size::M),
        deps: None,
        stage: None,
        race: None,
        pair: None,
    };
    let amended = apply_edits(&run, &[amend], &EditScope::Run, &EditSource::User, 6_000)
        .unwrap_or_else(|e| panic!("{}", show(&e)))
        .0;
    let t1 = task(&amended, "t1");
    assert_eq!(t1.route, opus(Effort::Medium));
    let pick = t1.list_pick.as_ref().expect("picked again");
    assert_eq!(pick.chosen_route(), Some(&t1.route));
}

#[test]
fn a_workers_decision_records_a_candidate_not_installed() {
    let lists = RouteLists {
        m: m_example(Pick::First),
        ..Default::default()
    };
    let mut run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists);
    run.orch.installed = installed(true, false);
    let mut run = add(&run, &[m("t2", "[\"crates/b/**\"]", "")]);
    let i = (run.tasks.iter()).position(|t| t.id() == "t2").expect("t2");
    run.tasks[i].session = 1;
    record_worker(&mut run, i, 100);
    let d = &task(&run, "t2").routing_decisions[0];
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        ("configured_list", LIST_POLICY)
    );
    assert_eq!(reasons(&d.candidates), [Some(NOT_INSTALLED), None]);
    assert_eq!(d.chosen, opus(Effort::Medium));
}

/// Fix round 2: an ended worker round on `route`, failed for an environment reason.
fn failed_round(route: Route) -> crate::run::model::AgentRound {
    let mut r = crate::run::orch::test_support::round(1, 0, Default::default());
    r.role = AgentRole::Worker;
    r.route = route;
    r.ended = true;
    r.environment_failed = true;
    r
}

const GPT6_SOL: &str = "gpt-6-sol";

/// [opus, codex/gpt-6-sol (standard)], with the roster holding gpt-6-sol.
fn substitute_run() -> Run {
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, OPUS, None),
                cand(Runtime::Codex, GPT6_SOL, None),
            ],
        ),
        ..Default::default()
    };
    let mut config = config();
    config.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: GPT6_SOL.into(),
        strength: Strength::Standard,
        note: String::new(),
    });
    let text = plan_with(PROFILE, &[m("t1", "[\"crates/a/**\"]", "")]);
    build_tuned(&text, &config, tuned(lists)).unwrap_or_else(|e| panic!("{}", show(&e)))
}

/// Ruling T10a-3: a route that failed in this task is substituted, not escalated: the
/// next list candidate that has not failed, whatever its strength.
#[test]
fn a_failed_route_is_substituted_by_the_next_unfailed_candidate() {
    let mut run = substitute_run();
    assert_eq!(task(&run, "t1").route, opus(Effort::Medium));
    // Not failed: never weaker, so gpt-6-sol is below and the roster escalates.
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.model.as_str(), step), (OPUS, None));
    // Failed: gpt-6-sol, standard, is taken.
    run.tasks[0].rounds = vec![failed_round(opus(Effort::Medium))];
    let (next, step) = rung2_route(&run, 0);
    let gpt6 = route(Runtime::Codex, GPT6_SOL, Strength::Standard, Effort::Medium);
    assert_eq!(next, gpt6);
    let step = step.expect("the list's step");
    assert_eq!(reasons(&step.candidates), [Some(CURRENT_ROUTE), None]);
    assert_eq!(every_route_failed(&run, 0, &next), None);
}

/// Ruling T10a-3: with every route failed, the original is retried and the run log
/// says so (also when the roster escalation has nothing left but the failed route).
#[test]
fn with_every_route_failed_the_original_is_retried_and_said() {
    let mut run = substitute_run();
    let gpt6 = route(Runtime::Codex, GPT6_SOL, Strength::Standard, Effort::Medium);
    run.tasks[0].rounds = vec![
        failed_round(opus(Effort::Medium)),
        failed_round(gpt6.clone()),
    ];
    run.roster
        .retain(|e| [OPUS, GPT6_SOL].contains(&e.model.as_str()));
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (opus(Effort::Medium), None));
    assert_eq!(
        every_route_failed(&run, 0, &next).as_deref(),
        Some("every route for task t1 failed in this task; retrying claude/claude-opus-5-5")
    );
}

/// Ruling T10a-5: with no list (and for an explicit route the list does not hold), a
/// route that failed in this task is substituted from the roster: the peer runtime at
/// the same strength, else the strongest route left (the peer runtime first on a tie);
/// only when every route has failed is the original retried, and said.
#[test]
fn with_no_list_a_failed_route_is_substituted_from_the_roster() {
    let mut run = built(&[m("t1", "[\"crates/a/**\"]", "")], RouteLists::default());
    run.tasks[0].route = opus(Effort::Medium);
    // Not failed: rung 2 escalates, never weaker.
    assert_eq!(rung2_route(&run, 0), (opus(Effort::High), None));
    // Failed: the peer runtime at the same strength.
    run.tasks[0].rounds = vec![failed_round(opus(Effort::Medium))];
    assert_eq!(rung2_route(&run, 0), (sol(Effort::High), None));
    // That failed too: the strongest left, the peer runtime's (Codex's default) first.
    run.tasks[0].rounds.push(failed_round(sol(Effort::High)));
    let codex_default = route(Runtime::Codex, "", Strength::Standard, Effort::High);
    let (next, _) = rung2_route(&run, 0);
    assert_eq!(next, codex_default);
    assert_eq!(every_route_failed(&run, 0, &next), None);
    // Every roster route failed: the original, said.
    for e in run.roster.clone() {
        let r = route(e.runtime, &e.model, e.strength, Effort::Low);
        run.tasks[0].rounds.push(failed_round(r));
    }
    let (next, _) = rung2_route(&run, 0);
    assert_eq!(next, opus(Effort::Medium));
    assert_eq!(
        every_route_failed(&run, 0, &next).as_deref(),
        Some("every route for task t1 failed in this task; retrying claude/claude-opus-5-5")
    );
}

/// Ruling T10a-5: an explicit route the list does not hold is substituted from the
/// roster too.
#[test]
fn a_failed_unlisted_explicit_route_is_substituted_from_the_roster() {
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, SONNET, None),
                cand(Runtime::Codex, LUNA, None),
            ],
        ),
        ..Default::default()
    };
    let extra = "[task.route]\nruntime = \"claude\"\nmodel = \"claude-opus-5-5\"";
    let mut run = built(&[m("t1", "[\"crates/a/**\"]", extra)], lists);
    assert_eq!(task(&run, "t1").route.model, OPUS);
    run.tasks[0].rounds = vec![failed_round(task(&run, "t1").route.clone())];
    assert_eq!(rung2_route(&run, 0), (sol(Effort::High), None));
}
