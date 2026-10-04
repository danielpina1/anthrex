//! Milestone 9.1's tiers 1 and 2 in the engine (task M9.1.13): tier 1 in place of M8a's
//! check for a tiered profile (decision 14, on decision 15's range), the tier-2 job a
//! merge candidate carries (decision 16), and what a tier job's outcome does to the run
//! (decision 9's note, decision 11's toolchain, decision 21's log lines) and to the
//! task's records. An untiered profile never reaches this file (decision 6). Pure
//! (design decision 2).

use proto::{BlockReason, GateKind, TaskState};

use super::dispatch::{block, history};
use super::gates::{awaited, passed};
use super::requests::log;
use super::{Effect, OpId, OpKind, OpResult, ScratchAt, deciders, emit_op, next_op, schedule};
use crate::run::env::profile_env;
use crate::run::model::{CheckRecord, Run, TierRecord};
use crate::run::slots::Priority;
use crate::run::tiers::{Affected, CacheCtx, TierOutcome, TierSpec, graph::note_once};

/// The toolchain part of a cache key when the profile has no `toolchain_id` (decision
/// 11), and the id that turns the cache off.
const NO_TOOLCHAIN: &str = "none";
const UNKNOWN_TOOLCHAIN: &str = "unknown";
/// Decision 21's log lines name at most this many modules.
const NAMES_SHOWN: usize = 5;

/// Whether the run's profile is tiered (decision 6).
pub(super) fn tiered(run: &Run) -> bool {
    run.profile.tiers.is_tiered()
}

/// Decision 14's check condition: the profile has `check`, or it is tiered and has
/// `build_check`, `module_test` or `module_tests`.
pub(super) fn has_check_gate(run: &Run) -> bool {
    let t = &run.profile.tiers;
    run.profile.check.is_some()
        || (tiered(run)
            && (t.build_check.is_some() || t.module_test.is_some() || t.module_tests.is_some()))
}

/// Decision 30's key context: none for an untiered profile, an `"unknown"` toolchain,
/// or a run with no repository data directory to keep the file in.
fn cache_ctx(run: &Run) -> Option<CacheCtx> {
    let toolchain = run.toolchain.as_deref().unwrap_or(NO_TOOLCHAIN);
    if !tiered(run) || toolchain == UNKNOWN_TOOLCHAIN || run.repo_dir.as_os_str().is_empty() {
        return None;
    }
    Some(CacheCtx {
        profile_hash: run.profile_hash.clone(),
        toolchain: toolchain.to_string(),
    })
}

/// Whether the run's tier steps read and write the result cache (decision 30), for the
/// report's line on what a cached result assumes (ruling C-27, M-2).
pub(crate) fn cache_enabled(run: &Run) -> bool {
    cache_ctx(run).is_some()
}

/// What every tier job of the run carries, for tier `tier` on stage `stage` in `dir`.
pub(super) fn spec(run: &Run, tier: u8, stage: u16, dir: std::path::PathBuf) -> TierSpec {
    let p = &run.profile;
    TierSpec {
        tier,
        stage,
        root: run.root.clone(),
        env: profile_env(p, &dir),
        dir,
        scratch: None,
        diff_base: String::new(),
        head: String::new(),
        profile: p.tiers.clone(),
        check: p.check.clone(),
        hub: p.hub.clone(),
        source: p.source.clone(),
        modules: p.modules.clone(),
        // Ruling C-12a (C-8 (5)): the profile's own value, never `[]` in its place.
        manifests: p.manifests.clone(),
        single_test: p.single_test.clone(),
        timeout_secs: p.check_timeout_secs,
        priority: if tier == 2 {
            Priority::Candidate
        } else {
            Priority::Gate
        },
        critical: false,
        cache: cache_ctx(run),
        toolchain: run.toolchain.clone(),
        repo_dir: run.repo_dir.clone(),
    }
}

/// Decision 14: task `i`'s check gate as `Tier { tier: 1 }`, in its `<task>.proof`
/// scratch checkout at the claimed head, measured from its stage's head (decision 15).
pub(super) fn start_tier1(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let id = task.id().to_string();
    let head = task.head.clone().unwrap_or_default();
    let mut job = spec(run, 1, task.stage(), run.proof_path(&task.checkout_name()));
    job.scratch = Some(ScratchAt {
        root: run.root.clone(),
        commit: head.clone(),
        setup: run.profile.setup.clone(),
    });
    job.diff_base = run.head_for(task).to_string();
    job.head = head;
    job.critical = schedule::critical_path(run).contains(&i);
    let op = next_op(run);
    run.tasks[i].gate_op = Some(op);
    emit_op(run, op, Some(&id), OpKind::Tier(Box::new(job)), fx);
    history(run, i, now, "running tier 1");
}

/// Decision 16: the tier-2 job of a candidate onto stage `stage`, measured from
/// `stage_head` (the candidate itself is filled in by the executor); `None` for an
/// untiered profile, or one with nothing to run at a gate.
pub(super) fn tier2_spec(run: &Run, stage: u16, stage_head: &str) -> Option<Box<TierSpec>> {
    if !tiered(run) || !has_check_gate(run) {
        return None;
    }
    let mut job = spec(run, 2, stage, run.integration_path());
    job.diff_base = stage_head.to_string();
    Some(Box::new(job))
}

