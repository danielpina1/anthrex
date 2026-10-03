//! Milestone 9.5 task 10a: the user's model lists route tasks and reviewers (decision
//! 9a). The roster adds the Codex rows `gpt-6.1-sol` (frontier) and `gpt-6-luna` (fast)
//! from `SHIPPED_CODEX`; every task owns distinct paths unless a test says otherwise.

use std::collections::BTreeSet;

use config::{Candidate, Pick, RouteList, RouteLists};
use proto::{
    AgentRole, Effort, ModelEntry, PlanEdit, PlanTask, Route, RoutingCandidate, Runtime, Strength,
};

use super::*;
use crate::run::edits::apply_edits;
use crate::run::model::{ListPolicy, Run};
use crate::run::orch::EditSource;
use crate::run::orch::context::{Asker, ContextInputs, context};
use crate::run::orch::contract::{ORCHESTRATOR_CONTRACT, PLANNER_CONTRACT};
use crate::run::plan::parse_plan;
use crate::run::refit::Tuned;
use crate::run::roster::pick_reviewer;
use crate::run::routing::record_worker;
use crate::run::test_support::{PROFILE, build_tuned, plan_with, show, task, task_toml};
use crate::run::validate::EditScope;

const SOL: &str = "gpt-6.1-sol";
const LUNA: &str = "gpt-6-luna";
const OPUS: &str = "claude-opus-5-5";
const SONNET: &str = "claude-sonnet-5";
const HAIKU: &str = "claude-haiku-4-5";

/// The default roster plus the two Codex rows the tests name, as shipped.
fn config() -> config::Orchestrator {
    let mut c = config::Orchestrator::default();
    for model in [SOL, LUNA] {
        let shipped = config::settings::SHIPPED_CODEX
            .iter()
            .find(|s| s.model == model)
            .expect("a shipped Codex row");
        c.models.push(ModelEntry {
            runtime: Runtime::Codex,
            model: model.into(),
            strength: shipped.strength,
            note: String::new(),
        });
    }
    c
}

fn cand(runtime: Runtime, model: &str, effort: Option<Effort>) -> Candidate {
    Candidate {
        runtime,
        model: model.into(),
        effort,
    }
}

fn list(pick: Pick, candidates: Vec<Candidate>) -> RouteList {
    RouteList { candidates, pick }
}

/// Decision 9a's `m` example: codex/gpt-6.1-sol high, claude/claude-opus-5-5 medium.
fn m_example(pick: Pick) -> RouteList {
    list(
        pick,
        vec![
            cand(Runtime::Codex, SOL, Some(Effort::High)),
            cand(Runtime::Claude, OPUS, Some(Effort::Medium)),
        ],
    )
}

fn tuned(lists: RouteLists) -> Tuned {
    Tuned {
        lists,
        ..Tuned::default()
    }
}

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.into(),
        strength,
        effort,
    }
}

fn sol(effort: Effort) -> Route {
    route(Runtime::Codex, SOL, Strength::Frontier, effort)
}

fn opus(effort: Effort) -> Route {
    route(Runtime::Claude, OPUS, Strength::Frontier, effort)
}

/// An M task owning `owns` (TOML array text), with `extra` lines.
fn m(id: &str, owns: &str, extra: &str) -> String {
    task_toml(id, "M", owns, extra)
}

fn built(tasks: &[String], lists: RouteLists) -> Run {
    let text = plan_with(PROFILE, tasks);
    build_tuned(&text, &config(), tuned(lists)).unwrap_or_else(|e| panic!("{}", show(&e)))
}

/// The task spec of one `[[task]]` table, as an edit would carry it.
fn spec(table: &str) -> PlanTask {
    let mut plan = parse_plan(&plan_with(PROFILE, &[table.to_string()])).expect("task parses");
    plan.tasks.pop().expect("one task")
}

fn add(run: &Run, tables: &[String]) -> Run {
    let edits: Vec<PlanEdit> = (tables.iter())
        .map(|t| PlanEdit::AddTask { task: spec(t) })
        .collect();
    let applied = apply_edits(run, &edits, &EditScope::Run, &EditSource::User, 5_000);
    applied.unwrap_or_else(|e| panic!("{}", show(&e))).0
}

fn reasons(candidates: &[RoutingCandidate]) -> Vec<Option<&str>> {
    (candidates.iter())
        .map(|c| c.skipped_reason.as_deref())
        .collect()
}

fn installed(claude: bool, codex: bool) -> Installed {
    [("claude".to_string(), claude), ("codex".to_string(), codex)].into()
}

