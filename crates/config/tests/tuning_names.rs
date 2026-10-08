//! Milestone 9.5 (ruling T7-1): the `[orchestrator.tuning]` types can be named from
//! outside the config crate, as the daemon's refit does. Milestone 9.8 (task M9.8.13):
//! the model-list types are the config crate's own now (only the migration reads them).

use config::{ConfiguredBudgets, Tuning, TuningConfig};

#[test]
fn the_tuning_types_are_named_from_outside_the_crate() {
    let (config, problems) = config::parse(
        r#"
[orchestrator.budget.m]
minutes = 90
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    let tuning: &TuningConfig = &config.orchestrator.tuning;
    let table: &Tuning = &tuning.table;
    assert_eq!(table.min_samples, 30);
    let configured: ConfiguredBudgets = tuning.configured;
    assert_eq!((configured.s, configured.m), (false, true));
}
