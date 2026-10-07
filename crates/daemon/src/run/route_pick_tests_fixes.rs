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
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
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
    assert_eq!(task(&run, "t1").route, sol(Effort::HIGH));
    let (next, step) = rung2_route(&run, 0);
    assert_eq!(next, opus(Effort::HIGH));
    assert_not_down(&sol(Effort::HIGH), &next);
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
    run.tasks[0].route = opus(Effort::MEDIUM);
    let none = Installed::new();
    assert_eq!(next_candidate(&run.limits, &run.tasks, 0, &none), None);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!(
        (next.clone(), step),
        (opus(Effort::HIGH), None),
        "roster::escalate"
    );
    assert_not_down(&opus(Effort::MEDIUM), &next);

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
    assert_eq!(task(&run, "t1").route, opus(Effort::MEDIUM));
    assert_eq!(next_candidate(&run.limits, &run.tasks, 0, &none), None);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (opus(Effort::HIGH), None));
    assert_not_down(&opus(Effort::MEDIUM), &next);

    // A one-candidate list: `roster::escalate`.
    let lists = RouteLists {
        m: list(Pick::First, vec![cand(Runtime::Codex, SOL, None)]),
        ..Default::default()
    };
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists);
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (sol(Effort::HIGH), None));
    assert_not_down(&sol(Effort::MEDIUM), &next);
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
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
    let medium = Effort::MEDIUM;
    assert_eq!(
        got,
        [
            (sol(medium.clone()), Some(0)),
            (opus(medium.clone()), Some(1)),
            (opus(medium.clone()), None),
            (sonnet(medium), Some(2)),
        ]
    );
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
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
    assert_eq!(t1.route, opus(Effort::MEDIUM));
    let pick = t1.list_pick.as_ref().expect("picked again");
    assert_eq!(pick.chosen_route(), Some(&t1.route));
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
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
    assert_eq!(d.chosen, opus(Effort::MEDIUM));
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
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_failed_route_is_substituted_by_the_next_unfailed_candidate() {
    let mut run = substitute_run();
    assert_eq!(task(&run, "t1").route, opus(Effort::MEDIUM));
    // Not failed: never weaker, so gpt-6-sol is below and the roster escalates.
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.model.as_str(), step), (OPUS, None));
    // Failed: gpt-6-sol, standard, is taken.
    run.tasks[0].rounds = vec![failed_round(opus(Effort::MEDIUM))];
    let (next, step) = rung2_route(&run, 0);
    let gpt6 = route(Runtime::Codex, GPT6_SOL, Strength::Standard, Effort::MEDIUM);
    assert_eq!(next, gpt6);
    let step = step.expect("the list's step");
    assert_eq!(reasons(&step.candidates), [Some(CURRENT_ROUTE), None]);
    assert_eq!(every_route_failed(&run, 0, &next), None);
}

/// Ruling T10a-3: with every route failed, the original is retried and the run log
/// says so (also when the roster escalation has nothing left but the failed route).
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn with_every_route_failed_the_original_is_retried_and_said() {
    let mut run = substitute_run();
    let gpt6 = route(Runtime::Codex, GPT6_SOL, Strength::Standard, Effort::MEDIUM);
    run.tasks[0].rounds = vec![
        failed_round(opus(Effort::MEDIUM)),
        failed_round(gpt6.clone()),
    ];
    run.roster
        .retain(|e| [OPUS, GPT6_SOL].contains(&e.model.as_str()));
    let (next, step) = rung2_route(&run, 0);
    assert_eq!((next.clone(), step), (opus(Effort::MEDIUM), None));
    assert_eq!(
        every_route_failed(&run, 0, &next).as_deref(),
        Some("every route for task t1 failed in this task; retrying claude/claude-opus-5-5")
    );
}

/// Ruling FW-3 (re-review N-1), narrowed by FW-5: a worker's group pick exempts no
/// dependent. `x` waits on `t1` alone and overlaps `t2`, so the group {t1, t2} stays off
/// Codex, where `t2` would run beside `x`.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_dependent_of_one_member_still_holds_the_group_by_another() {
    let lists = RouteLists {
        m: m_example(Pick::First),
        ..Default::default()
    };
    let run = built(
        &[
            m("t1", "[\"crates/a/**\", \"crates/s/**\"]", ""),
            m("t2", "[\"crates/s/x/**\", \"crates/b/**\"]", ""),
            m(
                "x",
                "[\"crates/b/y/**\"]",
                "deps = [\"t1\"]\n[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"",
            ),
        ],
        lists,
    );
    for id in ["t1", "t2"] {
        let t = task(&run, id);
        assert_eq!(t.route, opus(Effort::MEDIUM), "{id}");
        let pick = t.list_pick.as_ref().expect("a list pick");
        assert_eq!(reasons(&pick.candidates)[0], Some(OVERLAPPING_OWNS), "{id}");
    }
}

/// Ruling FW-5 (O-1): a worker's route always satisfies validation rule 9, which refuses
/// overlapping tasks on different runtimes whatever their deps. `x` waits on `a`, names
/// Claude and overlaps `a`, so `a` stays off the list's Codex candidate and the build
/// passes.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_dependent_on_claude_holds_its_dependency_off_codex() {
    let lists = RouteLists {
        m: m_example(Pick::First),
        ..Default::default()
    };
    let run = built(
        &[
            m("a", "[\"crates/a/**\"]", ""),
            m(
                "x",
                "[\"crates/a/x/**\"]",
                "deps = [\"a\"]\n[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"",
            ),
        ],
        lists,
    );
    let a = task(&run, "a");
    assert_eq!(a.route, opus(Effort::MEDIUM));
    let pick = a.list_pick.as_ref().expect("a list pick");
    assert_eq!(reasons(&pick.candidates)[0], Some(OVERLAPPING_OWNS));
}

/// Ruling FW-5 (O-1): rung 2's list step for a worker exempts no dependent. `t3` waits
/// on `t1` and overlaps it on Codex, so `t1` keeps to Codex.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_dependent_holds_rung_2s_list_step() {
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], ladder_lists());
    let run = add(&run, &[m("t3", "[\"crates/a/x/**\"]", "deps = [\"t1\"]")]);
    assert_eq!(task(&run, "t3").route.runtime, Runtime::Codex);
    let (next, step) =
        next_candidate(&run.limits, &run.tasks, 0, &Installed::new()).expect("a step");
    assert_eq!(next, sol(Effort::HIGH));
    assert_eq!(
        reasons(&step.candidates),
        [Some(CURRENT_ROUTE), Some(OVERLAPPING_OWNS), None]
    );
}
