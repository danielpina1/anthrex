//! The pushed run snapshot (decision 47, spec §16.5). Pure — no `std::fs`,
//! `std::process`, `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2).

use std::collections::BTreeMap;

use proto::{
    AgentRole, AgentRoundInfo, BaseMovedInfo, CheckInfo, PlanEditInfo, ProofInfo, ReviewInfo,
    RunInfo, RunUsage, RunsSnapshot, Severity, Spend, TaskEventInfo, TaskInfo, TokenUsage,
};

use super::contract::sha7;
use super::engine::EngineState;
use super::engine::actions::{ActionNode, available};
use super::engine::ladder::{round_spend, total_spend};
use super::engine::schedule::{critical_path, readers_busy, waves, writers_busy};
use super::messages::summary;
use super::model::{AgentRound, Run, Task};
pub use super::snapshot_orch::PAUSED_ATTENTION_SECS;
pub use super::snapshot_orch::SNAPSHOT_NOTE_MAX;
use super::snapshot_orch::{message_line, noted_lines, paused_line, plan_text_shown, task_notes};

/// History entries a task shows, newest first.
const HISTORY_SHOWN: usize = 10;

/// Plan edits a run shows, newest first (milestone 8c).
const PLAN_EDITS_SHOWN: usize = 10;

/// Every run, newest first, at the engine's revision. Every time in it is raw unix
/// seconds; `now` is the clients' time base (milestone 8c decision 2).
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
        now,
        proposals: Vec::new(),
        idle_orchestrators: super::chain::idle_list(&state.chains, &state.runs),
    }
}

fn run_info(run: &Run, now: u64) -> RunInfo {
    let path: Vec<usize> = critical_path(run);
    let waves = waves(run);
    let tasks = run
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let text = plan_text_shown(run, t);
            let mut info = task_info(t, path.contains(&i), waves[i], now, text);
            // Milestone 9.0.6 decision 7: each node's actions, built by the daemon.
            info.actions = available(run, &ActionNode::Task(t.id()));
            // Milestone 9.2 decision 41: a review fix's text names its threads' authors.
            info.fixes = t.fixes.as_ref().map(|f| super::engine::fix_text(run, f));
            info
        })
        .collect();
    RunInfo {
        actions: available(run, &ActionNode::Run),
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
        unconfined_checks: run.limits.unconfined_checks,
        trusted_project: run.trusted_project.clone(),
        rate_limits: run.rate_limits.clone(),
        tasks,
        critical_path: path.iter().map(|&i| run.tasks[i].spec.id.clone()).collect(),
        // Decision 42i: the run view's list also names the last discoveries and risks,
        // which the digest carries as `task_notes`.
        attention: [attention(run, now), noted_lines(run)].concat(),
        report_path: run.report_path(),
        outcome: run.outcome.clone(),
        created_at: run.created_at,
        // Milestone 8b: `scouts` is filled by milestone 9's run scouts.
        path: run.path,
        triage: run.triage.clone(),
        promote_requested_at: run.promote_requested_at,
        profile_source: run.profile_source,
        usage: Some(run_usage(run)),
        scouts: Vec::new(),
        // Milestone 8c; the estimates (M9.5) are placeholders.
        approved_at: run.approved_at,
        plan_edits: run
            .plan_edits
            .iter()
            .rev()
            .take(PLAN_EDITS_SHOWN)
            // Milestone 9 decision 40: sources, refusals and recipients.
            .map(|e| PlanEditInfo {
                at: e.at,
                text: e.text.clone(),
                source: e.source.clone(),
                accepted: e.accepted,
                error: e.error.clone(),
                recipients: e.recipients.clone(),
            })
            .collect(),
        plan_edits_since_approval: run.plan_edits_since_approval,
        planners: super::snapshot_orch::planners(run),
        estimate_left_secs: None,
        bound_ratio_permille: None,
        // Milestone 9: the orchestrator and holds (task M9.7), the integration reviews
        // and the research report (task M9.9).
        orchestrator: super::snapshot_orch::orchestrator(run),
        holds: super::snapshot_orch::holds(run),
        integration: super::snapshot_orch::integration(run),
        digest_revision: run.orch.digest_rev,
        research_report: super::snapshot_orch::research_report(run),
        // Milestone 9.1 decision 55; each stage's actions (milestone 9.0.6 decision 7)
        // here, not in `stage_infos`, which the orchestrator's digest shares.
        stages: super::snapshot_stages::stage_infos(run)
            .into_iter()
            .map(|mut s| {
                s.actions = available(run, &ActionNode::Stage(s.n));
                // Milestone 9.2 decision 41; here too, so the digest's stages stay 9.1's.
                s.pr = super::delivery::snapshot::stage_pr_info(run, s.n);
                s
            })
            .collect(),
        test_slots: run.test_slots,
        delivery: super::delivery::snapshot::delivery_info(run),
        chain: run.chain.clone(),
        round: run.round(),
        rounds: run.round_infos(),
        writer_caps: Default::default(),
    }
}

