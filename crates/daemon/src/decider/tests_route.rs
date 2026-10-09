//! Milestone 9.5 task 10b, milestone 9.8 (decision 42, ruling F25): a decider call's
//! route is its helper row from the live table (and, for a call about a project, the
//! repository's `models.toml`), unless the deciders are off; the call routes over what
//! the probe finds installed at each call, moving to the row's fallback only when the
//! row's runtime is missing and the fallback's is not (D2). No decider is spawned:
//! every binary is a path that does not exist or a stand-in the probe only stats.

use std::path::Path;

use proto::DeciderMode;

use super::DeciderContext;
use super::call::{route_over, routed};
use crate::manager::ManagerConfig;
use crate::run::model_roles::Installed;

const NO_CLAUDE: &str = "/nonexistent/anthrex-test/claude";
const NO_CODEX: &str = "/nonexistent/anthrex-test/codex";
const NO_DECIDER: &str = "/nonexistent/anthrex-test/decider";

/// A row of `model` falling back to `fallback`.
fn with_fallback(model: &str, fallback: Option<&str>) -> proto::models::RoleChoice {
    let mut r = row(model);
    r.fallback = fallback.map(|f| proto::models::ModelRef::parse(f).unwrap());
    r
}

/// Off: no route resolved anew and no probe; on, with nothing recorded installed: the
/// row's route, on the row's runtime's program (`ANTHREX_DECIDER_BIN` when set).
#[tokio::test]
async fn deciders_take_their_row_unless_off() {
    use crate::decider::DeciderKind;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = DeciderMode::Off;
    let off = live_ctx(cfg.clone(), dir.path());
    let r = routed(&off, DeciderKind::Triage, None).await;
    assert_eq!((r.ctx.mode, r.moved), (DeciderMode::Off, None));
    assert_eq!(r.ctx.route, off.route);
    cfg.deciders.mode = DeciderMode::Claude;
    let row = with_fallback("codex:gpt-6-luna", None);
    cfg.roles
        .rows
        .insert(proto::models::Role::Helpers, row.clone());
    let on = live_ctx(cfg, dir.path());
    let r = route_over(&on, row, &Installed::new());
    assert_eq!(label(&r.ctx.route), "codex:gpt-6-luna");
    assert_eq!(r.ctx.program.to_str(), Some(NO_DECIDER));
    assert_eq!(r.moved, None);
}

/// Rulings RL-2, I6 and T10b-1 on the row (milestone 9.8, D2): with the row's runtime
/// not installed, the call moves to the row's fallback on the other runtime, and the
/// log line and the record say so; with no such fallback it stays on the row.
#[tokio::test]
async fn deciders_route_over_what_is_installed_at_each_call() {
    use crate::decider::DeciderKind;
    let dir = tempfile::tempdir().unwrap();
    let codex = dir.path().join("codex");
    testexec::write_executable(&codex, "#!/bin/sh\nexit 0\n");
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = DeciderMode::Claude;
    let row = with_fallback("claude:claude-haiku-4-5", Some("codex:gpt-6-luna"));
    cfg.roles.rows.insert(proto::models::Role::Helpers, row);
    let mut ctx = live_ctx(cfg.clone(), dir.path());
    ctx.bins = (NO_CLAUDE.into(), codex.display().to_string());
    let moved = routed(&ctx, DeciderKind::BlockedReason, None).await;
    assert_eq!(label(&moved.ctx.route), "codex:gpt-6-luna");
    assert_eq!(
        moved.moved.as_ref().map(label).as_deref(),
        Some("claude:claude-haiku-4-5")
    );
    assert_eq!(
        moved.moved_line().as_deref(),
        Some("decider: claude is not installed; using codex")
    );
    let candidates = moved.candidates();
    let reasons: Vec<Option<&str>> = (candidates.iter())
        .map(|c| c.skipped_reason.as_deref())
        .collect();
    assert_eq!(reasons, [Some("not installed"), None]);
    // The binary removed between two calls: nothing installed counts as everything
    // installed, and the call stays on the row.
    std::fs::remove_file(&codex).unwrap();
    let second = routed(&ctx, DeciderKind::BlockedReason, None).await;
    assert_eq!(
        (label(&second.ctx.route), second.moved),
        ("claude:claude-haiku-4-5".into(), None)
    );
    // No fallback on the other runtime: the row's model, never another.
    testexec::write_executable(&codex, "#!/bin/sh\nexit 0\n");
    let mut plain = cfg;
    let row = with_fallback("claude:claude-haiku-4-5", Some("claude:claude-sonnet-5"));
    plain.roles.rows.insert(proto::models::Role::Helpers, row);
    let mut same = live_ctx(plain, dir.path());
    same.bins = (NO_CLAUDE.into(), codex.display().to_string());
    let stays = routed(&same, DeciderKind::BlockedReason, None).await;
    assert_eq!(
        (label(&stays.ctx.route), stays.moved),
        ("claude:claude-haiku-4-5".into(), None)
    );
}

