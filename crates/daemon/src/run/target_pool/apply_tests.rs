//! `apply` on hand-built `OpCtx` and `OpKind` values over a temporary data directory
//! (task M9.14.6), with no driver: every env the op carries for a leased checkout gets
//! the pool directory, the worker's sandbox grants it, and everything else is left as
//! the profile says.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{ProfileSource, Runtime};

use super::super::io_tests::Rig;
use super::super::{POOL_DIR_NAME, PoolCtx, TARGET_DIR_VAR, TargetPool};
use crate::run::driver::OpCtx;
use crate::run::engine::OpKind;
use crate::run::model::PropagateSpec;
use crate::run::role_launch::worker_spec;
use crate::run::slots::Priority;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
use crate::run::tiers::{TestAtSpec, TierProfile, TierSpec};

const PROFILE_VALUE: &str = "/profile/target";

fn env() -> Vec<(String, String)> {
    vec![
        (TARGET_DIR_VAR.to_string(), PROFILE_VALUE.to_string()),
        ("FOO".to_string(), "bar".to_string()),
    ]
}

fn pool_of(
    task_dirs: u16,
    engine_dirs: u16,
    change: impl FnOnce(&mut config::TargetPool),
) -> TargetPool {
    let mut config = config::TargetPool {
        task_dirs: Some(task_dirs),
        engine_dirs,
        ..config::TargetPool::default()
    };
    change(&mut config);
    TargetPool::new(&config, 3)
}

fn ctx(pool: Option<&PoolCtx>) -> OpCtx {
    OpCtx {
        run_id: "r1".into(),
        project: PathBuf::from("/repo"),
        data_dir: PathBuf::from("/data/runs/r1"),
        git_timeout: Duration::from_secs(5),
        check_timeout: Duration::from_secs(5),
        confine: None,
        pinned: Default::default(),
        pool: pool.cloned().map(Box::new),
        pool_grant: None,
    }
}

fn tier(dir: &Path) -> TierSpec {
    TierSpec {
        tier: 2,
        stage: 1,
        root: PathBuf::from("/repo"),
        dir: dir.to_path_buf(),
        scratch: None,
        diff_base: "base".into(),
        head: "head".into(),
        profile: TierProfile::default(),
        check: Some("true".into()),
        hub: Vec::new(),
        source: Vec::new(),
        modules: Vec::new(),
        manifests: Vec::new(),
        single_test: None,
        timeout_secs: 60,
        env: env(),
        priority: Priority::Gate,
        critical: false,
        cache: None,
        toolchain: None,
        repo_dir: PathBuf::from("/data/repos/repo-1"),
    }
}

fn check(dir: &Path) -> OpKind {
    OpKind::Check {
        dir: dir.to_path_buf(),
        command: "true".into(),
        timeout_secs: 60,
        env: env(),
        scratch: None,
    }
}

/// Every op kind that carries a profile env, each in its checkout.
fn every_kind(rig: &Rig) -> Vec<(&'static str, OpKind)> {
    let runs = &rig.ctx.runs_dir;
    let (integration, full) = (rig.ctx.integration.clone(), rig.ctx.full.clone());
    let root = PathBuf::from("/repo");
    vec![
        (
            "Proof",
            OpKind::Proof {
                root: root.clone(),
                path: runs.join("t1.proof"),
                red: "red".into(),
                head: "head".into(),
                command: "true".into(),
                passed: "ok".into(),
                timeout_secs: 60,
                setup: None,
                env: env(),
                red_only: false,
            },
        ),
        ("Check", check(&runs.join("t1.proof"))),
        (
            "PrepareWorktree",
            OpKind::PrepareWorktree {
                root: root.clone(),
                branch: "anthrex/r1/t1".into(),
                from: "base".into(),
                path: runs.join("t1"),
                setup: Some("true".into()),
                env: env(),
            },
        ),
        (
            "CreateRunBranch",
            OpKind::CreateRunBranch {
                root: root.clone(),
                branch: "anthrex/r1/integration".into(),
                base_sha: "base".into(),
                path: integration.clone(),
                setup: None,
                env: env(),
            },
        ),
        (
            "MergeCandidate",
            OpKind::MergeCandidate {
                root: root.clone(),
                integration: integration.clone(),
                run_branch: "anthrex/r1/integration".into(),
                expected_run_head: "head".into(),
                base_branch: "main".into(),
                expected_base: "base".into(),
                task_head: "task".into(),
                message: "merge".into(),
                check: None,
                timeout_secs: 60,
                env: env(),
                guarded: Vec::new(),
                also_integration: false,
                tier: Some(Box::new(tier(&integration))),
            },
        ),
        ("Tier", OpKind::Tier(Box::new(tier(&full)))),
        (
            "TestAt",
            OpKind::TestAt(Box::new(TestAtSpec {
                root: root.clone(),
                dir: runs.join("t2.proof"),
                commit: "c".into(),
                setup: None,
                commands: vec!["true".into()],
                timeout_secs: 60,
                env: env(),
            })),
        ),
        (
            "Propagate",
            OpKind::Propagate(Box::new(PropagateSpec {
                root,
                integration: integration.clone(),
                from: 1,
                to: 2,
                from_head: "h".into(),
                to_branch: "anthrex/r1/stage-2".into(),
                expected_to_head: "h2".into(),
                also_integration: false,
                base_branch: "main".into(),
                expected_base: "base".into(),
                guarded: Vec::new(),
                message: "propagate".into(),
                tier: Some(Box::new(tier(&integration))),
                check: None,
                timeout_secs: 60,
                env: env(),
                tasks: BTreeSet::new(),
            })),
        ),
    ]
}