#[test]
fn first_gives_every_task_the_first_candidate() {
    let lists = RouteLists {
        s: list(Pick::First, vec![cand(Runtime::Codex, LUNA, None)]),
        m: m_example(Pick::First),
        ..Default::default()
    };
    let run = built(
        &[
            task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
            m("t3", "[\"crates/c/**\"]", ""),
        ],
        lists,
    );
    // No effort in the S list: the class's (S: low).
    let luna = route(Runtime::Codex, LUNA, Strength::Fast, Effort::Low);
    assert_eq!(task(&run, "t1").route, luna);
    for id in ["t2", "t3"] {
        let t = task(&run, id);
        assert_eq!(t.route, sol(Effort::High), "{id}");
        let pick = t.list_pick.as_ref().expect("a list pick");
        assert_eq!(
            (pick.chosen, pick.pick),
            (Some(0), ListPolicy::First),
            "{id}"
        );
        // No `review` list: the roster's reviewer for the new author.
        let level = t.review_level.expect("reviewed");
        let reviewer = pick_reviewer(&run.roster, &t.route, level);
        assert_eq!(t.review_route.as_ref(), Some(&reviewer), "{id}");
    }
}

#[test]
fn spread_round_robins_overlap_groups_in_plan_order() {
    let lists = RouteLists {
        m: m_example(Pick::Spread),
        ..Default::default()
    };
    // The build passes the overlapping-runtimes rule (rule 9): t1 and t3 overlap.
    let run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
            m("t3", "[\"crates/a/x/**\"]", ""),
            m("t4", "[\"crates/c/**\"]", ""),
        ],
        lists,
    );
    let routes: Vec<&Route> = ["t1", "t2", "t3", "t4"]
        .iter()
        .map(|id| &task(&run, id).route)
        .collect();
    assert_eq!(
        routes,
        [
            &sol(Effort::High),
            &opus(Effort::Medium),
            &sol(Effort::High),
            &sol(Effort::High)
        ]
    );
    let slots: Vec<Option<u32>> = ["t1", "t2", "t3", "t4"]
        .iter()
        .map(|id| task(&run, id).list_pick.as_ref().and_then(|p| p.slot))
        .collect();
    assert_eq!(slots, [Some(0), Some(1), Some(0), Some(2)]);
}

#[test]
fn an_explicit_route_wins_and_a_runtime_only_route_takes_that_runtimes_first_candidate() {
    let lists = RouteLists {
        m: m_example(Pick::First),
        ..Default::default()
    };
    let run = built(
        &[
            m(
                "t1",
                "[\"crates/a/**\"]",
                "[task.route]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"",
            ),
            m(
                "t2",
                "[\"crates/b/**\"]",
                "[task.route]\nruntime = \"claude\"",
            ),
            m(
                "t3",
                "[\"crates/c/**\"]",
                "[task.route]\nruntime = \"codex\"\nstrength = \"frontier\"\neffort = \"low\"",
            ),
        ],
        lists,
    );
    let sonnet = route(Runtime::Claude, SONNET, Strength::Standard, Effort::Medium);
    assert_eq!(task(&run, "t1").route, sonnet, "the plan's model wins");
    assert_eq!(
        task(&run, "t2").route,
        opus(Effort::Medium),
        "claude's first"
    );
    assert_eq!(
        task(&run, "t3").route,
        sol(Effort::Low),
        "a named strength is the plan's"
    );
    let t1 = task(&run, "t1").list_pick.as_ref().expect("snapshotted");
    assert_eq!(t1.chosen, None, "the list did not choose an explicit route");
    let t2 = task(&run, "t2").list_pick.as_ref().expect("picked");
    assert_eq!(t2.chosen, Some(1));
    assert_eq!(
        reasons(&t2.candidates),
        [Some("the task names the runtime claude"), None]
    );
}

#[test]
fn an_added_task_joins_its_overlap_group() {
    let lists = RouteLists {
        m: m_example(Pick::Spread),
        ..Default::default()
    };
    let run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
        ],
        lists,
    );
    assert_eq!(task(&run, "t2").route, opus(Effort::Medium));
    let edited = add(
        &run,
        &[
            m("t3", "[\"crates/b/x/**\"]", ""),
            m("t4", "[\"crates/d/**\"]", ""),
        ],
    );
    assert_eq!(
        task(&edited, "t3").route,
        opus(Effort::Medium),
        "joins t2's"
    );
    let t3 = task(&edited, "t3").list_pick.as_ref().expect("picked");
    assert_eq!(t3.slot, Some(1));
    assert_eq!(
        task(&edited, "t4").route,
        sol(Effort::High),
        "the next slot"
    );
    let t4 = task(&edited, "t4").list_pick.as_ref().expect("picked");
    assert_eq!(t4.slot, Some(2));
    // An amend naming an empty route hands the task back to the lists.
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: Some(proto::RouteSpec {
            runtime: Some(Runtime::Claude),
            ..Default::default()
        }),
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    };
    let amended = apply_edits(&edited, &[amend], &EditScope::Run, &EditSource::User, 6_000)
        .unwrap_or_else(|e| panic!("{}", show(&e)))
        .0;
    assert_eq!(task(&amended, "t1").route, opus(Effort::Medium));
}

