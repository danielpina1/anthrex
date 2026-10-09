//! Task M9.8.7a: the run's frozen role table and the launchers that read it.

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{CatalogModel, CatalogSource, ModelCatalog, PlanEdit, RouteSpec, Runtime};

use super::*;
use crate::run::driver::build_models;
use crate::run::edits::apply_edits;
use crate::run::orch::EditSource;
use crate::run::test_support::{build_with, build_with_models, plan_with, show, task_toml};
use crate::run::validate::EditScope;

fn row(model: &str, effort: Option<&str>, fallback: Option<&str>) -> RoleChoice {
    RoleChoice {
        model: ModelRef::parse(model).unwrap(),
        effort: effort.map(Into::into),
        fallback: fallback.map(|f| ModelRef::parse(f).unwrap()),
    }
}

fn config_with(rows: &[(Role, RoleChoice)]) -> config::Orchestrator {
    config::Orchestrator {
        roles: ModelTable {
            rows: rows.iter().cloned().collect(),
            brainstorm: None,
        },
        ..config::Orchestrator::default()
    }
}

const PROFILE: &str = "goal = \"g\"\n\n[profile]\ncheck = \"true\"\nhub = [\"hub/**\"]\n";
const CHECK: &str = "test_mode = \"check\"\ntest_mode_reason = \"r\"";

#[test]
fn workers_take_their_size_row() {
    let cfg = config_with(&[
        (
            Role::ImplementerSmall,
            row("codex:gpt-6-luna", Some("low"), None),
        ),
        (
            Role::ImplementerMedium,
            row("claude:claude-sonnet-5", Some("medium"), None),
        ),
        (
            Role::ImplementerHub,
            row("codex:gpt-6.1-sol", Some("xhigh"), None),
        ),
    ]);
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("s", "S", "[\"a/**\"]", CHECK),
            task_toml("m", "M", "[\"b/**\"]", CHECK),
            task_toml("h", "M", "[\"hub/x.rs\"]", ""),
        ],
    );
    let run = build_with(&plan, &cfg).unwrap_or_else(|e| panic!("{}", show(&e)));
    let got: Vec<(String, String, String)> = (run.tasks.iter())
        .map(|t| {
            (
                t.id().to_string(),
                format!("{}:{}", t.route.runtime.label(), t.route.model),
                t.route.effort.as_str().to_string(),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("s".into(), "codex:gpt-6-luna".into(), "low".into()),
            ("m".into(), "claude:claude-sonnet-5".into(), "medium".into()),
            ("h".into(), "codex:gpt-6.1-sol".into(), "xhigh".into()),
        ]
    );
}

#[test]
fn the_reviewer_is_never_the_authors_model() {
    let author = RunModels::route_of(
        &ModelRef::parse("claude:claude-sonnet-5").unwrap(),
        Some("medium"),
    );
    let with_fallback = RunModels::resolve(
        &ModelTable {
            rows: [(
                Role::Reviewer,
                row(
                    "claude:claude-sonnet-5",
                    Some("high"),
                    Some("codex:gpt-6.1-sol"),
                ),
            )]
            .into(),
            brainstorm: None,
        },
        None,
    );
    let (route, warning) = with_fallback.reviewer_route(&author);
    assert_eq!(
        (
            route.runtime,
            route.model.as_str(),
            route.effort.is_default()
        ),
        (Runtime::Codex, "gpt-6.1-sol", true)
    );
    assert_eq!(warning, None);
    let without = RunModels::resolve(
        &ModelTable {
            rows: [(
                Role::Reviewer,
                row("claude:claude-sonnet-5", Some("high"), None),
            )]
            .into(),
            brainstorm: None,
        },
        None,
    );
    let (route, warning) = without.reviewer_route(&author);
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Claude, "claude-sonnet-5")
    );
    assert_eq!(
        warning.as_deref(),
        Some(
            "reviewer: claude:claude-sonnet-5 reviews work by the same model; set an \"if it struggles\" model for the reviewer in C-b S"
        )
    );
    // Same runtime, another model: allowed (D3).
    let other = RunModels::route_of(&ModelRef::parse("claude:claude-opus-5-5").unwrap(), None);
    assert_eq!(
        without.reviewer_route(&other),
        (
            RunModels::route_of(
                &ModelRef::parse("claude:claude-sonnet-5").unwrap(),
                Some("high")
            ),
            None
        )
    );
}

