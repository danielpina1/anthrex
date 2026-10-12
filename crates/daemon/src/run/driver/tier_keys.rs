//! The tier executor's result-cache keys (decisions 30 and 31, ruling C-13), split out
//! of `tier.rs` for its line budget (milestone 9.14, ruling C18): each step's key, which
//! hits count, whether a checkout may store, and the job's cache context.

use super::super::graph::TOOLCHAIN_UNKNOWN;
use super::Job;
use crate::run::git;
use crate::run::tiers::cache_key::{CacheKey, key, with_path};
use crate::run::tiers::{CacheCtx, Scope, Step, StepKind, TierSpec};

/// Each step's key under `ctx` (none without a context).
pub(super) fn step_keys(
    tree: &str,
    scope: Scope,
    steps: &[Step],
    ctx: Option<&CacheCtx>,
) -> Vec<Option<CacheKey>> {
    match ctx {
        Some(c) => steps
            .iter()
            .map(|step| Some(key(tree, scope, &step.command, &step.affected_key, c)))
            .collect(),
        None => vec![None; steps.len()],
    }
}

/// Ruling C-13 (2), amending decision 31: a job's steps are not independent (a test
/// step may use what the build step made), so a build step's hit counts only when
/// every later step also hits; otherwise everything from the build step on runs.
pub(super) fn usable_hits(steps: &[Step], mut hits: Vec<bool>) -> Vec<bool> {
    if let Some(build) = steps.iter().position(|s| s.kind == StepKind::Build)
        && hits[build..].iter().any(|hit| !hit)
    {
        for hit in &mut hits[build..] {
            *hit = false;
        }
    }
    hits
}

/// Whether the job's checkout is clean for tracked files and at `tree`, read on a
/// bounded blocking thread. Anything that goes wrong is `false`.
pub(super) async fn checkout_is_at(job: &Job<'_>, tree: &str) -> bool {
    let dir = job.dir.clone();
    match job.read(move |g, t| git::checkout_tree(g, &dir, t)).await {
        Ok(Some(at)) => at == tree,
        Ok(None) => false,
        Err(error) => {
            tracing::debug!(%error, "could not read a tier checkout's tree");
            false
        }
    }
}

/// Decision 30's context for this job's keys: none for an untiered profile, without a
/// context, or with an `"unknown"` toolchain. A toolchain this job ran (decision 11)
/// replaces the context's.
///
/// Ruling C-13 (5), beyond the brief: the toolchain part also carries a hash of the
/// steps' effective `PATH` (the daemon's own, unless the profile's `env` sets one).
pub(super) fn cache_ctx(
    spec: &TierSpec,
    tiered: bool,
    ran_toolchain: Option<&str>,
) -> Option<CacheCtx> {
    let mut ctx = spec.cache.clone().filter(|_| tiered)?;
    if let Some(id) = ran_toolchain {
        ctx.toolchain = id.to_string();
    }
    if ctx.toolchain == TOOLCHAIN_UNKNOWN {
        return None;
    }
    let path = spec
        .env
        .iter()
        .rev()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    ctx.toolchain = with_path(&ctx.toolchain, &path);
    Some(ctx)
}
