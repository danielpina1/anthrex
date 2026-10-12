//! The tier executor's result-cache keys (decisions 30 and 31, ruling C-13), split out
//! of `tier.rs` for its line budget (milestone 9.14, ruling C18): each step's key, which
//! hits count, whether a checkout may store, and the job's cache context. `tier.rs` and
//! the tier-2 pre-check (`tier_cached.rs`) both key through [`step_keys`].
//!
//! Milestone 9.14 decisions 26, 27, 30 and 31: a `per_module` step keys on the inputs
//! id of its affected set's closure (`tiers::inputs`); every other step keeps the tree
//! id. A result is stored only when the checkout is at the job's tree, and a build
//! step's miss no longer voids a later step's hit.

use super::super::graph::TOOLCHAIN_UNKNOWN;
use super::Job;
use crate::run::git::{self, TreeEntry};
use crate::run::tiers::cache_key::{CacheKey, key, with_env};
use crate::run::tiers::inputs::key_tree;
use crate::run::tiers::{Affected, CacheCtx, ModuleGraph, Scope, Step, StepKind, TierSpec};

/// What a job's keys take besides its steps: the job's tree, its scope, its affected
/// set, and the module graph that set came from (none when no graph was read).
pub(super) struct KeyPlan<'a> {
    pub tree: &'a str,
    pub scope: Scope,
    pub affected: &'a Affected,
    pub graph: Option<&'a ModuleGraph>,
}

/// Each step's key under `ctx` (none without a context). Decision 31: when a step is
/// `per_module`, the tree's entries are read once, through `job.read`; a failed read
/// leaves those steps keyless (a miss), never with a wrong key.
pub(super) async fn step_keys(
    job: &Job<'_>,
    spec: &TierSpec,
    plan: KeyPlan<'_>,
    steps: &[Step],
    ctx: Option<&CacheCtx>,
) -> Vec<Option<CacheKey>> {
    let Some(c) = ctx else {
        return vec![None; steps.len()];
    };
    let mut entries = None;
    if steps.iter().any(|s| s.per_module) {
        let (root, tree) = (job.root.clone(), plan.tree.to_string());
        match job
            .read(move |g, t| git::tree_entries(g, &root, &tree, t))
            .await
        {
            Ok(read) => entries = Some(read),
            Err(error) => tracing::debug!(%error, "could not list a tier job's tree"),
        }
    }
    steps
        .iter()
        .map(|step| {
            let tree = match step.per_module {
                true => module_tree(&plan, spec, entries.as_deref())?,
                false => plan.tree.to_string(),
            };
            Some(key(&tree, plan.scope, &step.command, &step.affected_key, c))
        })
        .collect()
}

/// Decision 26's key tree of a `per_module` step: `None` (keyless) without a module
/// set, a graph or the tree's entries.
fn module_tree(
    plan: &KeyPlan<'_>,
    spec: &TierSpec,
    entries: Option<&[TreeEntry]>,
) -> Option<String> {
    let (Affected::Modules(set), Some(graph), Some(entries)) = (plan.affected, plan.graph, entries)
    else {
        return None;
    };
    let names = spec.profile.module_names;
    Some(key_tree(
        plan.tree,
        entries,
        set,
        &spec.modules,
        names,
        graph,
    ))
}

/// Ruling C-13 (2) as milestone 9.14 decision 30 (2) amends it: a job's steps are not
/// independent (a test step may use what the build step made), so a later step's miss
/// forces the build step to run even on its hit. A build step's miss no longer voids a
/// later step's hit: a step that hits needs no build. With tree-id keys only this gives
/// the same answers as before (a build miss means the tree moved, so every later key
/// misses too).
pub(super) fn usable_hits(steps: &[Step], mut hits: Vec<bool>) -> Vec<bool> {
    if let Some(build) = steps.iter().position(|s| s.kind == StepKind::Build)
        && hits[build + 1..].iter().any(|hit| !hit)
    {
        hits[build] = false;
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
    ctx.toolchain = with_env(&ctx.toolchain, &spec.env);
    Some(ctx)
}
