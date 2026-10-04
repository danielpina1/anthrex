//! M8b decision 33: the records of `history.jsonl`, built from the run model, and
//! which of them are due. Pure (M8b decision 1); `history_io.rs` reads and writes the
//! file, and the engine (`engine/history.rs`) emits the ops.

use proto::{
    AgentRole, GateTally, HISTORY_VERSION, RunRecord, RunState, Severity, SeverityTally,
    TaskOutcome, TaskRecord, TaskState, TokenUsage, Verdict,
};

use super::contract::generated_files_message;
use super::model::{Run, SizeCheckState, Task};
use super::phases::phase_mut;

/// A run record keeps the goal's first 200 characters.
const GOAL_CHARS: usize = 200;

/// `<run>/<task>`.
pub fn task_record_id(run_id: &str, task_id: &str) -> String {
    format!("{run_id}/{task_id}")
}

/// Whether the run writes history: only a run started with it (`Run.history`, from
/// milestone 8b.16 on), and never a run from milestone 8a, which has no repository
/// data directory. A run started before has no phases, diffs or routing decisions.
pub fn enabled(run: &Run) -> bool {
    run.history && !run.repo_dir.as_os_str().is_empty()
}

/// The outcome of a run whose records are due: `accepted`, `discarded`, `failed`, or
/// `complete` with a task neither merged nor cancelled. A complete run whose tasks all
/// finished waits for its accept or discard.
pub fn run_outcome(run: &Run) -> Option<&'static str> {
    match run.state {
        RunState::Accepted => Some("accepted"),
        RunState::Discarded => Some("discarded"),
        RunState::Failed => Some("failed"),
        RunState::Complete if run.tasks.iter().any(|t| !t.state.is_finished()) => Some("complete"),
        _ => None,
    }
}

/// A task's outcome by its state.
pub fn outcome(task: &Task) -> TaskOutcome {
    match task.state {
        TaskState::Merged if task.merged_without_approval.is_some() => {
            TaskOutcome::MergedWithoutApproval
        }
        TaskState::Merged => TaskOutcome::Merged,
        TaskState::Cancelled => TaskOutcome::Cancelled,
        TaskState::Blocked => TaskOutcome::Blocked,
        // Milestone 9 decisions 35 and 36: finished, with nothing merged.
        TaskState::Reported => TaskOutcome::Reported,
        _ => TaskOutcome::Unfinished,
    }
}

/// The tasks with no record yet whose record is due now, with their outcome: a merged
/// or cancelled task at once, and every other one when the run has ended. None when
/// history is off for the run.
pub fn due(run: &Run) -> Vec<(usize, TaskOutcome)> {
    if !enabled(run) {
        return Vec::new();
    }
    let ended = run_outcome(run).is_some();
    run.tasks
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.history_written && (ended || t.state.is_finished()))
        .map(|(i, t)| (i, outcome(t)))
        .collect()
}

/// The run record's outcome, once the run has ended and every task's record was
/// emitted, until the run record itself is.
pub fn run_record_due(run: &Run) -> Option<&'static str> {
    if !enabled(run) || run.run_record_written || run.tasks.iter().any(|t| !t.history_written) {
        return None;
    }
    run_outcome(run)
}

