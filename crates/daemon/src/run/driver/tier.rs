//! Milestone 9.1's tier executor (task M9.1.9): `OpKind::Tier` and `OpKind::TestAt`.
//!
//! A tier job prepares its scratch checkout as M8a's check does, reads what the change
//! touches and the module graph, works out the affected set and its steps (the pure
//! `run::tiers`), and runs each step in order: it waits for the step's slots in the
//! daemon's [`TestScheduler`] (an `.await`, under no lock), runs the command on a
//! blocking thread with a bound (`tier_step.rs`), and gives the slots
//! back. A red `tests`, `timing` or `shard` step of a tiered profile is retried once
//! (decision 33 as ruling C-7 amends it): by name with `single_test` when at most
//! [`RETRY_NAMES_MAX`](crate::run::tiers::RETRY_NAMES_MAX) names were read, then the whole step once more; otherwise the
//! whole step with a new grant. A build step and a timed-out step are never retried.
//! An untiered profile runs M8a's `check` once, exactly as written.
//!
//! Decisions 30 and 31 (task M9.1.10, ruling C-13): a tiered job with a cache context
//! looks every step up in the daemon's [`TestCache`] before it runs any; a hit is not
//! run and is recorded `cached`, except that a build step's hit counts only when every
//! later step hits too. A green step is stored only when the checkout it ran in is
//! clean and at the key's tree. A step whose last whole run passed is stored, with its flaky
//! names; a red or timed-out step never is. An untiered profile, and a toolchain that
//! is `"unknown"`, neither reads nor writes the cache.
//!
//! Every git read, command, cache access, directory creation and removal runs on
//! `spawn_blocking` with a timeout; every git write goes through the run's
//! [`GitQueue`]. No lock is held here, and no slot is held across anything but its own
//! step.

use std::ffi::OsStr;

use super::OpCtx;
use super::gate_files;
use super::graph::{TOOLCHAIN_UNKNOWN, module_graph, toolchain};
use super::tier_step::{
    Job, JobSpec, SLACK, Stop, bounded, failed, fresh_dir, isolation_env, remove_dir, run_step,
    stop,
};
use crate::run::engine::{OpResult, ScratchAt};
use crate::run::git::{self, GitQueue};
use crate::run::model::OpId;
use crate::run::slots::{Priority, TestScheduler, Want};
use crate::run::test_cache::{CacheEntry, TestCache};
use crate::run::tiers::affected::affected;
use crate::run::tiers::cache_key::CacheKey;
use crate::run::tiers::steps::plan;
use crate::run::tiers::{
    Affected, GRAPH_TIMEOUT, GraphSource, GraphState, Scope, Step, StepKind, StepOutcome,
    TOOLCHAIN_TIMEOUT, TestAtSpec, TierOutcome, TierSpec,
};

/// The affected set a tier-3 job records: one fixed value, so a tier-3 cache key never
/// depends on a reason text (M9.1.6's note for task M9.1.14).
pub(crate) const FULL_SUITE: &str = "full suite";
/// The affected set of an untiered profile's single `check`.
pub(crate) const UNTIERED: &str = "untiered profile";

/// Executes `OpKind::Tier` (decisions 14, 16, 18, 20–28, 30–33).
pub(crate) async fn run_tier(
    ctx: &OpCtx,
    sched: &TestScheduler,
    cache: &TestCache,
    queue: &GitQueue,
    git: &OsStr,
    op: OpId,
    spec: &TierSpec,
) -> OpResult {
    match tier_job(ctx, (sched, queue, git), cache, op, spec).await {
        Ok(outcome) => OpResult::Tier(Box::new(outcome)),
        Err(result) => *result,
    }
}

