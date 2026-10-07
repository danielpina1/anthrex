//! Task M9.5.10a, continued from `route_pick_tests.rs` (split for the 600-line rule):
//! the lists are frozen into the run, an old run has none, and no list changes nothing
//! (milestone 9.8 decision 34: the planners see the role table instead).

use super::*;

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn lists_are_frozen_into_the_run() {
    let lists = RouteLists {
        m: m_example(Pick::Spread),
        review: list(Pick::First, vec![cand(Runtime::Codex, LUNA, None)]),
        ..Default::default()
    };
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists.clone());
    let frozen = &run.limits.route_lists;
    assert_eq!(frozen.m.pick, ListPolicy::Spread);
    let strengths: Vec<Strength> = frozen.m.candidates.iter().map(|c| c.strength).collect();
    assert_eq!(strengths, [Strength::Frontier, Strength::Frontier]);
    assert_eq!(frozen.review.candidates[0].strength, Strength::Fast);
    assert!(frozen.s.is_empty() && frozen.scout.is_empty());

    // run.json keeps them; an edit after the config changed uses the run's copy.
    let json = serde_json::to_string(&run).expect("serializes");
    let back: Run = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(back.limits.route_lists, *frozen);
    let edited = add(&back, &[m("t2", "[\"crates/b/**\"]", "")]);
    assert_eq!(task(&edited, "t2").route, opus(Effort::MEDIUM));
}

#[test]
fn an_old_run_json_has_no_lists() {
    let old: Run = serde_json::from_str(include_str!("../../tests/fixtures/run/m93-run.json"))
        .expect("m93-run.json");
    assert!(old.limits.route_lists.is_empty());
    assert!(old.tasks.iter().all(|t| t.list_pick.is_none()));
    let json = serde_json::to_string(&old).expect("serializes");
    for key in ["route_lists", "list_pick", "list_escalation"] {
        assert!(!json.contains(key), "{key}");
    }
}

/// Pinning: with no list, every route is exactly the class resolution and the
/// roster's reviewer, and nothing new is written.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn no_lists_reproduce_today() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
            m("t3", "[\"crates/proto/**\"]", ""),
            m(
                "t4",
                "[\"crates/c/**\"]",
                "[task.route]\nruntime = \"codex\"",
            ),
        ],
    );
    let run = build_tuned(&text, &config(), Tuned::default()).expect("builds");
    let sonnet = |effort| route(Runtime::Claude, SONNET, Strength::Standard, effort);
    let expected = [
        sonnet(Effort::LOW),
        sonnet(Effort::MEDIUM),
        opus(Effort::HIGH),
        route(Runtime::Codex, "", Strength::Standard, Effort::MEDIUM),
    ];
    for (t, want) in run.tasks.iter().zip(&expected) {
        assert_eq!(&t.route, want, "{}", t.id());
        assert_eq!(t.list_pick, None, "{}", t.id());
        let reviewer = t
            .review_level
            .map(|l| pick_reviewer(&run.roster, &t.route, l));
        assert_eq!(t.review_route, reviewer, "{}", t.id());
    }
    let json = serde_json::to_string(&run).expect("serializes");
    assert!(!json.contains("route_lists") && !json.contains("list_pick"));
    // An edit too.
    let edited = add(&run, &[m("t5", "[\"crates/d/**\"]", "")]);
    assert_eq!(task(&edited, "t5").route, sonnet(Effort::MEDIUM));
    assert_eq!(task(&edited, "t5").list_pick, None);
}

/// Targets by id: the edit path's.
#[test]
fn targets_are_the_named_unfinished_tasks() {
    let run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
        ],
        RouteLists::default(),
    );
    let ids: BTreeSet<String> = ["t2".to_string()].into();
    assert_eq!(targets(&run.tasks, &ids), [1].into());
}

/// Rule 9 sees a list's pick as the plan's runtime: a task added on another runtime
/// over a listed task's `owns` is refused, as it would be over a planned one.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn the_overlap_rule_sees_a_lists_runtime() {
    let lists = RouteLists {
        m: list(Pick::First, vec![cand(Runtime::Codex, SOL, None)]),
        ..Default::default()
    };
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], lists);
    assert_eq!(task(&run, "t1").route.runtime, Runtime::Codex);
    let explicit = "[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"";
    let edit = PlanEdit::AddTask {
        task: spec(&m("t2", "[\"crates/a/x/**\"]", explicit)),
    };
    let errors = apply_edits(&run, &[edit], &EditScope::Run, &EditSource::User, 5_000)
        .expect_err("rule 9 refuses it");
    assert!(show(&errors).contains("(rule 9)"), "{}", show(&errors));
}

/// Every candidate of the class list counts as a start route, so its runtimes are
/// reachable (the project-trust and API-key checks cover them). A superset: rung 2's
/// guard (ruling T10a-1) keeps opus from stepping down to luna, but a pick by a later
/// edit could take it.
#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn a_class_lists_runtimes_are_reachable() {
    let mut config = config::Orchestrator::default();
    config.models.push(ModelEntry {
        runtime: Runtime::Codex,
        model: LUNA.into(),
        strength: Strength::Fast,
        note: String::new(),
    });
    let lists = RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, OPUS, Some(Effort::HIGH)),
                cand(Runtime::Codex, LUNA, None),
            ],
        ),
        ..Default::default()
    };
    let text = plan_with(PROFILE, &[m("t1", "[\"crates/a/**\"]", "")]);
    let run = build_tuned(&text, &config, tuned(lists)).expect("builds");
    assert_eq!(task(&run, "t1").route, opus(Effort::HIGH));
    let reachable = crate::run::reach::reachable_runtimes(&run);
    assert_eq!(reachable, [Runtime::Claude, Runtime::Codex]);
}