fn sum(task: &Task, role: AgentRole) -> TokenUsage {
    let mut total = TokenUsage::default();
    for round in task.rounds.iter().filter(|r| r.role == role) {
        total += round.usage;
    }
    total
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A task's gate tally. Milestone 9.5 minor m3: a red-only check (decision 25's, of a
/// test writer's claim) is not a proof.
pub(crate) fn gates(task: &Task) -> GateTally {
    let own = || task.checks.iter().filter(|c| !c.on_candidate);
    let proofs = || task.proofs.iter().filter(|p| !p.red_only);
    // A generated-file bounce (decision 55) is known by its message's opening words.
    let message = generated_files_message(&[]);
    let generated = &message[..message.find(" outside").unwrap_or(message.len())];
    GateTally {
        proofs: count(proofs().count()),
        proofs_failed: count(
            proofs()
                .filter(|p| !(p.red_failed && p.head_passed && p.matched))
                .count(),
        ),
        checks: count(own().count()),
        checks_failed: count(own().filter(|c| !c.ok).count()),
        review_rounds: count(task.reviews.len()),
        reviews_rejected: count(
            task.reviews
                .iter()
                .filter(|r| r.verdict == Some(Verdict::Changes))
                .count(),
        ),
        candidates_red: count(
            task.checks
                .iter()
                .filter(|c| c.on_candidate && !c.ok)
                .count(),
        ),
        generated_bounces: count(
            task.failure_log
                .iter()
                .filter(|text| text.starts_with(generated))
                .count(),
        ),
    }
}

fn severities(task: &Task) -> SeverityTally {
    let mut tally = SeverityTally::default();
    for finding in task.reviews.iter().flat_map(|r| r.findings.iter()) {
        let slot = match finding.severity {
            Severity::Critical => &mut tally.critical,
            Severity::Important => &mut tally.important,
            Severity::Minor => &mut tally.minor,
        };
        *slot += 1;
    }
    tally
}

/// The record of a finished task, or, at the run's end, of an unfinished one. Its
/// phases include the time in its current state up to `now`; `wall_secs` is their sum.
pub fn task_record(run: &Run, task: &Task, outcome: TaskOutcome, now: u64) -> TaskRecord {
    let mut phases = task.phases;
    if task.phase_since > 0
        && let Some(open) = phase_mut(&mut phases, task.state)
    {
        *open = open.saturating_add(now.saturating_sub(task.phase_since));
    }
    let wall_secs = [
        phases.queued,
        phases.preparing,
        phases.working,
        phases.proof,
        phases.check,
        phases.review,
        phases.merge,
        phases.blocked,
    ]
    .iter()
    .fold(0u64, |a, b| a.saturating_add(*b));
    TaskRecord {
        v: HISTORY_VERSION,
        record_id: task_record_id(&run.id, task.id()),
        at: now,
        run_id: run.id.clone(),
        task_id: task.id().to_string(),
        path: run.path,
        kind: task.spec.kind,
        hub: task.hub,
        test_mode: task.test_mode,
        planned_size: task.spec.size,
        final_size: task.size,
        size_check: match &task.size_check {
            Some(SizeCheckState::Done(info)) => Some(info.clone()),
            _ => None,
        },
        route: task.route.clone(),
        review_routes: task.reviews.iter().map(|r| r.route.clone()).collect(),
        routing_decisions: task.routing_decisions.clone(),
        outcome,
        block: task.block.as_ref().map(|b| b.reason),
        diff: task.diff,
        tool_calls: task
            .rounds
            .iter()
            .filter(|r| r.role == AgentRole::Worker)
            .fold(0u32, |a, r| a.saturating_add(r.tool_calls)),
        worker_usage: sum(task, AgentRole::Worker),
        reviewer_usage: sum(task, AgentRole::Reviewer),
        decider_usage: task.decider_usage,
        phases,
        wall_secs,
        gates: gates(task),
        severities: severities(task),
        bounces: task.bounces,
        failures: task.failures,
        stalls: task.stalls,
        budget_exceeded: task.budget_exceeded,
        conflicts: task.conflicts,
        max_rung: task.max_rung.max(task.rung),
        sessions: task.session,
        done_signal: task.done.as_ref().map(|d| d.signal),
        merge_commit: task.merge_commit.clone(),
        stage: task.spec.stage,
        origin: proto::TaskOrigin::Plan,
        pattern: None,
        race_winner: None,
        race_adopted: false,
        writer_failures: 0,
        round: 0,
    }
}

/// The record of a run that ended. `accepted_commit` is left for the driver, which
/// reads the base branch just before appending it.
pub fn run_record(run: &Run, outcome: &str, now: u64) -> RunRecord {
    RunRecord {
        v: HISTORY_VERSION,
        record_id: run.id.clone(),
        at: now,
        run_id: run.id.clone(),
        goal: run.goal.chars().take(GOAL_CHARS).collect(),
        path: run.path,
        triage: run.triage.clone(),
        profile_source: run.profile_source,
        outcome: outcome.to_string(),
        base_branch: run.base_branch.clone(),
        accepted_commit: None,
        tasks: count(run.tasks.len()),
        usage: Some(super::snapshot::run_usage(run)),
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;