/// A tier outcome as the task keeps it (decision 55).
pub(super) fn record(outcome: &TierOutcome, now: u64) -> TierRecord {
    let failing = outcome
        .steps
        .iter()
        .rev()
        .find(|s| !s.ok)
        .map(|s| s.failing.clone())
        .unwrap_or_default();
    TierRecord {
        tier: outcome.tier,
        affected: affected_text(&outcome.affected),
        steps: count(outcome.steps.len()),
        cached: count(outcome.steps.iter().filter(|s| s.cached).count()),
        ok: outcome.ok,
        secs: outcome.secs,
        flaky: outcome.steps.iter().flat_map(|s| s.flaky.clone()).collect(),
        failing,
        at: now,
        commit: String::new(),
    }
}

fn count(n: usize) -> u8 {
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// Decision 21's affected part: `3 modules (a, b, c)`, `build only (no module
/// affected)`, `full suite (<reason>)`.
pub(crate) fn affected_text(affected: &Affected) -> String {
    match affected {
        Affected::Full(reason) => format!("full suite ({reason})"),
        Affected::Modules(names) if names.is_empty() => "build only (no module affected)".into(),
        Affected::Modules(names) => {
            let shown: Vec<&str> = names.iter().take(NAMES_SHOWN).map(String::as_str).collect();
            format!("{} modules ({})", names.len(), shown.join(", "))
        }
    }
}

/// Decision 21's log lines for one job, in order.
pub(super) fn lines(outcome: &TierOutcome) -> Vec<String> {
    let n = outcome.tier;
    let mut lines = vec![format!("tier {n}: {}", affected_text(&outcome.affected))];
    let cached = outcome.steps.iter().filter(|s| s.cached).count();
    if cached > 0 {
        lines.push(format!("tier {n}: cached ({cached} steps)"));
    }
    for name in outcome.steps.iter().flat_map(|s| s.flaky.iter()) {
        lines.push(format!("tier {n}: flaky {name} passed on retry"));
    }
    lines
}

/// What any tier job tells the run: decision 11's toolchain id, kept once, and
/// decision 9's unknown-graph note, logged once.
pub(super) fn run_facts(run: &mut Run, outcome: &TierOutcome, now: u64) {
    if run.toolchain.is_none()
        && let Some(id) = &outcome.toolchain
    {
        run.toolchain = Some(id.clone());
    }
    if let Some(reason) = &outcome.graph_note {
        let graph = crate::run::tiers::GraphState::Unknown(reason.clone());
        if let Some(line) = note_once(&mut run.graph_note, &graph) {
            log(run, now, line);
        }
    }
}

/// Task `i`'s history lines for `outcome`, and its check record.
fn task_facts(
    run: &mut Run,
    i: usize,
    outcome: &TierOutcome,
    on_candidate: bool,
    now: u64,
) -> String {
    for line in lines(outcome) {
        history(run, i, now, line);
    }
    let red = outcome.steps.iter().rev().find(|s| !s.ok);
    let command = red.map(|s| s.command.clone()).unwrap_or_default();
    run.tasks[i].checks.push(CheckRecord {
        at: now,
        ok: outcome.ok,
        code: red.map_or(Some(0), |s| s.code),
        timed_out: red.is_some_and(|s| s.timed_out),
        tail: outcome.tail.clone(),
        secs: outcome.secs,
        on_candidate,
        summary: None,
        summary_source: None,
        tier: Some(record(outcome, now)),
        lane: None,
    });
    command
}

/// Decision 14: tier 1's result goes through M8a's check gate. Green passes it; red
/// is a gate failure of `check` whose rung waits for the summary, naming the red step's
/// command; a job that could not run blocks the task on its environment.
pub(super) fn tier1_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if let OpResult::Tier(outcome) = &result {
        run_facts(run, outcome, now);
        // Decision 57: the job's `tier` and `flaky` lines, whoever awaits it.
        let (id, stage) = (run.tasks[i].id().to_string(), run.tasks[i].stage());
        super::history::tier_job(run, (op, Some(&id), stage), outcome, now, fx);
    }
    if !awaited(run, i, op, TaskState::Check) {
        return;
    }
    match result {
        OpResult::Tier(outcome) => {
            let command = task_facts(run, i, &outcome, false, now);
            if outcome.ok {
                passed(run, i, TaskState::Check, now);
            } else {
                deciders::summarise(run, i, GateKind::Check, &command, now, fx);
            }
        }
        OpResult::SetupFailed { output } => {
            let text = format!("setup failed in the check worktree:\n{output}");
            block(run, i, BlockReason::Environment, text, now);
        }
        OpResult::Failed { message } => {
            let text = format!("could not run tier 1: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// Decision 16: a tier-2 outcome on task `i`'s candidate, merged or red: the history
/// lines and the check record (`on_candidate`). Returns the red step's command.
pub(super) fn tier2_facts(run: &mut Run, i: usize, outcome: &TierOutcome, now: u64) -> String {
    task_facts(run, i, outcome, true, now)
}