/// Every env `kind` carries, in the order `apply` reads them (via its debug form, so
/// the test does not share `apply`'s own walk).
fn target_values(kind: &OpKind) -> Vec<String> {
    let text = format!("{kind:?}");
    let needle = format!("(\"{TARGET_DIR_VAR}\", \"");
    text.match_indices(&needle)
        .map(|(at, _)| {
            let rest = &text[at + needle.len()..];
            rest[..rest.find('"').unwrap()].to_string()
        })
        .collect()
}

fn foo_values(kind: &OpKind) -> usize {
    format!("{kind:?}").matches("(\"FOO\", \"bar\")").count()
}

#[tokio::test]
async fn every_env_of_the_op_gets_the_leased_dir() {
    let rig = Rig::new();
    let pool = pool_of(3, 2, |_| {});
    let root = rig.ctx.repo_dir.join(POOL_DIR_NAME);
    for (name, mut kind) in every_kind(&rig) {
        let envs = target_values(&kind).len();
        let foos = foo_values(&kind);
        let mut c = ctx(Some(&rig.ctx));
        let notes = pool.apply(&mut c, &mut kind).await;
        assert_eq!(notes, Vec::<(String, String)>::new(), "{name}");
        let values = target_values(&kind);
        assert_eq!(values.len(), envs, "{name}");
        let class = match name {
            "CreateRunBranch" | "MergeCandidate" | "Tier" | "Propagate" => "engine-",
            _ => "task-",
        };
        for value in &values {
            let leaf = Path::new(value).file_name().unwrap().to_str().unwrap();
            assert_eq!(Path::new(value).parent(), Some(root.as_path()), "{name}");
            assert!(leaf.starts_with(class), "{name}: {value}");
            assert!(Path::new(value).is_dir(), "{name}: {value}");
        }
        assert!(
            values.windows(2).all(|w| w[0] == w[1]),
            "{name}: {values:?}"
        );
        assert_eq!(foo_values(&kind), foos, "{name}: other variables untouched");
        assert_eq!(c.pool_grant, None, "{name}");
    }

    // An op with no `CARGO_TARGET_DIR` is untouched, and leases nothing.
    let mut kind = check(&rig.ctx.runs_dir.join("t9"));
    if let OpKind::Check { env, .. } = &mut kind {
        env.retain(|(k, _)| k != TARGET_DIR_VAR);
    }
    let before = kind.clone();
    pool.apply(&mut ctx(Some(&rig.ctx)), &mut kind).await;
    assert_eq!(kind, before);
}

/// A run with one task `t1` whose worktree is the rig's `t1`, sandboxed, on `runtime`.
fn window(rig: &Rig, runtime: Runtime) -> OpKind {
    let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
    run.limits.worker_sandbox = true;
    run.tasks[0].route.runtime = runtime;
    run.tasks[0].worktree = rig.ctx.runs_dir.join("t1");
    let mut spec = worker_spec(&run, &run.tasks[0]);
    spec.env = env();
    OpKind::CreateWindow {
        name: "w".into(),
        spec: Box::new(spec),
        session_uuid: None,
        first_turn: String::new(),
        project: PathBuf::from("/repo"),
        worktree: rig.ctx.runs_dir.join("t1"),
        jitter_ms: 0,
        extract: None,
    }
}