/// The list of the rung-2 tests: [sol medium, opus high, sol high].
fn ladder_lists() -> RouteLists {
    RouteLists {
        m: list(
            Pick::First,
            vec![
                cand(Runtime::Codex, SOL, Some(Effort::Medium)),
                cand(Runtime::Claude, OPUS, Some(Effort::High)),
                cand(Runtime::Codex, SOL, Some(Effort::High)),
            ],
        ),
        ..Default::default()
    }
}

#[test]
fn rung_2_takes_the_next_candidate_and_never_breaks_the_overlap_rule() {
    let run = built(
        &[
            m("t1", "[\"crates/a/**\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
        ],
        ladder_lists(),
    );
    let none = Installed::new();
    assert_eq!(task(&run, "t1").route, sol(Effort::Medium));
    let (next, step) = next_candidate(&run.limits, &run.tasks, 0, &none).expect("a next one");
    assert_eq!(next, opus(Effort::High));
    assert_eq!(step.chosen, Some(1));
    assert_eq!(reasons(&step.candidates)[0], Some(CURRENT_ROUTE));

    // An unfinished task on codex overlaps t1: the claude candidate is skipped.
    let mut overlapped = add(&run, &[m("t3", "[\"crates/a/x/**\"]", "")]);
    assert_eq!(task(&overlapped, "t3").route.runtime, Runtime::Codex);
    let (next, step) =
        next_candidate(&overlapped.limits, &overlapped.tasks, 0, &none).expect("the codex step");
    assert_eq!(next, sol(Effort::High));
    assert_eq!(
        reasons(&step.candidates),
        [Some(CURRENT_ROUTE), Some(OVERLAPPING_OWNS), None]
    );

    // From the last candidate it cycles to the first, skipping one identical to it.
    overlapped.tasks[0].route = sol(Effort::High);
    let (next, _) =
        next_candidate(&overlapped.limits, &overlapped.tasks, 0, &none).expect("cycles");
    assert_eq!(next, sol(Effort::Medium));

    // A one-candidate list leaves rung 2 to `roster::escalate`.
    let one = RouteLists {
        m: list(Pick::First, vec![cand(Runtime::Codex, SOL, None)]),
        ..Default::default()
    };
    let run = built(&[m("t1", "[\"crates/a/**\"]", "")], one);
    assert_eq!(next_candidate(&run.limits, &run.tasks, 0, &none), None);
}

#[test]
fn the_reviewer_is_the_first_qualifying_candidate() {
    let lists = RouteListsFrozen::freeze(
        &RouteLists {
            review: list(
                Pick::First,
                vec![
                    cand(Runtime::Claude, HAIKU, None),
                    cand(Runtime::Claude, OPUS, Some(Effort::High)),
                    cand(Runtime::Codex, SOL, None),
                ],
            ),
            ..Default::default()
        },
        &config().models,
    );
    let none = Installed::new();
    let codex_author = sol(Effort::Medium);
    let (chosen, candidates) =
        reviewer(&lists, &codex_author, ReviewLevel::Medium, &none, &[]).expect("a list");
    assert_eq!(chosen, Some(opus(Effort::High)));
    assert_eq!(reasons(&candidates)[0], Some(BELOW_STRENGTH));

    let claude_author = route(Runtime::Claude, SONNET, Strength::Standard, Effort::Medium);
    let (chosen, candidates) =
        reviewer(&lists, &claude_author, ReviewLevel::Medium, &none, &[]).expect("a list");
    // No effort in the list: the review level's (medium).
    assert_eq!(chosen, Some(sol(Effort::Medium)));
    assert_eq!(
        reasons(&candidates),
        [Some(AUTHOR_RUNTIME), Some(AUTHOR_RUNTIME), None]
    );

    // None qualifying: `pick_reviewer`.
    let haiku_only = RouteListsFrozen::freeze(
        &RouteLists {
            review: list(Pick::First, vec![cand(Runtime::Claude, HAIKU, None)]),
            ..Default::default()
        },
        &config().models,
    );
    let (chosen, _) =
        reviewer(&haiku_only, &codex_author, ReviewLevel::Medium, &none, &[]).expect("a list");
    assert_eq!(chosen, None);
    let roster = config().models;
    assert_eq!(
        review_route(
            &haiku_only,
            &roster,
            &codex_author,
            ReviewLevel::Medium,
            &none
        ),
        pick_reviewer(&roster, &codex_author, ReviewLevel::Medium)
    );
    // No list at all.
    let empty = RouteListsFrozen::default();
    assert_eq!(
        reviewer(&empty, &codex_author, ReviewLevel::Medium, &none, &[]),
        None
    );
}

#[test]
fn a_candidate_not_installed_is_skipped() {
    let lists = RouteLists {
        s: list(
            Pick::First,
            vec![
                cand(Runtime::Codex, LUNA, None),
                cand(Runtime::Claude, HAIKU, None),
            ],
        ),
        m: m_example(Pick::Spread),
        review: list(
            Pick::First,
            vec![
                cand(Runtime::Codex, SOL, None),
                cand(Runtime::Claude, OPUS, None),
            ],
        ),
        ..Default::default()
    };
    let mut run = built(
        &[
            task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", ""),
            m("t2", "[\"crates/b/**\"]", ""),
            m("t3", "[\"crates/c/**\"]", ""),
        ],
        lists,
    );
    run.orch.installed = installed(true, false);
    // Every task pick, by an edit and by a pick over the whole plan.
    let edited = add(&run, &[m("t4", "[\"crates/d/**\"]", "")]);
    let mut all = run.clone();
    let targets = (0..all.tasks.len()).collect();
    pick(
        &all.limits,
        &all.roster,
        &mut all.tasks,
        &targets,
        &all.orch.installed,
    );
    let picked = all.tasks.iter().chain([task(&edited, "t4")]);
    for t in picked {
        assert_eq!(t.route.runtime, Runtime::Claude, "{}", t.id());
        let p = t.list_pick.as_ref().expect("picked");
        assert_eq!(reasons(&p.candidates)[0], Some(NOT_INSTALLED), "{}", t.id());
    }
    // Rung 2 and the reviewer skip it too.
    let step = next_candidate(&all.limits, &all.tasks, 1, &all.orch.installed);
    assert_eq!(step, None, "only codex is left after opus");
    let author = opus(Effort::Medium);
    let installed = &all.orch.installed;
    let (chosen, candidates) = reviewer(
        &all.limits.route_lists,
        &author,
        ReviewLevel::Medium,
        installed,
        &[],
    )
    .expect("list");
    assert_eq!(chosen, None);
    assert_eq!(
        reasons(&candidates),
        [Some(NOT_INSTALLED), Some(AUTHOR_RUNTIME)]
    );
}

#[test]
fn task_routing_history_keeps_the_list_and_choice() {
    let lists = RouteLists {
        m: m_example(Pick::First),
        ..Default::default()
    };
    let mut run = built(
        &[
            m(
                "t1",
                "[\"crates/a/**\"]",
                "[task.route]\nruntime = \"claude\"\nmodel = \"claude-opus-5-5\"",
            ),
            m("t2", "[\"crates/a/x/**\"]", ""),
            m("t3", "[\"crates/b/**\"]", ""),
        ],
        lists,
    );
    // t2 overlaps t1 (claude, explicit): codex is skipped for it.
    assert_eq!(task(&run, "t2").route, opus(Effort::Medium));
    run.tasks[1].session = 1;
    record_worker(&mut run, 1, 100);
    let d = &task(&run, "t2").routing_decisions[0];
    assert_eq!(
        (
            d.trigger.as_str(),
            d.source.as_str(),
            d.policy_version.as_str()
        ),
        ("initial", "configured_list", LIST_POLICY)
    );
    assert_eq!(d.pick_policy.as_deref(), Some("first"));
    assert_eq!(d.selected_index, 1);
    assert_eq!(d.chosen, opus(Effort::Medium));
    assert_eq!(
        reasons(&d.candidates),
        [Some(OVERLAPPING_OWNS), None],
        "the entire list, in order"
    );
    let routes: Vec<&Route> = d.candidates.iter().map(|c| &c.route).collect();
    assert_eq!(routes, [&sol(Effort::High), &opus(Effort::Medium)]);

    // An explicit route records `explicit_task`, appended to the list.
    run.tasks[0].session = 1;
    record_worker(&mut run, 0, 100);
    let d = &task(&run, "t1").routing_decisions[0];
    assert_eq!(
        (d.source.as_str(), d.policy_version.as_str()),
        ("explicit_task", LIST_POLICY)
    );
    assert_eq!((d.selected_index, d.candidates.len()), (1, 2));
    assert_eq!(d.role, AgentRole::Worker);
}

#[path = "route_pick_tests_frozen.rs"]
mod frozen;

#[path = "route_pick_tests_roles.rs"]
mod roles;