#[test]
fn the_racer_takes_the_rows_fallback_else_the_same_model() {
    let current = RunModels::route_of(
        &ModelRef::parse("claude:claude-sonnet-5").unwrap(),
        Some("medium"),
    );
    let with = RunModels::resolve(
        &ModelTable {
            rows: [(
                Role::ImplementerMedium,
                row(
                    "claude:claude-sonnet-5",
                    Some("medium"),
                    Some("codex:gpt-6-sol"),
                ),
            )]
            .into(),
            brainstorm: None,
        },
        None,
    );
    assert_eq!(
        with.racer_route(Role::ImplementerMedium, &current),
        RunModels::route_of(&ModelRef::parse("codex:gpt-6-sol").unwrap(), None)
    );
    let without = RunModels::resolve(&ModelTable::default(), None);
    assert_eq!(
        without.racer_route(Role::ImplementerMedium, &current),
        current
    );
}

fn catalog(models: &[(&str, &[&str], Option<&str>)], source: CatalogSource) -> ModelCatalog {
    ModelCatalog {
        runtime: Runtime::Codex,
        cli_version: "0.160.1".into(),
        fetched_at: 1,
        source,
        problem: None,
        models: (models.iter())
            .map(|(id, efforts, default)| CatalogModel {
                id: id.to_string(),
                label: id.to_string(),
                description: String::new(),
                efforts: efforts.iter().map(|e| e.to_string()).collect(),
                default_effort: default.map(Into::into),
                is_default: false,
            })
            .collect(),
    }
}

fn small_at(effort: &str) -> RunModels {
    RunModels::resolve(
        &ModelTable {
            rows: [(
                Role::ImplementerSmall,
                row("codex:gpt-6-luna", Some(effort), None),
            )]
            .into(),
            brainstorm: None,
        },
        None,
    )
}

#[test]
fn an_effort_the_model_does_not_offer_is_replaced_and_logged() {
    let mut models = small_at("max");
    let live = catalog(
        &[(
            "gpt-6-luna",
            &["low", "medium", "high", "xhigh"],
            Some("medium"),
        )],
        CatalogSource::Live,
    );
    let lines = models.validate(std::slice::from_ref(&live));
    assert_eq!(
        lines,
        ["effort 'max' not offered by gpt-6-luna; using medium"]
    );
    assert_eq!(
        models.choice(Role::ImplementerSmall).effort.as_deref(),
        Some("medium")
    );
    assert_eq!(
        models.efforts[&ModelRef::parse("codex:gpt-6-luna").unwrap()].efforts,
        ["low", "medium", "high", "xhigh"]
    );
    // Validated once: a second pass changes nothing.
    assert!(models.validate(&[live]).is_empty());
}

#[test]
fn an_effort_replaced_without_a_default_says_so() {
    let mut models = small_at("max");
    let cached = catalog(&[("gpt-6-luna", &["low"], None)], CatalogSource::Cached);
    assert_eq!(
        models.validate(&[cached]),
        ["effort 'max' not offered by gpt-6-luna; using the model's default"]
    );
    assert_eq!(models.choice(Role::ImplementerSmall).effort, None);
}

#[test]
fn a_builtin_catalog_validates_nothing() {
    let mut models = small_at("max");
    assert!(
        models
            .validate(&[catalog(
                &[("gpt-6-luna", &["low"], None)],
                CatalogSource::Builtin
            )])
            .is_empty()
    );
    assert_eq!(
        models.choice(Role::ImplementerSmall).effort.as_deref(),
        Some("max")
    );
}

#[test]
fn rule_9_names_the_sizes_and_the_role_table() {
    let cfg = config_with(&[
        (Role::ImplementerSmall, row("codex:default", None, None)),
        (
            Role::ImplementerMedium,
            row("claude:claude-sonnet-5", None, None),
        ),
    ]);
    let plan = plan_with(
        PROFILE,
        &[
            task_toml("s", "S", "[\"src/a.rs\"]", CHECK),
            task_toml("m", "M", "[\"src/**\"]", CHECK),
        ],
    );
    let errors = show(&build_with(&plan, &cfg).unwrap_err());
    assert!(
        errors.contains("the role table runs size S on codex and size M on claude; give them the same size or separate owns (rule 9)"),
        "{errors}"
    );
}

