//! The pushed run snapshot (decision 47, spec §16.5). Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).

use proto::{
    AgentRole, AgentRoundInfo, BaseMovedInfo, CheckInfo, ProofInfo, ReviewInfo, RunInfo,
    RunsSnapshot, Severity, Spend, TaskInfo,
};

use super::contract::sha7;
use super::engine::EngineState;
use super::engine::ladder::{round_spend, total_spend};
use super::engine::schedule::{critical_path, readers_busy, waves, writers_busy};
use super::messages::summary;
use super::model::{AgentRound, Run, Task};

/// History entries a task shows, newest first.
const HISTORY_SHOWN: usize = 10;

/// Every run, newest first, at the engine's revision.
pub fn snapshot(state: &EngineState, now: u64) -> RunsSnapshot {
    let mut runs: Vec<RunInfo> = state.runs.values().map(|r| run_info(r, now)).collect();
    runs.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(a.run_id.cmp(&b.run_id))
    });
    RunsSnapshot {
        revision: state.revision,
        runs,
    }
}

fn run_info(run: &Run, now: u64) -> RunInfo {
    let path: Vec<usize> = critical_path(run);
    let waves = waves(run);
    let tasks = run
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| task_info(t, path.contains(&i), waves[i], now))
        .collect();
    RunInfo {
        run_id: run.id.clone(),
        goal: run.goal.clone(),
        project: run.project.clone(),
        root: run.root.clone(),
        state: run.state,
        paused_from: run.paused_from,
        halted_reason: run.halted_reason.clone(),
        approved_by: run.approved_by.clone(),
        base_branch: run.base_branch.clone(),
        base_sha: run.base_sha.clone(),
        run_branch: run.run_branch(),
        run_head: run.run_head.clone(),
        base_moved: run.base_moved.as_ref().map(|m| BaseMovedInfo {
            from: m.from.clone(),
            to: m.to.clone(),
            commits: Vec::new(),
            total: m.commits,
        }),
        revision: run.revision,
        max_writers: run.limits.max_writers,
        max_readers: run.limits.max_readers,
        max_bounces: run.limits.max_bounces,
        writers_busy: u8::try_from(writers_busy(run)).unwrap_or(u8::MAX),
        readers_busy: u8::try_from(readers_busy(run)).unwrap_or(u8::MAX),
        unverified: run.unverified,
        worker_sandbox: run.limits.worker_sandbox,
        trusted_project: run.trusted_project.clone(),
        rate_limits: run.rate_limits.clone(),
        tasks,
        critical_path: path.iter().map(|&i| run.tasks[i].spec.id.clone()).collect(),
        attention: attention(run),
        report_path: run.report_path(),
        outcome: run.outcome.clone(),
        created_at: run.created_at,
    }
}

/// One line per thing the user must look at: blocked tasks, a moved base, a failed
/// final check.
fn attention(run: &Run) -> Vec<String> {
    let mut lines: Vec<String> = run
        .tasks
        .iter()
        .filter_map(|t| {
            let block = t.block.as_ref()?;
            let reason = serde_json::to_value(block.reason).ok()?;
            Some(format!(
                "{} blocked ({}): {}",
                t.spec.id,
                reason.as_str().unwrap_or_default(),
                block.text
            ))
        })
        .collect();
    if let Some(moved) = &run.base_moved {
        lines.push(format!(
            "base {} moved from {} to {} ({} new commits); accept will list them",
            run.base_branch,
            sha7(&moved.from),
            sha7(&moved.to),
            moved.commits
        ));
    }
    if run.final_check_failed {
        lines.push("final check failed on the run head".to_string());
    }
    lines
}

fn round_info(r: &AgentRound, now: u64) -> AgentRoundInfo {
    AgentRoundInfo {
        role: r.role,
        session: r.session,
        round: r.round,
        window_id: r.window_id,
        route: r.route.clone(),
        session_id: r.session_id.clone(),
        started_at: r.started_at,
        ended_at: r.ended_at,
        tool_calls: r.tool_calls,
        last_event: r.last_event,
        turn_open: r.turn_open,
        turns: r.turns,
        rate_limited: r.rate_limited_until.is_some_and(|t| t > now),
        open_subagents: u32::try_from(r.open_subagents.len()).unwrap_or(u32::MAX),
        denials: r.denials,
        usage: r.usage,
    }
}

/// The current worker session's spend: its tool calls, seconds since it started, and
/// billable tokens (decision 40).
fn session_spend(task: &Task, now: u64) -> Spend {
    task.rounds
        .iter()
        .rev()
        .find(|r| r.role == AgentRole::Worker)
        .map(|r| round_spend(r, task.clock_stopped, now))
        .unwrap_or_default()
}

/// `<hh:mm>` in UTC.
fn clock(at: u64) -> String {
    format!("{:02}:{:02}", (at / 3600) % 24, (at / 60) % 60)
}

fn task_info(t: &Task, on_critical_path: bool, wave: u32, now: u64) -> TaskInfo {
    let done = t.done.as_ref();
    TaskInfo {
        id: t.spec.id.clone(),
        title: t.spec.title.clone(),
        epic: t.spec.epic.clone(),
        kind: t.spec.kind,
        size: t.size,
        hub: t.hub,
        test_mode: t.test_mode,
        test_mode_reason: t.spec.test_mode_reason.clone(),
        notes: t.notes.clone(),
        owns: t.spec.owns.clone(),
        deps: t.spec.deps.clone(),
        implicit_deps: t.implicit_deps.clone(),
        priority: t.spec.priority,
        route: t.route.clone(),
        review_route: t.review_route.clone(),
        budget: t.budget,
        spent_session: session_spend(t, now),
        spent_total: total_spend(t, now),
        state: t.state,
        block: t.block.clone(),
        rung: t.rung,
        failures: t.failures,
        bounces: t.bounces,
        stalls: t.stalls,
        budget_exceeded: t.budget_exceeded,
        conflicts: t.conflicts,
        branch: t.branch.clone(),
        worktree: t.worktree.clone(),
        start_commit: t.start_commit.clone(),
        head: t.head.clone(),
        test: done.and_then(|d| d.test.clone()),
        red: done.and_then(|d| d.red.clone()),
        done_signal: done.map(|d| d.signal),
        rounds: t.rounds.iter().map(|r| round_info(r, now)).collect(),
        reviews: t
            .reviews
            .iter()
            .map(|r| ReviewInfo {
                round: r.round,
                route: r.route.clone(),
                verdict: r.verdict,
                summary: r.summary.clone(),
                findings: r.findings.clone(),
                blocking: r.findings.iter().any(|f| f.severity != Severity::Minor),
            })
            .collect(),
        last_check: t.checks.last().map(|c| CheckInfo {
            at: c.at,
            ok: c.ok,
            code: c.code,
            timed_out: c.timed_out,
            secs: c.secs,
            summary: summary(&c.tail),
            on_candidate: c.on_candidate,
        }),
        last_proof: t.proofs.last().map(|p| ProofInfo {
            at: p.at,
            test: p.test.clone(),
            red: p.red.clone(),
            red_failed: p.red_failed,
            head_passed: p.head_passed,
            matched: p.matched,
            ok: p.red_failed && p.head_passed && p.matched,
        }),
        merge_commit: t.merge_commit.clone(),
        merged_without_approval: t.merged_without_approval.clone(),
        salvage_refs: t.salvage_refs.clone(),
        on_critical_path,
        wave,
        history: t
            .history
            .iter()
            .rev()
            .take(HISTORY_SHOWN)
            .map(|e| format!("{} {}", clock(e.at), e.text))
            .collect(),
    }
}
