//! Milestone 9.8 task 1 (MR §8): what each role resolves to with today's code, for a
//! matrix of old configs, pinned in `model_roles_golden.json` before anything changes.
//! Task M9.8.3 adds the comparison with `config::models::resolve`; task M9.8.13 deletes
//! `today` with the code it calls, and the JSON stays the record.

use std::collections::BTreeMap;

use proto::{DeciderMode, Route, Runtime, TuningFile};

use crate::run::model::{FrozenList, Run};
use crate::run::orch::test_support as orch_support;
use crate::run::orch::{launch, roles};
use crate::run::refit;
use crate::run::route_pick::{self, Installed};
use crate::run::test_support::{build_tuned, plan_with, show, task_toml};

/// A cell whose plan does not validate today (decision 16).
pub(crate) const UNRESOLVED: &str = "unresolved";

const GOLDEN: &str = include_str!("model_roles_golden.json");

const CODEX_ROWS: &str = r#"
[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-luna"
strength = "fast"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6.1-sol"
strength = "frontier"
"#;

const ROUTES: &str = r#"
[[orchestrator.models]]
runtime = "codex"
model = "gpt-6-sol"
strength = "standard"

[[orchestrator.models]]
runtime = "codex"
model = "gpt-6.1-sol"
strength = "frontier"

[orchestrator.routes.s]
candidates = [{ runtime = "codex", model = "gpt-6-sol", effort = "low" }, { runtime = "claude", model = "claude-sonnet-5" }]

[orchestrator.routes.m]
candidates = [{ runtime = "claude", model = "claude-sonnet-5", effort = "medium" }, { runtime = "codex", model = "gpt-6-sol" }]

[orchestrator.routes.hub]
candidates = [{ runtime = "claude", model = "claude-opus-5-5", effort = "high" }, { runtime = "codex", model = "gpt-6.1-sol" }]

[orchestrator.routes.review]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }, { runtime = "claude", model = "claude-opus-5-5" }]

[orchestrator.routes.scout]
candidates = [{ runtime = "claude", model = "claude-sonnet-5", effort = "medium" }]

[orchestrator.routes.decider]
candidates = [{ runtime = "claude", model = "claude-sonnet-5" }]

[orchestrator.routes.planner]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }]

[orchestrator.routes.orchestrator]
candidates = [{ runtime = "codex", model = "gpt-6.1-sol", effort = "high" }, { runtime = "claude", model = "claude-opus-5-5" }]

[orchestrator.routes.brainstorm]
candidates = [{ runtime = "codex", model = "gpt-6-sol" }, { runtime = "claude", model = "claude-sonnet-5" }]
"#;

/// The matrix (MR §8): each name and its `config.toml`. Built at test time so the
/// roster rows are written once.
pub(crate) fn matrix() -> Vec<(&'static str, String)> {
    vec![
        ("default", String::new()),
        ("default_runtime_codex", "[orchestrator]\ndefault_runtime = \"codex\"\n".into()),
        ("routes", ROUTES.into()),
        ("agent", "[orchestrator.agent]\nruntime = \"claude\"\nmodel = \"claude-sonnet-5\"\neffort = \"medium\"\n".into()),
        ("planners", "[orchestrator.planners]\nruntime = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n".into()),
        ("scouts", "[orchestrator.scouts]\nruntime = \"claude\"\nstrength = \"standard\"\neffort = \"high\"\n".into()),
        ("deciders", "[orchestrator.deciders]\nmode = \"codex\"\nstrength = \"standard\"\neffort = \"medium\"\n".into()),
        ("custom_roster", format!("[orchestrator]\ndefault_runtime = \"codex\"\n{CODEX_ROWS}")),
        ("no_builtin_models", format!("[orchestrator]\ndefault_runtime = \"codex\"\nbuiltin_models = false\n{CODEX_ROWS}")),
    ]
}

pub(crate) fn golden() -> BTreeMap<String, BTreeMap<String, String>> {
    serde_json::from_str(GOLDEN).expect("model_roles_golden.json parses")
}

/// `<runtime>:<model>`, `default` for the CLI's own.
fn model(route: &Route) -> String {
    let id = if route.model.is_empty() {
        "default"
    } else {
        route.model.as_str()
    };
    format!("{}:{id}", route.runtime.label())
}

