//! Task M9.5.10b, continued from `route_pick_tests.rs` (split for the 600-line rule):
//! the failed-in-this-task skip (ruling RL-1) at rung 2, `run retry` and the reviewer,
//! with and without lists; the skips in the routing history; research and review tasks
//! by the role lists (RL-4); and a role list's pick and rotation.

use super::*;
use crate::run::model::AgentRound;
use crate::run::roster::{escalate, escalate_skipping, pick_reviewer_skipping};
use crate::run::routing::{record_listed_reviewer, record_reviewer};

/// A round of `role` on `route`, ended; `failed` when it ended for an environment reason.
fn round(role: AgentRole, route: Route, failed: bool) -> AgentRound {
    let mut r = crate::run::orch::test_support::round(1, 0, Default::default());
    r.role = role;
    r.route = route;
    r.ended = true;
    r.environment_failed = failed;
    r
}

fn haiku(effort: Effort) -> Route {
    route(Runtime::Claude, HAIKU, Strength::Fast, effort)
}

fn sonnet(effort: Effort) -> Route {
    route(Runtime::Claude, SONNET, Strength::Standard, effort)
}

fn luna(effort: Effort) -> Route {
    route(Runtime::Codex, LUNA, Strength::Fast, effort)
}

fn frozen(lists: RouteLists) -> RouteListsFrozen {
    RouteListsFrozen::freeze(&lists, &config().models)
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn retry_skips_the_route_that_just_failed_in_this_task() {
    let none = Installed::new();
    // The reviewer from the list: haiku's session ended blocked(environment).
    let lists = frozen(RouteLists {
        review: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, HAIKU, None),
                cand(Runtime::Claude, SONNET, None),
            ],
        ),
        ..Default::default()
    });
    let author = luna(Effort::MEDIUM);
    let failed = [haiku(Effort::LOW)];
    let (chosen, candidates) =
        reviewer(&lists, &author, ReviewLevel::Small, &none, &failed).expect("a list");
    assert_eq!(chosen, Some(sonnet(Effort::LOW)));
    assert_eq!(reasons(&candidates), [Some(FAILED_IN_TASK), None]);
    // With no list, `pick_reviewer`'s roster pick excludes it likewise.
    let roster = config().models;
    assert_eq!(
        pick_reviewer(&roster, &author, ReviewLevel::Small),
        haiku(Effort::LOW)
    );
    assert_eq!(
        pick_reviewer_skipping(&roster, &author, ReviewLevel::Small, &failed),
        sonnet(Effort::LOW)
    );
    assert_eq!(
        pick_reviewer_skipping(&roster, &author, ReviewLevel::Small, &[]),
        haiku(Effort::LOW)
    );

    // Rung 2 (and `run retry`, which takes the same route): opus failed in t1.
    let mut run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
        ],
        ladder_lists(),
    );
    run.tasks[0].rounds = vec![round(AgentRole::Worker, opus(Effort::HIGH), true)];
    assert_eq!(failed_routes(&run.tasks[0]), [opus(Effort::HIGH)]);
    let (next, step) = next_candidate(&run.limits, &run.tasks, 0, &none).expect("a step");
    assert_eq!(next, sol(Effort::HIGH));
    assert_eq!(
        reasons(&step.candidates),
        [Some(CURRENT_ROUTE), Some(FAILED_IN_TASK), None]
    );
    // A route that failed in another task is not skipped.
    let mut other = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
        ],
        ladder_lists(),
    );
    other.tasks[1].rounds = vec![round(AgentRole::Worker, opus(Effort::HIGH), true)];
    let (next, _) = next_candidate(&other.limits, &other.tasks, 0, &none).expect("a step");
    assert_eq!(next, opus(Effort::HIGH));
    // A session that ended for another reason is no failure.
    other.tasks[0].rounds = vec![round(AgentRole::Reviewer, opus(Effort::HIGH), false)];
    assert!(failed_routes(&other.tasks[0]).is_empty());

    // With no list, `roster::escalate` excludes it: sonnet high would step to Codex's
    // default, which failed, so it steps up to opus.
    let codex_default = route(Runtime::Codex, "", Strength::Standard, Effort::MEDIUM);
    assert_eq!(
        escalate(&roster, &sonnet(Effort::HIGH)).runtime,
        Runtime::Codex
    );
    assert_eq!(
        escalate_skipping(&roster, &sonnet(Effort::HIGH), &[codex_default]),
        opus(Effort::HIGH)
    );
    // A route that failed itself is not given more effort: it steps as a `high` one.
    let codex_high = route(Runtime::Codex, "", Strength::Standard, Effort::HIGH);
    assert_eq!(
        escalate_skipping(&roster, &sonnet(Effort::LOW), &[sonnet(Effort::LOW)]),
        codex_high
    );
    assert_eq!(
        escalate_skipping(&roster, &sonnet(Effort::LOW), &[]),
        sonnet(Effort::MEDIUM)
    );
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn escalation_and_review_history_keep_skips() {
    // Rung 2's list step: the current route, an overlap and a failure.
    let mut run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m(
                "t2",
                "[\"crates/a/x/**\"]",
                "[task.route]\nruntime = \"codex\"\nmodel = \"gpt-6.1-sol\"",
            ),
        ],
        RouteLists {
            m: list(
                Pick::First,
                vec![
                    cand(Runtime::Codex, SOL, Some(Effort::MEDIUM)),
                    cand(Runtime::Claude, OPUS, Some(Effort::HIGH)),
                    cand(Runtime::Codex, LUNA, None),
                    cand(Runtime::Codex, SOL, Some(Effort::HIGH)),
                ],
            ),
            ..Default::default()
        },
    );
    // Ruling FW-5: for a worker, an unfinished overlapping `t2` holds `t1` off its runtime
    // whatever the dependencies between them.
    assert_eq!(run.tasks[1].implicit_deps, ["t1"]);
    run.history = true;
    run.tasks[0].rounds = vec![round(AgentRole::Worker, luna(Effort::MEDIUM), true)];
    run.tasks[0].session = 1;
    record_worker(&mut run, 0, 100);
    let (next, step) =
        next_candidate(&run.limits, &run.tasks, 0, &run.orch.installed).expect("a step");
    assert_eq!(next, sol(Effort::HIGH));
    let from = std::mem::replace(&mut run.tasks[0].route, next);
    run.tasks[0].escalated_from = Some(from);
    run.tasks[0].list_escalation = Some(step);
    run.tasks[0].session = 2;
    record_worker(&mut run, 0, 200);
    let d = task(&run, "t1")
        .routing_decisions
        .last()
        .expect("rung 2's")
        .clone();
    assert_eq!(d.trigger, "escalation");
    assert_eq!(
        reasons(&d.candidates),
        [
            Some(CURRENT_ROUTE),
            Some(OVERLAPPING_OWNS),
            Some(FAILED_IN_TASK),
            None
        ]
    );

    // The reviewer's list: below the author's strength, not installed, failed.
    run.orch.installed = installed(true, false);
    run.limits.route_lists.review = frozen(RouteLists {
        review: list(
            Pick::First,
            vec![
                cand(Runtime::Claude, HAIKU, None),
                cand(Runtime::Codex, LUNA, None),
                cand(Runtime::Claude, OPUS, None),
            ],
        ),
        ..Default::default()
    })
    .review;
    run.tasks[0]
        .rounds
        .push(round(AgentRole::Reviewer, opus(Effort::MEDIUM), true));
    let failed = failed_routes(&run.tasks[0]);
    let author = sol(Effort::HIGH);
    let lists = run.limits.route_lists.clone();
    let (chosen, list_snapshot) = reviewer(
        &lists,
        &author,
        ReviewLevel::Medium,
        &run.orch.installed,
        &failed,
    )
    .expect("a list");
    assert_eq!(chosen, None, "none qualifies");
    let fallback = pick_reviewer_skipping(&run.roster, &author, ReviewLevel::Medium, &failed);
    assert_ne!(fallback.model, OPUS, "the failed route is excluded");
    record_listed_reviewer(&mut run, 0, list_snapshot, &fallback, 1, 300);
    let d = task(&run, "t1")
        .routing_decisions
        .last()
        .expect("the reviewer's")
        .clone();
    let got = reasons(&d.candidates);
    assert_eq!(
        got[..3],
        [
            Some(BELOW_STRENGTH),
            Some(NOT_INSTALLED),
            Some(FAILED_IN_TASK)
        ]
    );

    // With no list, the roster pools record it too.
    record_reviewer(
        &mut run,
        0,
        (&author, ReviewLevel::Medium),
        &fallback,
        2,
        400,
    );
    let d = task(&run, "t1")
        .routing_decisions
        .last()
        .expect("the pool's")
        .clone();
    let opus_reason = (d.candidates.iter())
        .find(|c| c.route.model == OPUS)
        .and_then(|c| c.skipped_reason.as_deref());
    assert_eq!(opus_reason, Some(FAILED_IN_TASK));
    // A chosen route on the failed model, built here at another effort (as a fallback
    // to it would be): the record never calls its own choice's model skipped for it
    // (review 10b, minor 2).
    let pooled = (d.candidates.iter())
        .find(|c| c.route.model == OPUS)
        .map(|c| c.route.clone())
        .expect("opus is pooled");
    let effort = if pooled.effort == Effort::HIGH {
        Effort::LOW
    } else {
        Effort::HIGH
    };
    let fell_back = route(pooled.runtime, OPUS, pooled.strength, effort);
    record_reviewer(
        &mut run,
        0,
        (&author, ReviewLevel::Medium),
        &fell_back,
        3,
        450,
    );
    let d = task(&run, "t1")
        .routing_decisions
        .last()
        .expect("it")
        .clone();
    let opus_reasons: Vec<Option<&str>> = (d.candidates.iter())
        .filter(|c| c.route.model == OPUS)
        .map(|c| c.skipped_reason.as_deref())
        .collect();
    assert!(
        !opus_reasons.contains(&Some(FAILED_IN_TASK)),
        "{opus_reasons:?}"
    );

    // Unchanged after the run is reloaded (decisions are never recomputed).
    let decisions = task(&run, "t1").routing_decisions.clone();
    let json = serde_json::to_string(&run).expect("serializes");
    let back: Run = serde_json::from_str(&json).expect("deserializes");
    assert_eq!(task(&back, "t1").routing_decisions, decisions);
    let edited = add(&back, &[m("t3", "[\"crates/c/**\"]", "")]);
    assert_eq!(task(&edited, "t1").routing_decisions, decisions);
}