/// Decision 9: a run recorded before the role table (milestone 9.1's `run.json`) gets
/// the current one at restore; its tasks keep their routes.
#[test]
fn a_run_from_before_the_role_table_gets_the_current_one() {
    let text = include_str!("delivery/m9_1_run.json");
    let mut run: Run = serde_json::from_str(text).expect("a 9.1 run.json loads");
    assert_eq!(run.limits.models, None);
    let routes: Vec<proto::Route> = run.tasks.iter().map(|t| t.route.clone()).collect();
    let global = ModelTable {
        rows: [(Role::Reviewer, row("codex:gpt-6.1-sol", None, None))].into(),
        brainstorm: None,
    };
    let data = tempfile::tempdir().unwrap();
    let frozen = build_models::freeze(&global, &run.project, data.path(), &[]);
    build_models::fill(&mut run, frozen, 9_000);
    let models = run.limits.models.as_ref().expect("filled");
    assert_eq!(*models, validated(&global));
    assert_eq!(
        run.log.last().map(|e| e.text.as_str()),
        Some("models: this run predates the role table; using the current one")
    );
    let after: Vec<proto::Route> = run.tasks.iter().map(|t| t.route.clone()).collect();
    assert_eq!(after, routes);
    // A run that has one is left alone.
    let len = run.log.len();
    let again = build_models::freeze(&ModelTable::default(), &run.project, data.path(), &[]);
    build_models::fill(&mut run, again, 9_001);
    assert_eq!(run.log.len(), len);
    assert_eq!(*run.limits.models(), validated(&global));
}

/// `global` resolved and validated with no catalog in memory: the built-in effort lists
/// (M9.8.8 fix round 1).
fn validated(global: &ModelTable) -> RunModels {
    let mut models = RunModels::resolve(global, None);
    assert!(models.validate(&[]).is_empty());
    models
}

/// Decision 28: a paired hub task's test writer takes the `test_writer` row, unless an
/// unfinished task on another runtime than the row's overlaps the task's `owns` (rule 9
/// keeps every overlapping task on the hub task's own runtime, Claude here); then the
/// task's own route.
#[test]
fn the_test_writer_takes_its_row_unless_its_runtime_is_held() {
    let cfg = config_with(&[(Role::TestWriter, row("codex:default", None, None))]);
    let paired = task_toml(
        "h",
        "M",
        "[\"hub/x.rs\"]",
        "pair = true\ntest_to_write = \"x::works\"",
    );
    let profile = format!("{PROFILE}single_test = \"t {{test}}\"\n");
    let alone = build_with(&plan_with(&profile, std::slice::from_ref(&paired)), &cfg)
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    let writer = writer_route(&alone, 0);
    assert_eq!(
        (writer.runtime, writer.model.as_str()),
        (Runtime::Codex, "")
    );
    let other = task_toml("o", "M", "[\"hub/**\"]", "");
    let held = build_with(&plan_with(&profile, &[paired, other]), &cfg)
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(held.tasks[1].route.runtime, Runtime::Claude);
    assert_eq!(writer_route(&held, 0), held.tasks[0].route);
}