async fn tier_job(
    ctx: &OpCtx,
    shared: (&TestScheduler, &GitQueue, &OsStr),
    cache: &TestCache,
    op: OpId,
    spec: &TierSpec,
) -> Result<TierOutcome, Stop> {
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
    .map_err(stop)?;
    if let Some(scratch) = &spec.scratch {
        job.prepare(scratch).await?;
    }
    let (root, head) = (job.root.clone(), spec.head.clone());
    let tree = job
        .read(move |g, t| git::tree_of(g, &root, &head, t))
        .await
        .map_err(stop)?;
    let scope = if spec.tier >= 3 {
        Scope::Full
    } else {
        Scope::Gate
    };
    let tiered = spec.profile.is_tiered();
    let mut outcome = TierOutcome {
        tier: spec.tier,
        scope,
        affected: Affected::Full(UNTIERED.to_string()),
        tree,
        steps: Vec::new(),
        ok: true,
        secs: 0,
        tail: String::new(),
        toolchain: None,
        graph_note: None,
    };
    let steps = if tiered {
        let (affected, note) = affected_set(&job, spec, scope).await?;
        outcome.graph_note = note;
        if spec.toolchain.is_none()
            && let Some(command) = spec.profile.toolchain_id.clone()
        {
            outcome.toolchain = Some(run_toolchain(&job, command).await?);
        }
        let planned = plan(spec.tier, &affected, &spec.profile, spec.check.as_deref());
        outcome.affected = affected;
        planned.steps
    } else {
        // Decision 6 and controller ruling 1: M8a's check, exactly as written.
        spec.check
            .iter()
            .map(|check| Step {
                kind: StepKind::Tests,
                command: check.clone(),
                exclusive: false,
                affected_key: "-".to_string(),
            })
            .collect()
    };
    let context = cache_ctx(spec, tiered, outcome.toolchain.as_deref());
    let keys = step_keys(&outcome.tree, scope, &steps, context.as_ref());
    let hits = usable_hits(&steps, lookup_all(cache, &spec.repo_dir, &keys).await);
    let mut elsewhere_logged = false;
    for (k, step) in steps.iter().enumerate() {
        if hits[k] {
            outcome.steps.push(hit(step));
            continue;
        }
        let (ran, tail) = run_step(&job, spec, tiered, k + 1, step).await?;
        outcome.secs += ran.secs;
        if !ran.ok {
            outcome.steps.push(ran);
            outcome.ok = false;
            outcome.tail = tail;
            break;
        }
        if let Some(key) = keys[k].clone() {
            // Ruling C-13 (1): the key names the judged head's tree; store only what
            // ran on exactly that tree.
            if checkout_is_at(&job, &key.tree).await {
                let entry = CacheEntry {
                    ok: true,
                    secs: ran.secs,
                    flaky: ran.flaky.clone(),
                    tail,
                    at: super::unix_now(),
                    run: ctx.run_id.clone(),
                    tier: spec.tier,
                };
                store(cache, &spec.repo_dir, key, entry).await;
            } else if !elsewhere_logged {
                elsewhere_logged = true;
                tracing::info!(
                    dir = %spec.dir.display(),
                    tree = %key.tree,
                    "tier steps ran on a checkout that is not the judged tree, or is dirty: not cached"
                );
            }
        }
        outcome.steps.push(ran);
    }
    Ok(outcome)
}

/// Which keys are green hits, every one looked up before anything runs, on one bounded
/// blocking thread. A step without a key misses; anything that goes wrong is a miss.
async fn lookup_all(
    cache: &TestCache,
    repo_dir: &std::path::Path,
    keys: &[Option<CacheKey>],
) -> Vec<bool> {
    let n = keys.len();
    if keys.iter().all(Option::is_none) {
        return vec![false; n];
    }
    let (cache, repo_dir, keys) = (cache.clone(), repo_dir.to_path_buf(), keys.to_vec());
    bounded(SLACK, move || {
        Ok(keys
            .iter()
            .map(|key| {
                key.as_ref()
                    .is_some_and(|k| cache.lookup(&repo_dir, k).is_some())
            })
            .collect())
    })
    .await
    .unwrap_or_else(|_| vec![false; n])
}

/// Records a green step on a bounded blocking thread. A failure costs a later re-run
/// only, so it is logged and ignored.
async fn store(cache: &TestCache, repo_dir: &std::path::Path, key: CacheKey, entry: CacheEntry) {
    let (cache, repo_dir) = (cache.clone(), repo_dir.to_path_buf());
    let stored = bounded(SLACK, move || {
        cache
            .store(&repo_dir, key, entry)
            .map_err(|error| error.to_string())
    })
    .await;
    if let Err(error) = stored {
        tracing::debug!(%error, "could not store a green tier step in the test cache");
    }
}