/// Research and review tasks, S, owning nothing.
fn reader(id: &str, kind: &str) -> String {
    let target = if kind == "review" {
        "\nreview_target = \"main\""
    } else {
        ""
    };
    task_toml(id, "S", "[]", &format!("kind = \"{kind}\"{target}"))
}

#[test]
#[ignore = "M9.8.7a: tasks take their role-table rows, so no model list picks a worker, reviewer or rung-2 step; deleted with route_pick.rs in M9.8.13"]
fn research_routes_by_the_scout_list_and_review_tasks_by_the_review_list() {
    let lists = RouteLists {
        scout: list(
            Pick::Spread,
            vec![
                cand(Runtime::Codex, LUNA, None),
                cand(Runtime::Claude, HAIKU, Some(Effort::LOW)),
            ],
        ),
        review: list(
            Pick::Spread,
            vec![
                cand(Runtime::Codex, SOL, None),
                cand(Runtime::Claude, OPUS, Some(Effort::HIGH)),
            ],
        ),
        ..Default::default()
    };
    let run = built(
        &[
            reader("r1", "research"),
            reader("r2", "research"),
            reader("v1", "review"),
            reader("v2", "review"),
            task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", ""),
        ],
        lists,
    );
    let effort = task(&run, "t1").route.effort.clone();
    assert_eq!(task(&run, "r1").route, luna(effort.clone()));
    assert_eq!(task(&run, "r2").route, haiku(Effort::LOW), "scouts spread");
    // Review tasks take the review list's first unskipped candidate.
    assert_eq!(task(&run, "v1").route, sol(effort.clone()));
    assert_eq!(task(&run, "v2").route, sol(effort));
    let p = task(&run, "r2").list_pick.as_ref().expect("picked");
    assert_eq!((p.chosen, p.pick), (Some(1), ListPolicy::Spread));
    // The class lists are untouched: t1 keeps its class route.
    assert_eq!(task(&run, "t1").list_pick, None);

    // Not installed: skipped. Rung 2 follows the role list.
    let mut off = run.clone();
    off.orch.installed = installed(true, false);
    let added = add(&off, &[reader("r3", "research")]);
    assert_eq!(task(&added, "r3").route, haiku(Effort::LOW));
    let i = (run.tasks.iter()).position(|t| t.id() == "v1").expect("v1");
    let (next, _) = next_candidate(&run.limits, &run.tasks, i, &Installed::new()).expect("next");
    assert_eq!(next, opus(Effort::HIGH));
}

