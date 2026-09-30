//! Decision 16 (task M9.1.13): a merge candidate's tier-2 job is looked up in the
//! result cache **before** the candidate is materialized. When every step of its plan
//! hits (ruling C-13's all-or-nothing rule for a build step included), the candidate
//! is not materialized and no command runs.
//!
//! The affected set needs the module graph, and the graph is read in a checkout. Before
//! `materialize` the integration worktree holds the stage head, not the candidate, so
//! the pre-check trusts that graph only when it is the candidate's too: the worktree is
//! clean and at the stage head's tree, and the change touches no file the graph is read
//! from (a `Cargo.toml` or `Cargo.lock` for `cargo`; a `manifests` match for a command,
//! and with no `manifests` any change at all) and adds no module directory. Anything
//! else, and anything that goes wrong, is a miss: the executor materializes and runs
//! the job as usual. A miss costs one materialize; a wrong hit would skip a test.

use std::path::Path;

use super::{
    Job, JobSpec, OpCtx, TierSpec, affected_of, cache_ctx, changed_paths, hit, lookup_all,
    usable_hits,
};
use crate::run::git::{self, GitQueue};
use crate::run::globs::{OwnsMatcher, path_module};
use crate::run::model::OpId;
use crate::run::slots::TestScheduler;
use crate::run::test_cache::TestCache;
use crate::run::tiers::cache_key::key;
use crate::run::tiers::steps::plan;
use crate::run::tiers::{GraphSource, Scope, TierOutcome};

/// The tier-2 outcome of `spec` (its `head` the candidate) when every step is a
/// cached green, without materializing or running anything; `None` otherwise.
pub(crate) async fn cached_outcome(
    ctx: &OpCtx,
    shared: (&TestScheduler, &GitQueue, &std::ffi::OsStr),
    cache: &TestCache,
    op: OpId,
    spec: &TierSpec,
) -> Option<TierOutcome> {
    if spec.tier != 2 || !spec.profile.is_tiered() {
        return None;
    }
    // Decision 11: a toolchain id not yet run is not known, so neither is the key.
    if spec.toolchain.is_none() && spec.profile.toolchain_id.is_some() {
        return None;
    }
    let context = cache_ctx(spec, true, None)?;
    let job = Job::new(
        ctx,
        shared,
        op,
        JobSpec {
            root: &spec.root,
            dir: &spec.dir,
            env: &spec.env,
            timeout_secs: spec.timeout_secs,
            priority: spec.priority,
            critical: spec.critical,
        },
    )
    .await
    .ok()?;
    let (root, head, base) = (job.root.clone(), spec.head.clone(), spec.diff_base.clone());
    let tree = job
        .read(move |g, t| git::tree_of(g, &root, &head, t))
        .await
        .ok()?;
    let root = job.root.clone();
    let base_tree = job
        .read(move |g, t| git::tree_of(g, &root, &base, t))
        .await
        .ok()?;
    let dir = job.dir.clone();
    let at = job
        .read(move |g, t| git::checkout_tree(g, &dir, t))
        .await
        .ok()??;
    if at != base_tree {
        return None;
    }
    let changed = changed_paths(&job, spec).await.ok()?;
    let dir = job.dir.clone();
    let (profile, modules, manifests) = (
        spec.profile.module_graph.clone(),
        spec.modules.clone(),
        spec.manifests.clone(),
    );
    let names = changed.clone();
    let same_graph = super::bounded(super::SLACK, move || {
        Ok(graph_unchanged(
            &profile, &names, &manifests, &modules, &dir,
        ))
    })
    .await
    .unwrap_or(false);
    if !same_graph {
        return None;
    }
    let (affected, graph_note) = affected_of(&job, spec, &changed).await;
    let steps = plan(spec.tier, &affected, &spec.profile, spec.check.as_deref()).steps;
    let keys: Vec<_> = steps
        .iter()
        .map(|s| {
            Some(key(
                &tree,
                Scope::Gate,
                &s.command,
                &s.affected_key,
                &context,
            ))
        })
        .collect();
    let hits = usable_hits(&steps, lookup_all(cache, &spec.repo_dir, &keys).await);
    if steps.is_empty() || hits.iter().any(|h| !h) {
        return None;
    }
    Some(TierOutcome {
        tier: spec.tier,
        scope: Scope::Gate,
        affected,
        tree,
        steps: steps.iter().map(hit).collect(),
        ok: true,
        secs: 0,
        tail: String::new(),
        toolchain: None,
        graph_note,
    })
}

/// Whether the graph read in `dir` (at the stage head) is the candidate's too: the
/// change touches no file the graph is read from and adds no module directory.
/// Blocking (it looks at `dir`).
fn graph_unchanged(
    source: &GraphSource,
    changed: &[String],
    manifests: &[String],
    modules: &[String],
    dir: &Path,
) -> bool {
    let input = |path: &str| -> bool {
        match source {
            GraphSource::None => false,
            GraphSource::Cargo => {
                let name = path.rsplit('/').next().unwrap_or(path);
                name == "Cargo.toml" || name == "Cargo.lock"
            }
            GraphSource::Command(_) => match OwnsMatcher::new(manifests) {
                // Ruling C-8 (2b): with no `manifests` the key is the checkout's tree,
                // which any change moves.
                Ok(_) if manifests.is_empty() => true,
                Ok(m) => m.matches(path),
                Err(_) => true,
            },
        }
    };
    let new_module =
        |path: &str| path_module(path, modules).is_some_and(|m| !dir.join(&m).is_dir());
    !changed.iter().any(|p| input(p) || new_module(p))
}
