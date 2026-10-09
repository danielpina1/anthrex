//! `DeciderContext::new` (M8b.7, ruling R-T5-3; milestone 9.8): the timeout and paths
//! from the config, the route the live `helpers` row's, the program its runtime's.

use super::DeciderContext;
use crate::live_config::LiveSettings;
use crate::manager::ManagerConfig;
use proto::models::{ModelRef, Role, RoleChoice};
use proto::{DeciderMode, Effort, Runtime};
use std::path::Path;
use std::time::Duration;

/// A manager whose runtime commands do not exist: these tests never spawn, and nothing
/// built here could reach a real agent binary if one did. No decider override, so the
/// route's own command is the program.
fn manager() -> ManagerConfig {
    let mut manager = ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    manager.decider_bin = None;
    manager
}

fn context(cfg: config::Orchestrator) -> DeciderContext {
    DeciderContext::new(
        LiveSettings::defaults_of(cfg),
        &manager(),
        Path::new("/data"),
    )
}

#[test]
fn the_context_takes_its_timeout_and_paths_from_the_config() {
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = DeciderMode::Claude;
    cfg.deciders.timeout_secs = 42;
    let ctx = context(cfg);
    assert_eq!(ctx.timeout, Duration::from_secs(42));
    assert_eq!(ctx.cwd, Path::new("/data/deciders/cwd"));
    assert_eq!(ctx.schema_dir, Path::new("/data/deciders/schemas"));
    assert_eq!(ctx.data_dir, Path::new("/data"));
    assert_eq!(ctx.program, "/nonexistent/anthrex-test/claude");
    // The defaults: the built-in `helpers` row, Haiku at its default effort, in 90 s.
    let default = context(config::Orchestrator::default());
    assert_eq!(
        (
            default.route.runtime,
            default.route.model.as_str(),
            default.route.effort
        ),
        (Runtime::Claude, "claude-haiku-4-5", Effort::DEFAULT)
    );
    assert_eq!(default.timeout, Duration::from_secs(90));
}

/// Milestone 9.8 (decision 17): the route and program are the `helpers` row's runtime's,
/// whatever the mode says (it only says off or on).
#[test]
fn the_route_is_the_helpers_row() {
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = DeciderMode::Claude;
    let row = RoleChoice {
        model: ModelRef::default_of(Runtime::Codex),
        effort: Some("medium".into()),
        fallback: None,
    };
    cfg.roles.rows.insert(Role::Helpers, row);
    let ctx = context(cfg);
    assert_eq!(
        (
            ctx.route.runtime,
            ctx.route.model.as_str(),
            ctx.route.effort
        ),
        (Runtime::Codex, "", Effort::MEDIUM)
    );
    assert_eq!(ctx.program, "/nonexistent/anthrex-test/codex");
}
