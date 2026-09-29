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
//! Every git read, command, directory creation and removal runs on `spawn_blocking`
//! with a timeout; every git write goes through the run's [`GitQueue`]. No lock is held
//! here, and no slot is held across anything but its own step.

use std::ffi::OsStr;

use super::OpCtx;
use super::graph::{module_graph, toolchain};
use super::tier_step::{
    Job, JobSpec, SLACK, Stop, bounded, failed, fresh_dir, isolation_env, remove_dir, run_step,
    stop,
};
use crate::run::engine::{OpResult, ScratchAt};
use crate::run::git::{self, GitQueue};
use crate::run::model::OpId;
use crate::run::slots::{Priority, TestScheduler, Want};
use crate::run::tiers::affected::affected;
use crate::run::tiers::steps::plan;
use crate::run::tiers::{
    Affected, GRAPH_TIMEOUT, GraphState, Scope, Step, StepKind, TOOLCHAIN_TIMEOUT, TestAtSpec,
    TierOutcome, TierSpec,
};

/// The affected set a tier-3 job records: one fixed value, so a tier-3 cache key never
/// depends on a reason text (M9.1.6's note for task M9.1.14).
pub(crate) const FULL_SUITE: &str = "full suite";
/// The affected set of an untiered profile's single `check`.
pub(crate) const UNTIERED: &str = "untiered profile";

/// Executes `OpKind::Tier` (decisions 14, 16, 18, 20–28, 32, 33).
pub(crate) async fn run_tier(
    ctx: &OpCtx,
    sched: &TestScheduler,
    queue: &GitQueue,
    git: &OsStr,
    op: OpId,
    spec: &TierSpec,
) -> OpResult {
    match tier_job(ctx, (sched, queue, git), op, spec).await {
        Ok(outcome) => OpResult::Tier(Box::new(outcome)),
        Err(result) => *result,
    }
}

async fn tier_job(
    ctx: &OpCtx,
    shared: (&TestScheduler, &GitQueue, &OsStr),
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
            outcome.toolchain = Some(run_toolchain(&job, command).await);
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
    for (k, step) in steps.iter().enumerate() {
        let (ran, tail) = run_step(&job, spec, tiered, k + 1, step).await?;
        outcome.secs += ran.secs;
        let red = !ran.ok;
        outcome.steps.push(ran);
        if red {
            outcome.ok = false;
            outcome.tail = tail;
            break;
        }
    }
    Ok(outcome)
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
    let range = if spec.tier == 1 {
        format!("{}...{}", spec.diff_base, spec.head)
    } else {
        format!("{} {}", spec.diff_base, spec.head)
    };
    let root = job.root.clone();
    let changed = job
        .read(move |g, t| git::changed_paths(g, &root, &range, t))
        .await
        .map_err(stop)?;
    let graph = read_graph(job, spec).await;
    let note = match &graph {
        GraphState::Unknown(reason) => Some(reason.clone()),
        GraphState::Known(_) => None,
    };
    let mut set = affected(
        &changed,
        &spec.profile,
        &spec.hub,
        &spec.source,
        &spec.modules,
        &graph,
    );
    // Ruling C-5: with no `check`, a known graph runs the module tests of every module.
    if spec.check.is_none()
        && matches!(set, Affected::Full(_))
        && let GraphState::Known(graph) = &graph
    {
        set = Affected::Modules(graph.modules.keys().cloned().collect());
    }
    Ok((set, note))
}

/// Decision 9's graph in the job's checkout, confined as its commands are, with its own
/// isolated `TMPDIR` (M9.1.7's notes: `manifests` and `env` passed through).
async fn read_graph(job: &Job<'_>, spec: &TierSpec) -> GraphState {
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
    let bound = GRAPH_TIMEOUT + job.ctx.git_timeout * 4 + SLACK;
    let read = bounded(bound, move || {
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
        Ok(graph)
    })
    .await;
    read.unwrap_or_else(GraphState::Unknown)
}

/// Decision 11's toolchain id, run in the job's checkout.
async fn run_toolchain(job: &Job<'_>, command: String) -> String {
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
    bounded(TOOLCHAIN_TIMEOUT + SLACK, move || {
        let tmp = fresh_dir(&common, &base, &name)?;
        let mut env = env;
        env.extend(isolation_env(&tmp));
        let id = toolchain(&dir, &command, &env, confine.as_ref());
        remove_dir(&tmp);
        Ok(id)
    })
    .await
    .unwrap_or_else(|_| "unknown".to_string())
}

/// Executes `OpKind::TestAt` (decision 36): the probe's checkout at its commit, then
/// each command at `FullStage` priority on half the slots, a failure retried once
/// (decision 33) unless it timed out. Red when any command failed twice.
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
            Err(error) => return failed(error),
        };
        if !last.ok && !last.timed_out {
            last = match job.run(&at, command, &grant, false).await {
                Ok(run) => run.outcome,
                Err(error) => return failed(error),
            };
        }
        if !last.ok {
            failing.push(command.clone());
            tail = last.tail;
        }
    }
    OpResult::TestAt {
        red: !failing.is_empty(),
        failing,
        tail,
        show: None,
    }
}

#[cfg(test)]
#[path = "tier_tests.rs"]
mod tests;