/// M8b decision 29: usage by role. Worker and reviewer rounds from their streams,
/// deciders with triage, run scouts, and the orchestrator from OTLP. Every role is
/// listed, and every sum saturates: OTLP totals come from any local process.
pub(crate) fn run_usage(run: &Run) -> RunUsage {
    let mut by_role: BTreeMap<String, TokenUsage> = [
        "worker",
        "reviewer",
        "scout",
        "decider",
        "orchestrator",
        "planner",
    ]
    .into_iter()
    .map(|role| (role.to_string(), TokenUsage::default()))
    .collect();
    let mut credit = |role: &str, u: TokenUsage| *by_role.entry(role.to_string()).or_default() += u;
    for round in run.tasks.iter().flat_map(|t| &t.rounds) {
        let role = match round.role {
            // Milestone 9.5: racers and test writers are worker sessions.
            AgentRole::Worker | AgentRole::Racer | AgentRole::TestWriter => "worker",
            AgentRole::Reviewer => "reviewer",
            AgentRole::Scout => "scout",
            AgentRole::Orchestrator => "orchestrator",
            AgentRole::Planner => "planner",
            // A decider has no rounds (decision 43); its usage is `decider_usage`.
            AgentRole::Decider => "decider",
        };
        credit(role, round.usage);
    }
    credit("decider", run.decider_usage);
    credit("decider", run.triage_usage);
    credit("scout", run.scout_usage);
    credit("orchestrator", run.orchestrator_usage);
    // Milestone 9 decision 32: sub-planners' sessions.
    credit("planner", run.orch.planner_usage);
    let mut total = TokenUsage::default();
    for u in by_role.values() {
        total += *u;
    }
    RunUsage {
        total,
        by_role,
        decider_calls: run.decider_calls,
        decider_fallbacks: run.decider_fallbacks,
    }
}