/// Decision 10: a user's route (the task edit form, `anthrex run edit`) is kept over
/// the task's row.
#[test]
fn a_users_route_is_kept() {
    let cfg = config_with(&[(Role::ImplementerMedium, row("codex:gpt-6-sol", None, None))]);
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &cfg,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(
        (
            run.tasks[0].route.runtime,
            run.tasks[0].route.model.as_str()
        ),
        (Runtime::Codex, "gpt-6-sol")
    );
    let edit = PlanEdit::AmendTask {
        task_id: "m".into(),
        brief: None,
        acceptance: None,
        route: Some(RouteSpec {
            runtime: Some(Runtime::Claude),
            model: Some("claude-opus-5-5".into()),
            strength: None,
            effort: None,
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
    let (edited, _) = apply_edits(&run, &[edit], &EditScope::Run, &EditSource::User, 5_000)
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    let route = &edited.tasks[0].route;
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Claude, "claude-opus-5-5")
    );
}

/// Review focus 5: a broken repository file leaves the run on the global table, with a
/// warning for its log, and the run builds.
#[test]
fn a_broken_repository_file_falls_back_to_the_global_table() {
    let data = tempfile::tempdir().unwrap();
    let project = std::path::Path::new("/tmp/p");
    let repo = crate::profile::repo_dir(data.path(), project);
    std::fs::create_dir_all(&repo).unwrap();
    let path = repo.join(config::models::REPO_FILE);
    std::fs::write(&path, "[models.reviewer\n").unwrap();
    let global = ModelTable {
        rows: [(
            Role::ImplementerMedium,
            row("codex:gpt-6-sol", Some("high"), None),
        )]
        .into(),
        brainstorm: None,
    };
    let (models, lines) = build_models::freeze(&global, project, data.path(), &[]);
    assert_eq!(models, validated(&global));
    let prefix = format!("{}: not valid TOML", path.display());
    assert!(
        lines.len() == 1 && lines[0].starts_with(&prefix),
        "{lines:?}"
    );
    let plan = plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]);
    let cfg = config::Orchestrator::default();
    let run = build_with_models(&plan, &cfg, models, lines.clone())
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(run.tasks[0].route.model, "gpt-6-sol");
    assert_eq!(run.log.last().map(|e| &e.text), lines.last());
}

/// A repository file's row wins over the global one (decision 7).
#[test]
fn a_repository_row_wins_over_the_global_one() {
    let data = tempfile::tempdir().unwrap();
    let project = std::path::Path::new("/tmp/p");
    let repo = crate::profile::repo_dir(data.path(), project);
    std::fs::create_dir_all(&repo).unwrap();
    let text = "[models.implementer.medium]\nmodel = \"claude:claude-opus-5-5\"\n";
    std::fs::write(repo.join(config::models::REPO_FILE), text).unwrap();
    let global = ModelTable {
        rows: [(Role::ImplementerMedium, row("codex:gpt-6-sol", None, None))].into(),
        brainstorm: None,
    };
    let (models, lines) = build_models::freeze(&global, project, data.path(), &[]);
    assert!(lines.is_empty(), "{lines:?}");
    assert_eq!(
        models.choice(Role::ImplementerMedium).model.label(),
        "claude:claude-opus-5-5"
    );
}

/// Decision 27 with ruling RL-1: a reviewer route that failed in this task gives way to
/// the row's other model, never the author's.
#[test]
fn a_reviewer_that_failed_in_this_task_takes_the_rows_other_model() {
    let models = RunModels::resolve(
        &ModelTable {
            rows: [(
                Role::Reviewer,
                row(
                    "codex:gpt-6.1-sol",
                    Some("high"),
                    Some("claude:claude-opus-5-5"),
                ),
            )]
            .into(),
            brainstorm: None,
        },
        None,
    );
    let author = RunModels::route_of(&ModelRef::parse("claude:claude-sonnet-5").unwrap(), None);
    let sol = models.route(Role::Reviewer);
    assert_eq!(
        reviewer_at_launch(&models, &author, &[]),
        (sol.clone(), None)
    );
    let opus = RunModels::route_of(&ModelRef::parse("claude:claude-opus-5-5").unwrap(), None);
    let failed = [sol.clone()];
    assert_eq!(
        reviewer_at_launch(&models, &author, &failed),
        (opus.clone(), None)
    );
    // Both failed: the row's choice stays (the ladder decides what follows).
    let both = [sol.clone(), opus];
    assert_eq!(reviewer_at_launch(&models, &author, &both).0, sol);
    // The fallback is the author's model: it is never taken.
    let by_opus = RunModels::route_of(&ModelRef::parse("claude:claude-opus-5-5").unwrap(), None);
    assert_eq!(reviewer_at_launch(&models, &by_opus, &failed).0, sol);
}