/// Decision 31: a cached step, not run. Its flakes were recorded when it ran.
fn hit(step: &Step) -> StepOutcome {
    StepOutcome {
        kind: step.kind,
        command: step.command.clone(),
        ok: true,
        code: Some(0),
        timed_out: false,
        secs: 0,
        cached: true,
        retried: false,
        failing: Vec::new(),
        flaky: Vec::new(),
        granted: 0,
    }
}

/// Decision 22 (with ruling C-5) on the job's change, and decision 9's graph note.
async fn affected_set(
    job: &Job<'_>,
    spec: &TierSpec,
    scope: Scope,
) -> Result<(Affected, Option<String>), Stop> {
    if scope == Scope::Full {
        return Ok((Affected::Full(FULL_SUITE.to_string()), None));
    }
    let changed = changed_paths(job, spec).await?;
    affected_of(job, spec, &changed).await
}

/// The paths the job's change touches: decision 15's `<diff_base>...<head>` for tier 1,
/// decision 16's `<diff_base> <head>` for tier 2.
async fn changed_paths(job: &Job<'_>, spec: &TierSpec) -> Result<Vec<String>, Stop> {
    let range = if spec.tier == 1 {
        format!("{}...{}", spec.diff_base, spec.head)
    } else {
        format!("{} {}", spec.diff_base, spec.head)
    };
    let root = job.root.clone();
    job.read(move |g, t| git::changed_paths(g, &root, &range, t))
        .await
        .map_err(stop)
}

/// Decision 22 on `changed`, with the graph read in the job's checkout (ruling C-5:
/// with no `check`, a known graph runs the module tests of every module). A graph
/// command refused for a changed gate script stops the job (milestone 9.13).
async fn affected_of(
    job: &Job<'_>,
    spec: &TierSpec,
    changed: &[String],
) -> Result<(Affected, Option<String>), Stop> {
    let graph = read_graph(job, spec).await?;
    let note = match &graph {
        GraphState::Unknown(reason) => Some(reason.clone()),
        GraphState::Known(_) => None,
    };
    let mut set = affected(
        changed,
        &spec.profile,
        &spec.hub,
        &spec.source,
        &spec.modules,
        &graph,
    );
    if spec.check.is_none()
        && matches!(set, Affected::Full(_))
        && let GraphState::Known(graph) = &graph
    {
        set = Affected::Modules(graph.modules.keys().cloned().collect());
    }
    Ok((set, note))
}

/// Milestone 9.13 decision 12 (ruling C5): `OpResult::GateRefused` when a blocking
/// closure's own check refused its command (`Err(path)`), else what the command gave.
fn gated<T>(checked: Result<T, String>) -> Result<T, Stop> {
    checked.map_err(|path| Box::new(OpResult::GateRefused { path }))
}

/// Decision 9's graph in the job's checkout, confined as its commands are, with its own
/// isolated `TMPDIR` (M9.1.7's notes: `manifests` and `env` passed through). Milestone
/// 9.13 decision 12 (ruling C5): a command graph first checks the pinned gate scripts
/// on the closure's blocking thread, and a changed one stops the job unrun.
async fn read_graph(job: &Job<'_>, spec: &TierSpec) -> Result<GraphState, Stop> {
    let git = job.git.clone();
    let (dir, repo_dir) = (job.dir.clone(), spec.repo_dir.clone());
    let (profile, modules, manifests) = (
        spec.profile.clone(),
        spec.modules.clone(),
        spec.manifests.clone(),
    );
    let (env, confine) = (job.env.clone(), job.ctx.confine.as_deref().cloned());
    let (common, base, name) = (
        job.common.clone(),
        job.base.clone(),
        format!("s{}-graph", job.op),
    );
    let command = matches!(spec.profile.module_graph, GraphSource::Command(_));
    let pinned = job.ctx.pinned.clone();
    let bound = GRAPH_TIMEOUT + job.ctx.git_timeout * 4 + SLACK;
    let read = bounded(bound, move || {
        if command && let Err(path) = gate_files::verify(&dir, &pinned) {
            return Ok(Err(path));
        }
        let tmp = fresh_dir(&common, &base, &name)?;
        let mut env = env;
        env.extend(isolation_env(&tmp));
        let graph = module_graph(
            &git,
            &dir,
            &repo_dir,
            &profile,
            &modules,
            &manifests,
            &env,
            confine.as_ref(),
        );
        remove_dir(&tmp);
        Ok(Ok(graph))
    })
    .await;
    match read {
        Ok(checked) => gated(checked),
        Err(error) => Ok(GraphState::Unknown(error)),
    }
}