/// One line per thing the user must look at at `now`: blocked tasks (a
/// `paused(message)` one only after [`PAUSED_ATTENTION_SECS`], milestone 9 decision
/// 42c), a moved base, a failed final check.
pub(crate) fn attention(run: &Run, now: u64) -> Vec<String> {
    let mut lines: Vec<String> = run
        .tasks
        .iter()
        .filter_map(|t| {
            if crate::run::edits_state::is_paused(t) {
                return paused_line(t, now);
            }
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
    // Milestone 9.1 decision 19: a tiered run's red tier 3 names its stage instead.
    let full = crate::run::engine::full_attention(run);
    if run.final_check_failed && full.is_empty() {
        lines.push("final check failed on the run head".to_string());
    }
    lines.extend(full);
    lines.extend(crate::run::engine::propagate_attention(run));
    lines.extend(crate::run::engine::undelivered_lines(run));
    lines.extend(run.stale_profile_line());
    lines.extend(crate::run::engine::integration_attention(run));
    lines.extend(crate::run::engine::delivery::attention(run));
    // Milestone 9 decision 13: the orchestrator could not start, or its window exited.
    let terminal = run.state.is_terminal();
    lines.extend(
        run.orch
            .orchestrator
            .as_ref()
            .and_then(|o| o.attention(terminal)),
    );
    // A held wake-up; milestone 9.5 decisions 38 and 39: a start prompt.
    lines.extend(crate::run::orch::orchestrator_lines(&run.orch, terminal));
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
        rate_limited_since: r.rate_limited_since,
        rate_limited_until: r.rate_limited_until,
        sent_back_at: r.sent_back_at.clone(),
        lane: None,
        failed_error: None,
        failed_until: None,
    }
}

/// The current worker session's spend: its tool calls, seconds since it started, and
/// billable tokens (decision 40).
fn session_spend(task: &Task, now: u64) -> Spend {
    task.rounds
        .iter()
        .rev()
        .find(|r| r.role == AgentRole::Worker)
        .map(|r| round_spend(r, task.clock.stopped, now))
        .unwrap_or_default()
}

fn task_info(t: &Task, on_critical_path: bool, wave: u32, now: u64, plan_text: bool) -> TaskInfo {
    let done = t.done.as_ref();
    TaskInfo {
        actions: Vec::new(),
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
                lane: None,
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
            decider_summary: c
                .summary
                .clone()
                .filter(|_| c.summary_source == Some(proto::DeciderSource::Decider)),
            summary_source: c.summary_source,
            tier: c.tier.as_ref().map(tier_info),
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
            .map(|e| TaskEventInfo {
                at: e.at,
                text: e.text.clone(),
            })
            .collect(),
        decider_usage: (t.decider_usage != Default::default()).then_some(t.decider_usage),
        size_check: match &t.size_check {
            Some(crate::run::model::SizeCheckState::Done(info)) => Some(info.clone()),
            _ => None,
        },
        // M8b decisions 31 and 32.
        diff: t.diff,
        phases: (t.phases != Default::default()).then_some(t.phases),
        block_source: t.block_source,
        // Decision 16a: only at the gate.
        brief: if plan_text {
            t.spec.brief.clone()
        } else {
            String::new()
        },
        acceptance: if plan_text {
            t.spec.acceptance.clone()
        } else {
            Vec::new()
        },
        route_spec: if plan_text {
            t.spec.route.clone()
        } else {
            Default::default()
        },
        hold: t.orch.gate_hold.clone(),
        review_target: None,
        research_bytes: None,
        // Decision 42d: counts and one line, never message texts.
        message_count: u32::try_from(t.orch.messages.len()).unwrap_or(u32::MAX),
        last_message_kind: t.orch.messages.last().map(|m| m.kind),
        last_message_line: message_line(t),
        task_notes: task_notes(t),
        // Milestone 9.1 decision 55: the last tier record of any check.
        stage: t.spec.stage,
        origin: t.origin,
        // Set by `run_info`, which has the run (milestone 9.2 decision 41).
        fixes: None,
        tier: t
            .checks
            .iter()
            .rev()
            .find_map(|c| c.tier.as_ref())
            .map(tier_info),
        weakening: super::engine::weakening::signal_infos(t),
        // Milestone 9.0.5 decision 2: the live round's only.
        activity: super::snapshot_detail::live_activity(t),
        atomic: t.spec.atomic,
        atomic_reason: t.spec.atomic_reason.clone(),
        interface_change: t.spec.interface_change,
        round: t.round,
        race: None,
        pair: None,
    }
}

/// A tier record as a client sees it (decision 55).
fn tier_info(t: &super::model::TierRecord) -> proto::TierInfo {
    proto::TierInfo {
        tier: t.tier,
        affected: t.affected.clone(),
        steps: t.steps,
        cached: t.cached,
        ok: t.ok,
        secs: t.secs,
        flaky: t.flaky.clone(),
    }
}

/// Decision 16a's test: 50 complete runs of 20 tasks encode under 1.25 KiB a task
/// (the decision's 256 KiB cannot hold; Implementation notes, M9.6).
#[cfg(test)]
const SNAPSHOT_BOUND: usize = 50 * 20 * 1280;

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "snapshot_tests_stages.rs"]
mod tests_stages;