/// Preflight ruling F24: a user's route is kept as given, so an effort that is no
/// effort name is refused, naming the field.
#[test]
fn a_users_route_with_a_bad_effort_is_refused() {
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &config::Orchestrator::default(),
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    let amend = |effort: &str| PlanEdit::AmendTask {
        task_id: "m".into(),
        brief: None,
        acceptance: None,
        route: Some(RouteSpec {
            effort: Some(proto::Effort::new(effort)),
            ..RouteSpec::default()
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
    let errors = apply_edits(
        &run,
        &[amend("Max!")],
        &EditScope::Run,
        &EditSource::User,
        5_000,
    )
    .expect_err("refused");
    assert_eq!(
        show(&errors),
        "[route] task m: route.effort: \"Max!\" is not an effort name (1 to 16 characters of a-z, 0-9, _ and -)"
    );
    let (edited, _) = apply_edits(
        &run,
        &[amend("xhigh")],
        &EditScope::Run,
        &EditSource::User,
        5_000,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(edited.tasks[0].route.effort.as_str(), "xhigh");
}

/// Controller ruling (M9.8.7a fix round 1): a user's route naming a runtime or a
/// strength but no model is refused, since the row would silently replace it.
#[test]
fn a_users_route_without_a_model_is_refused() {
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &config::Orchestrator::default(),
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    let amend = |route: RouteSpec| PlanEdit::AmendTask {
        task_id: "m".into(),
        brief: None,
        acceptance: None,
        route: Some(route),
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    };
    let refused = |route: RouteSpec| {
        let errors = apply_edits(
            &run,
            &[amend(route)],
            &EditScope::Run,
            &EditSource::User,
            5_000,
        )
        .expect_err("refused");
        show(&errors)
    };
    let runtime = RouteSpec {
        runtime: Some(Runtime::Codex),
        ..RouteSpec::default()
    };
    assert_eq!(
        refused(runtime),
        "[route] task m: route.model: choose a model; runtime alone no longer selects one"
    );
    let strength = RouteSpec {
        strength: Some("frontier".into()),
        ..RouteSpec::default()
    };
    assert_eq!(
        refused(strength),
        "[route] task m: route.model: choose a model; strength alone no longer selects one"
    );
}

/// Fix round 1: the global table a start falls back to (the repository file unread)
/// is validated against the catalogs too.
#[test]
fn the_global_fallback_is_validated() {
    let global = ModelTable {
        rows: [(
            Role::ImplementerSmall,
            row("codex:gpt-6-luna", Some("max"), None),
        )]
        .into(),
        brainstorm: None,
    };
    let live = catalog(
        &[("gpt-6-luna", &["low", "medium"], Some("medium"))],
        CatalogSource::Live,
    );
    let (models, lines) = build_models::global_only(&global, &[live], "first");
    assert_eq!(
        lines,
        [
            "first",
            "effort 'max' not offered by gpt-6-luna; using medium"
        ]
    );
    assert_eq!(
        models.choice(Role::ImplementerSmall).effort.as_deref(),
        Some("medium")
    );
}

/// Final review M2 (MR §4.4): a model the catalog lists with no efforts offers none, so a
/// row's effort is replaced by the model's default (none) and logged.
#[test]
fn a_model_that_reports_no_efforts_loses_the_rows_effort() {
    let mut models = small_at("low");
    let live = catalog(&[("gpt-6-luna", &[], None)], CatalogSource::Live);
    assert_eq!(
        models.validate(&[live]),
        ["effort 'low' not offered by gpt-6-luna; using the model's default"]
    );
    assert_eq!(models.choice(Role::ImplementerSmall).effort, None);
}

/// Final review M5: the brainstorm pair shares one effort, so a model of the pair that
/// does not offer it sends the pair back to the models' defaults, logged once.
#[test]
fn a_brainstorm_effort_one_model_does_not_offer_is_dropped() {
    let table = ModelTable {
        rows: Default::default(),
        brainstorm: Some(proto::models::BrainstormChoice {
            first: ModelRef::parse("codex:gpt-6-sol").unwrap(),
            second: ModelRef::parse("codex:gpt-6-luna").unwrap(),
            effort: Some("max".into()),
        }),
    };
    let mut models = RunModels::resolve(&table, None);
    let live = catalog(
        &[
            (
                "gpt-6-sol",
                &["low", "medium", "high", "max"],
                Some("medium"),
            ),
            ("gpt-6-luna", &["low", "medium", "high"], Some("low")),
        ],
        CatalogSource::Live,
    );
    assert_eq!(
        models.validate(std::slice::from_ref(&live)),
        ["effort 'max' not offered by gpt-6-luna; using the model's default"]
    );
    assert_eq!(models.brainstorm.effort, None);
    assert!(models.validate(&[live]).is_empty());
}