/// Decision 11's toolchain id, run in the job's checkout. Milestone 9.13 decision 12
/// (ruling C5): the pinned gate scripts are checked first, on the closure's blocking
/// thread, and a changed one stops the job unrun.
async fn run_toolchain(job: &Job<'_>, command: String) -> Result<String, Stop> {
    let (dir, env, confine) = (
        job.dir.clone(),
        job.env.clone(),
        job.ctx.confine.as_deref().cloned(),
    );
    let (common, base, name) = (
        job.common.clone(),
        job.base.clone(),
        format!("s{}-toolchain", job.op),
    );
    let pinned = job.ctx.pinned.clone();
    let ran = bounded(TOOLCHAIN_TIMEOUT + SLACK, move || {
        if let Err(path) = gate_files::verify(&dir, &pinned) {
            return Ok(Err(path));
        }
        let tmp = fresh_dir(&common, &base, &name)?;
        let mut env = env;
        env.extend(isolation_env(&tmp));
        let id = toolchain(&dir, &command, &env, confine.as_ref());
        remove_dir(&tmp);
        Ok(Ok(id))
    })
    .await;
    match ran {
        Ok(checked) => gated(checked),
        Err(_) => Ok(TOOLCHAIN_UNKNOWN.to_string()),
    }
}

/// Executes `OpKind::TestAt` (decision 36): the probe's checkout at its commit, then
/// each command at `FullStage` priority on half the slots, a failure retried once
/// (decision 33) unless it timed out. Red when any command failed twice. `show` is the
/// probed commit's `show --stat` when it is a merge (task M9.1.15).
pub(crate) async fn run_test_at(
    ctx: &OpCtx,
    sched: &TestScheduler,
    queue: &GitQueue,
    git: &OsStr,
    op: OpId,
    spec: &TestAtSpec,
) -> OpResult {
    let job = Job::new(
        ctx,
        (sched, queue, git),
        op,
        JobSpec {
            root: &spec.root,
            dir: &spec.dir,
            env: &spec.env,
            timeout_secs: spec.timeout_secs,
            priority: Priority::FullStage,
            critical: false,
        },
    )
    .await;
    let job = match job {
        Ok(job) => job,
        Err(error) => return failed(error),
    };
    let scratch = ScratchAt {
        root: spec.root.clone(),
        commit: spec.commit.clone(),
        setup: spec.setup.clone(),
    };
    if let Err(result) = job.prepare(&scratch).await {
        return *result;
    }
    let grant = job
        .acquire(Want::Half, false, format!("probe at {}", spec.commit))
        .await;
    let (mut failing, mut tail) = (Vec::new(), String::new());
    for (n, command) in spec.commands.iter().enumerate() {
        let at = (n + 1).to_string();
        let mut last = match job.run(&at, command, &grant, false).await {
            Ok(run) => run.outcome,
            Err(result) => return *result,
        };
        if !last.ok && !last.timed_out {
            last = match job.run(&at, command, &grant, false).await {
                Ok(run) => run.outcome,
                Err(result) => return *result,
            };
        }
        if !last.ok {
            failing.push(command.clone());
            tail = last.tail;
        }
    }
    drop(grant);
    // Decision 37 (task M9.1.15): a merge's `show --stat`, which names the culprit's
    // change in a bisect fix task's brief. A failed read leaves it out.
    let (root, commit) = (spec.root.clone(), spec.commit.clone());
    let show = job
        .read(move |g, t| git::show_stat(g, &root, &commit, t))
        .await
        .ok()
        .flatten();
    OpResult::TestAt {
        red: !failing.is_empty(),
        failing,
        tail,
        show,
    }
}

// The cache keys and what decides a hit or a store, split out for the 530-line budget
// (milestone 9.14, ruling C18).
#[path = "tier_keys.rs"]
mod keys;
use keys::{cache_ctx, checkout_is_at, step_keys, usable_hits};

// Decision 16's pre-check of a candidate's tier-2 job (task M9.1.13), split out for
// the 600-line rule.
#[path = "tier_cached.rs"]
mod cached;
pub(crate) use cached::cached_outcome;

#[cfg(test)]
#[path = "tier_tests.rs"]
mod tests;