fn row(model: &str) -> proto::models::RoleChoice {
    proto::models::RoleChoice {
        model: proto::models::ModelRef::parse(model).expect("a model"),
        effort: None,
        fallback: None,
    }
}

/// A context over live settings holding `cfg`, with data directory `data`.
fn live_ctx(cfg: config::Orchestrator, data: &Path) -> DeciderContext {
    let live = crate::live_config::LiveSettings::defaults_of(cfg);
    let mut manager = ManagerConfig::for_tests("/tmp/unused.sock".into(), "/bin/sh".into());
    manager.claude_bin = NO_CLAUDE.into();
    manager.codex_bin = NO_CODEX.into();
    manager.decider_bin = Some(NO_DECIDER.into());
    DeciderContext::new(live, &manager, data)
}

fn label(route: &proto::Route) -> String {
    let id = if route.model.is_empty() {
        "default"
    } else {
        &route.model
    };
    format!("{}:{id}", route.runtime.label())
}

/// Milestone 9.8 (MR §3.4): each helper kind takes its row: its own override, else the
/// `helpers` row; a repository file's row comes first for a call about that project.
#[test]
fn each_kind_takes_its_row() {
    use crate::decider::DeciderKind;
    use proto::models::{HelperKind, ModelTable, Role};
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config::Orchestrator::default();
    cfg.deciders.mode = DeciderMode::Claude;
    cfg.roles
        .rows
        .insert(Role::Helpers, row("claude:claude-haiku-4-5"));
    let sonnet = row("claude:claude-sonnet-5");
    cfg.roles
        .rows
        .insert(Role::Helper(HelperKind::RunName), sonnet);
    let ctx = live_ctx(cfg, dir.path());
    let name = ctx.route_for(DeciderKind::RunName, None);
    assert_eq!(label(&name), "claude:claude-sonnet-5");
    assert_eq!(
        label(&ctx.route_for(DeciderKind::Triage, None)),
        "claude:claude-haiku-4-5"
    );
    let project = dir.path().join("repo");
    let mut repo = ModelTable::default();
    repo.rows
        .insert(Role::Helper(HelperKind::Triage), row("codex:default"));
    let file = crate::profile::repo_dir(dir.path(), &project).join(config::models::REPO_FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    config::models::save_repo(&file, &repo).unwrap();
    let triage = ctx.route_for(DeciderKind::Triage, Some(&project));
    assert_eq!(label(&triage), "codex:default");
    assert_eq!(
        label(&ctx.route_for(DeciderKind::Triage, None)),
        "claude:claude-haiku-4-5"
    );
}

/// Decision 42: a Settings save swaps the table into the live settings, and the next
/// call reads it.
#[test]
fn a_live_save_changes_the_next_calls_route() {
    use crate::decider::DeciderKind;
    use proto::models::Role;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config::Orchestrator::default();
    let ctx = live_ctx(cfg.clone(), dir.path());
    let before = ctx.route_for(DeciderKind::CheckSummary, None);
    assert_eq!(label(&before), "claude:claude-haiku-4-5", "the built-in");
    let mut saved = cfg;
    saved
        .roles
        .rows
        .insert(Role::Helpers, row("codex:gpt-6-luna"));
    ctx.live.swap_owned(&saved, Default::default());
    let after = ctx.route_for(DeciderKind::CheckSummary, None);
    assert_eq!(label(&after), "codex:gpt-6-luna");
}
