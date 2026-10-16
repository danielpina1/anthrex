//! `apply`, the op-shaped wrapper over `lease` (milestone 9.14 decisions 9–11, task
//! M9.14.6): right before the driver executes an op, each checkout the op names gets a
//! leased directory, substituted for `CARGO_TARGET_DIR` in every `env` the op carries
//! for it, and granted to the worker's sandbox and to the confined commands.

use std::path::{Path, PathBuf};

use proto::Runtime;

use super::{Leased, PoolClass, TARGET_DIR_VAR, TargetPool};
use crate::lock;
use crate::run::driver::OpCtx;
use crate::run::engine::OpKind;
use crate::run::model::PropagateSpec;
use crate::run::tiers::TierSpec;

type Env = Vec<(String, String)>;

/// Every `(checkout, env)` pair `kind` carries, nested tier jobs included. **A new op
/// kind that carries a profile `env` must be added here** (the match is exhaustive on
/// purpose, so the compiler names it): classify it, or its builds stay in the profile's
/// own target directory.
fn envs(kind: &mut OpKind) -> Vec<(PathBuf, &mut Env)> {
    let mut out = Vec::new();
    match kind {
        OpKind::CreateRunBranch { path, env, .. }
        | OpKind::PrepareWorktree { path, env, .. }
        | OpKind::Proof { path, env, .. } => out.push((path.clone(), env)),
        OpKind::CreateWindow { spec, .. } => out.push((spec.cwd.clone(), &mut spec.env)),
        OpKind::Check { dir, env, .. } => out.push((dir.clone(), env)),
        OpKind::MergeCandidate {
            integration,
            env,
            tier,
            ..
        } => {
            out.push((integration.clone(), env));
            if let Some(tier) = tier {
                out.push(tier_env(tier));
            }
        }
        OpKind::Tier(spec) => out.push(tier_env(spec)),
        OpKind::TestAt(spec) => out.push((spec.dir.clone(), &mut spec.env)),
        OpKind::Propagate(spec) => {
            let PropagateSpec {
                integration,
                env,
                tier,
                ..
            } = &mut **spec;
            out.push((integration.clone(), env));
            if let Some(tier) = tier {
                out.push(tier_env(tier));
            }
        }
        // No profile `env` (milestone 9.14 fact check: none added since `56f87a1d`).
        OpKind::ResumeSession { .. }
        | OpKind::VerifyDone { .. }
        | OpKind::CountCommits { .. }
        | OpKind::DiffSoFar { .. }
        | OpKind::PrepareReview { .. }
        | OpKind::HandBack { .. }
        | OpKind::AbortMerge { .. }
        | OpKind::RemoveWorktree { .. }
        | OpKind::VerifyRefs { .. }
        | OpKind::CreateStageBranch { .. }
        | OpKind::Accept { .. }
        | OpKind::Discard { .. }
        | OpKind::Decide { .. }
        | OpKind::MeasureDiff { .. }
        | OpKind::AppendHistory { .. }
        | OpKind::CreateOrchestrator { .. }
        | OpKind::RestartOrchestrator { .. }
        | OpKind::StartScout { .. }
        | OpKind::StartPlanner { .. }
        | OpKind::StartDesignAgent { .. }
        | OpKind::ResolveTarget { .. }
        | OpKind::Host { .. }
        | OpKind::CrownRacer { .. }
        | OpKind::CommitDesignDocs(_)
        | OpKind::QueueDistill { .. } => {}
    }
    out
}

fn tier_env(spec: &mut TierSpec) -> (PathBuf, &mut Env) {
    (spec.dir.clone(), &mut spec.env)
}

fn sets_target(env: &Env) -> bool {
    env.iter().any(|(k, _)| k == TARGET_DIR_VAR)
}

impl TargetPool {
    /// Decision 9: leases a directory for each checkout `kind` names whose env sets
    /// `CARGO_TARGET_DIR` (once per distinct checkout), substitutes it, and records the
    /// grants on `ctx`, the op's own copy: `pool_grant` for a `CreateWindow` (decision
    /// 10, C1) and the `ConfineSpec`'s `target_dir` (decision 11). Returns the run log
    /// notes `(run id, text)` of decision 7. A `None` `ctx.pool` changes nothing.
    pub(crate) async fn apply(&self, ctx: &mut OpCtx, kind: &mut OpKind) -> Vec<(String, String)> {
        let Some(pool) = ctx.pool.as_deref() else {
            return Vec::new();
        };
        let pool = pool.clone();
        let wanted: Vec<(PathBuf, Env)> = envs(kind)
            .into_iter()
            .filter(|(_, env)| sets_target(env))
            .map(|(checkout, env)| (checkout, env.clone()))
            .collect();
        let mut leased: Vec<(PathBuf, PathBuf)> = Vec::new();
        let mut seen: Vec<PathBuf> = Vec::new();
        let mut notes = Vec::new();
        for (checkout, env) in wanted {
            if seen.contains(&checkout) {
                continue;
            }
            seen.push(checkout.clone());
            match self.lease_why(&pool, &checkout, &env).await {
                Leased::Dir(dir) => leased.push((checkout, dir)),
                Leased::Full(class) => {
                    if lock(&self.noted).insert(checkout.clone()) {
                        notes.push((pool.run_id.clone(), fallback_note(class, &checkout)));
                    }
                }
                Leased::Skip => {}
            }
        }
        let dir_of = |checkout: &Path| {
            leased
                .iter()
                .find(|(c, _)| c == checkout)
                .map(|(_, d)| d.clone())
        };
        for (checkout, env) in envs(kind) {
            if let Some(dir) = dir_of(&checkout) {
                for (_, value) in env.iter_mut().filter(|(k, _)| k == TARGET_DIR_VAR) {
                    *value = dir.display().to_string();
                }
            }
        }
        if let OpKind::CreateWindow { spec, .. } = kind
            && let Some(dir) = dir_of(&spec.cwd)
        {
            if let Some(sandbox) = spec.claude_sandbox.as_mut()
                && !sandbox.writable_roots.contains(&dir)
            {
                sandbox.writable_roots.push(dir.clone());
            }
            if spec.runtime == Runtime::Codex && !spec.codex_writable_roots.contains(&dir) {
                spec.codex_writable_roots.push(dir.clone());
            }
            ctx.pool_grant = Some(dir);
        }
        // Every checkout of one op is the same one in practice (a merge's tier-2 job
        // runs in the integration checkout); its confined commands get that directory.
        if let (Some(confine), Some((_, dir))) = (ctx.confine.as_mut(), leased.first()) {
            confine.target_dir = Some(dir.clone());
        }
        notes
    }
}

/// Decision 7's run log line.
fn fallback_note(class: PoolClass, checkout: &Path) -> String {
    let class = match class {
        PoolClass::Task => "task",
        PoolClass::Engine => "engine",
    };
    let name = checkout
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| checkout.display().to_string());
    format!(
        "target pool: no free {class} directory for {name}; it builds in its own target directory"
    )
}

#[cfg(test)]
#[path = "apply_tests.rs"]
mod tests;