#[tokio::test]
async fn a_worker_session_sees_the_leased_target_dir_and_may_write_it() {
    let rig = Rig::new();
    let pool = pool_of(2, 1, |_| {});
    let dir = rig.pool_dir("task-1");

    let mut claude = window(&rig, Runtime::Claude);
    let mut c = ctx(Some(&rig.ctx));
    pool.apply(&mut c, &mut claude).await;
    let OpKind::CreateWindow { spec, .. } = &claude else {
        unreachable!()
    };
    let target = (TARGET_DIR_VAR.to_string(), dir.display().to_string());
    assert!(spec.env.contains(&target), "{:?}", spec.env);
    let roots = &spec.claude_sandbox.as_ref().unwrap().writable_roots;
    assert!(roots.contains(&dir), "{roots:?}");
    assert_eq!(c.pool_grant, Some(dir.clone()));

    // A Codex worker on another checkout of the same name's run: its own pool.
    let rig = Rig::new();
    let mut codex = window(&rig, Runtime::Codex);
    let mut c = ctx(Some(&rig.ctx));
    pool_of(2, 1, |_| {}).apply(&mut c, &mut codex).await;
    let OpKind::CreateWindow { spec, .. } = &codex else {
        unreachable!()
    };
    let dir = rig.pool_dir("task-1");
    assert!(
        spec.codex_writable_roots.contains(&dir),
        "{:?}",
        spec.codex_writable_roots
    );
    assert_eq!(c.pool_grant, Some(dir));
}

#[tokio::test]
async fn a_review_checkout_and_a_disabled_repository_keep_the_profile_value() {
    let rig = Rig::new();
    let review = check(&rig.ctx.runs_dir.join("t1.review"));
    let task = check(&rig.ctx.runs_dir.join("t1"));
    let repo = rig.ctx.repo_root.display().to_string();
    // The configured root spelled through a link resolves to the same repository.
    let link = rig.root.join("repo-link");
    std::os::unix::fs::symlink(&rig.ctx.repo_root, &link).unwrap();
    let pools = [
        ("review", pool_of(2, 1, |_| {}), review.clone()),
        (
            "disabled",
            pool_of(2, 1, |c| c.disabled_repos = vec![repo.clone()]),
            task.clone(),
        ),
        (
            "disabled via a link",
            pool_of(2, 1, |c| {
                c.disabled_repos = vec![link.display().to_string()]
            }),
            task.clone(),
        ),
        ("off", pool_of(2, 1, |c| c.enabled = false), task.clone()),
    ];
    for (name, pool, kind) in pools {
        let mut changed = kind.clone();
        pool.apply(&mut ctx(Some(&rig.ctx)), &mut changed).await;
        assert_eq!(changed, kind, "{name}");
    }

    // The run's root spelled unresolved (`..`) is still the disabled repository.
    let mut spelled = rig.ctx.clone();
    spelled.repo_root = rig.root.join("wt/../repo");
    let mut changed = task.clone();
    pool_of(2, 1, |c| c.disabled_repos = vec![repo.clone()])
        .apply(&mut ctx(Some(&spelled)), &mut changed)
        .await;
    assert_eq!(changed, task, "a disabled root spelled through ..");

    // An unstored profile: `OpCtx::of` gives no pool, so nothing changes.
    let mut run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", "[\"a\"]", "")]));
    run.repo_dir = rig.ctx.repo_dir.clone();
    run.profile
        .env
        .insert(TARGET_DIR_VAR.to_string(), "{worktree}/target".to_string());
    run.profile_source = Some(ProfileSource::Plan);
    assert!(OpCtx::of(&run).pool.is_none());
    run.profile_source = Some(ProfileSource::Stored);
    assert!(
        OpCtx::of(&run).pool.is_some(),
        "a stored profile with {{worktree}}"
    );
    let mut changed = task.clone();
    let mut unstored = OpCtx::of(&run);
    unstored.pool = None;
    pool_of(2, 1, |_| {})
        .apply(&mut unstored, &mut changed)
        .await;
    assert_eq!(changed, task);
}

#[tokio::test]
async fn no_free_dir_falls_back_to_the_profile_value_and_notes_once() {
    let rig = Rig::new();
    let pool = pool_of(2, 1, |_| {});
    for name in ["t1", "t2"] {
        let mut kind = check(&rig.ctx.runs_dir.join(name));
        pool.apply(&mut ctx(Some(&rig.ctx)), &mut kind).await;
        assert_ne!(target_values(&kind), [PROFILE_VALUE], "{name} leases");
    }
    let third = rig.ctx.runs_dir.join("t3");
    let mut notes = Vec::new();
    for _ in 0..2 {
        let mut kind = check(&third);
        notes.extend(pool.apply(&mut ctx(Some(&rig.ctx)), &mut kind).await);
        assert_eq!(target_values(&kind), [PROFILE_VALUE]);
        assert_eq!(kind, check(&third));
    }
    assert_eq!(
        notes,
        [(
            "r1".to_string(),
            "target pool: no free task directory for t3; it builds in its own target directory"
                .to_string()
        )]
    );
}