#[test]
fn scouts_spread_over_their_list() {
    let lists = frozen(RouteLists {
        scout: list(
            Pick::Spread,
            vec![
                cand(Runtime::Codex, LUNA, None),
                cand(Runtime::Claude, HAIKU, Some(Effort::LOW)),
            ],
        ),
        planner: list(Pick::First, vec![cand(Runtime::Codex, SOL, None)]),
        ..Default::default()
    });
    let none = Installed::new();
    let picked: Vec<Option<Route>> = (0..3)
        .map(|n| {
            role(&lists.scout, n, Effort::MEDIUM, &none)
                .expect("a list")
                .route
        })
        .collect();
    assert_eq!(
        picked,
        [
            Some(luna(Effort::MEDIUM)),
            Some(haiku(Effort::LOW)),
            Some(luna(Effort::MEDIUM))
        ]
    );
    // Skipped when not installed, whatever the rotation.
    let codexless = installed(true, false);
    let p = role(&lists.scout, 0, Effort::MEDIUM, &codexless).expect("a list");
    assert_eq!(p.route, Some(haiku(Effort::LOW)));
    assert_eq!(reasons(&p.candidates), [Some(NOT_INSTALLED), None]);
    // Every candidate skipped: no route (today's resolution), the snapshot kept.
    let p = role(&lists.planner, 0, Effort::HIGH, &codexless).expect("a list");
    assert_eq!((p.route, p.candidates.len()), (None, 1));
    // `first` ignores the rotation; no list, no pick.
    let p = role(&lists.planner, 5, Effort::HIGH, &none).expect("a list");
    assert_eq!((p.route, p.rotation), (Some(sol(Effort::HIGH)), 5));
    assert_eq!(role(&lists.decider, 0, Effort::LOW, &none), None);
}

