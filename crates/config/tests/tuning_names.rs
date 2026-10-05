//! Milestone 9.5 (ruling T7-1): the `[orchestrator.tuning]` and model-list types can be
//! named from outside the config crate, as the daemon's refit and route picking do.

use config::{Candidate, ConfiguredBudgets, Pick, RouteList, RouteLists, Tuning, TuningConfig};

#[test]
fn the_tuning_types_are_named_from_outside_the_crate() {
    let (config, problems) = config::parse(
        r#"
[orchestrator.budget.m]
minutes = 90

[orchestrator.routes.review]
candidates = [{ runtime = "claude", model = "claude-sonnet-5", effort = "high" }]
pick = "spread"
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let tuning: &TuningConfig = &config.orchestrator.tuning;
    let table: &Tuning = &tuning.table;
    assert_eq!(table.min_samples, 30);
    let configured: ConfiguredBudgets = tuning.configured;
    assert_eq!((configured.s, configured.m), (false, true));
    let lists: &RouteLists = &tuning.routes;
    let review: &RouteList = &lists.review;
    assert_eq!(review.pick, Pick::Spread);
    assert_eq!(
        review.candidates,
        vec![Candidate {
            runtime: proto::Runtime::Claude,
            model: "claude-sonnet-5".to_string(),
            effort: Some(proto::Effort::High),
        }]
    );
}