const PROFILE: &str = "goal = \"golden\"\n\n[profile]\ncheck = \"true\"\nhub = [\"hub/**\"]\n";

/// A one-task plan built with `cfg`: the task's route is today's whole resolution
/// (`resolve_task_lenient`, then `route_pick::pick_all`). Built with what a start
/// with no history freezes (`driver::tuning::tune_for_start` -> `refit::tuned`), so
/// the run carries `cfg`'s `[orchestrator.routes]` lists; `build_with`'s
/// `Tuned::default()` would drop them.
fn one_task(cfg: &config::Orchestrator, size: &str, owns: &str) -> Result<Run, String> {
    let extra = "test_mode = \"check\"\ntest_mode_reason = \"golden\"";
    let plan = plan_with(PROFILE, &[task_toml("t", size, owns, extra)]);
    let start = refit::tuned(&TuningFile::default(), cfg);
    build_tuned(&plan, cfg, start).map_err(|e| show(&e))
}

/// Today's helpers route: `DeciderContext::new`'s route, then `route_over` with every
/// runtime installed (its `decider` list's first, else the mode's route).
fn helpers_today(cfg: &config::Orchestrator) -> Route {
    let d = &cfg.deciders;
    let runtime = if d.mode == DeciderMode::Codex {
        Runtime::Codex
    } else {
        Runtime::Claude
    };
    let base = crate::decider::call::ladder_route(&cfg.models, runtime, d.strength, d.effort);
    let list = FrozenList::freeze(&cfg.tuning.routes.decider, &cfg.models);
    route_pick::role(&list, 0, base.effort, &Installed::new())
        .and_then(|pick| pick.route)
        .unwrap_or(base)
}

/// What each golden row resolves to with today's code.
pub(crate) fn today(cfg: &config::Orchestrator) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (key, size, owns) in [
        ("implementer.small", "S", "[\"a/**\"]"),
        ("implementer.medium", "M", "[\"b/**\"]"),
        ("implementer.hub", "M", "[\"hub/x.rs\"]"),
    ] {
        let cell = one_task(cfg, size, owns)
            .map_or(UNRESOLVED.to_string(), |run| model(&run.tasks[0].route));
        out.insert(key.to_string(), cell);
    }
    let mut run =
        one_task(cfg, "M", "[\"b/**\"]").expect("the medium plan builds in every matrix config");
    let none = Installed::new();
    let agent = &cfg.agent.agent;
    let orchestrator = roles::lists::orchestrator(
        None,
        &run.limits.route_lists.orchestrator,
        agent.effort,
        &none,
        || launch::resolve_orchestrator(None, agent, cfg.default_runtime, &run.roster),
    )
    .expect("the orchestrator resolves")
    .route;
    let mut record = orch_support::orchestrator();
    record.route = orchestrator.clone();
    run.orch.orchestrator = Some(record);
    let picks = roles::lists::brainstorm_picks(&run);
    let rows = [
        ("orchestrator", orchestrator),
        (
            "planner",
            launch::planner_route(&run).expect("an orchestrator is set"),
        ),
        ("test_writer", route_pick::writer_route(&run, 0)),
        (
            "reviewer",
            run.tasks[0]
                .review_route
                .clone()
                .expect("an M task is reviewed"),
        ),
        ("research", launch::scout_route_of(&run, "golden")),
        ("brainstorm.first", picks[0].route.clone()),
        ("brainstorm.second", picks[1].route.clone()),
        ("helpers", helpers_today(cfg)),
    ];
    for (key, route) in rows {
        out.insert(key.to_string(), model(&route));
    }
    out
}

#[test]
fn todays_resolution_matches_the_golden_file() {
    let golden = golden();
    let matrix = matrix();
    assert_eq!(
        golden.len(),
        matrix.len(),
        "one golden entry per matrix config"
    );
    for (name, text) in &matrix {
        let (config, problems) = config::parse(text);
        assert!(problems.is_empty(), "{name}: {problems:?}");
        assert_eq!(today(&config.orchestrator), golden[*name], "{name}");
    }
}
