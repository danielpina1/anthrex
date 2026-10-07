//! `DeciderContext::new` (M8b.7, ruling R-T5-3): the route, effort, timeout and paths.

use super::DeciderContext;
use crate::manager::ManagerConfig;
use proto::{DeciderMode, Effort, ModelEntry, Runtime, Strength};
use std::path::Path;
use std::time::Duration;

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.into(),
        strength,
        note: String::new(),
    }
}

/// A manager whose runtime commands do not exist: these tests never spawn, and nothing
/// built here could reach a real agent binary if one did. No decider override, so the
/// mode's own command is the program.
fn manager() -> ManagerConfig {
    let mut manager = ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    manager.decider_bin = None;
    manager
}

fn context(
    mode: DeciderMode,
    models: Option<Vec<ModelEntry>>,
    strength: Strength,
) -> DeciderContext {
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = mode;
    cfg.deciders.strength = strength;
    cfg.deciders.effort = Effort::MEDIUM;
    cfg.deciders.timeout_secs = 42;
    if let Some(models) = models {
        cfg.models = models;
    }
    DeciderContext::new(&cfg, &manager(), Path::new("/data"))
}

#[test]
fn the_context_takes_its_timeout_paths_and_effort_from_the_config() {
    let ctx = context(DeciderMode::Claude, None, Strength::Fast);
    assert_eq!(ctx.timeout, Duration::from_secs(42));
    assert_eq!(ctx.cwd, Path::new("/data/deciders/cwd"));
    assert_eq!(ctx.schema_dir, Path::new("/data/deciders/schemas"));
    assert_eq!(ctx.route.effort, Effort::MEDIUM);
    assert_eq!(ctx.program, "/nonexistent/anthrex-test/claude");
    assert_eq!(
        context(DeciderMode::Codex, None, Strength::Fast).program,
        "/nonexistent/anthrex-test/codex"
    );
    // The defaults: the fast tier at low effort, within 90 s.
    let default = DeciderContext::new(
        &config::Orchestrator::default(),
        &manager(),
        Path::new("/d"),
    );
    assert_eq!(
        (
            default.route.model.as_str(),
            default.route.strength,
            default.route.effort
        ),
        ("claude-haiku-4-5", Strength::Fast, Effort::LOW)
    );
    assert_eq!(default.timeout, Duration::from_secs(90));
}

#[test]
fn the_route_is_the_lowest_strength_at_or_above_on_the_modes_runtime() {
    // The built-in roster: Claude's fast model, and Codex's only (standard) entry.
    let claude = context(DeciderMode::Claude, None, Strength::Fast).route;
    assert_eq!(
        (claude.runtime, claude.model.as_str(), claude.strength),
        (Runtime::Claude, "claude-haiku-4-5", Strength::Fast)
    );
    let codex = context(DeciderMode::Codex, None, Strength::Fast).route;
    assert_eq!(
        (codex.runtime, codex.model.as_str(), codex.strength),
        (Runtime::Codex, "", Strength::Standard)
    );
    // Not the first entry: the lowest strength at or above the one asked for.
    let roster = vec![
        entry(Runtime::Claude, "opus", Strength::Frontier),
        entry(Runtime::Codex, "gpt", Strength::Fast),
        entry(Runtime::Claude, "sonnet", Strength::Standard),
        entry(Runtime::Claude, "haiku", Strength::Fast),
    ];
    let fast = context(DeciderMode::Claude, Some(roster.clone()), Strength::Fast).route;
    assert_eq!(
        (fast.model.as_str(), fast.strength),
        ("haiku", Strength::Fast)
    );
    let standard = context(
        DeciderMode::Claude,
        Some(roster.clone()),
        Strength::Standard,
    )
    .route;
    assert_eq!(
        (standard.model.as_str(), standard.strength),
        ("sonnet", Strength::Standard)
    );
    // Nothing at or above: that runtime's first entry.
    let only_fast = vec![entry(Runtime::Claude, "haiku", Strength::Fast)];
    let above = context(DeciderMode::Claude, Some(only_fast), Strength::Frontier).route;
    assert_eq!(
        (above.model.as_str(), above.strength),
        ("haiku", Strength::Fast)
    );
    // Nothing on the mode's runtime: no model (the CLI's default), never the peer's.
    let none = context(
        DeciderMode::Codex,
        Some(vec![entry(Runtime::Claude, "haiku", Strength::Fast)]),
        Strength::Fast,
    )
    .route;
    assert_eq!(
        (none.runtime, none.model.as_str(), none.strength),
        (Runtime::Codex, "", Strength::Fast)
    );
}