/// A role list's every candidate can be taken (a `spread` scout list rotates, a later
/// epic's planner takes its own pick), so its runtimes are reachable: the start's
/// project-trust and API-key checks cover them.
#[test]
#[ignore = "M9.8.7b: the scouts, planners and brainstormers take their role-table rows, so no role list is reached; deleted with route_pick.rs in M9.8.13"]
fn a_role_lists_runtimes_are_reachable() {
    let mut run = crate::run::orch::test_support::run_of(1);
    run.tasks.clear();
    run.roster.retain(|e| e.runtime == Runtime::Claude);
    run.roster
        .extend(config().models.into_iter().filter(|e| e.model == LUNA));
    run.orch.orchestrator = Some(crate::run::orch::test_support::orchestrator());
    let reachable = crate::run::reach::reachable_runtimes;
    assert_eq!(reachable(&run), [Runtime::Claude]);
    for lists in [
        RouteLists {
            scout: list(
                Pick::Spread,
                vec![
                    cand(Runtime::Claude, HAIKU, None),
                    cand(Runtime::Codex, LUNA, None),
                ],
            ),
            ..Default::default()
        },
        RouteLists {
            planner: list(
                Pick::Spread,
                vec![
                    cand(Runtime::Claude, OPUS, None),
                    cand(Runtime::Codex, LUNA, None),
                ],
            ),
            ..Default::default()
        },
    ] {
        run.limits.route_lists = RouteListsFrozen::freeze(&lists, &run.roster);
        assert_eq!(reachable(&run), [Runtime::Claude, Runtime::Codex]);
    }
}
