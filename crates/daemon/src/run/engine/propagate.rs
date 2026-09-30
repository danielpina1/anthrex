//! Milestone 9.1 decisions 49–53, engine side: each stage's head flows into the stage
//! above it. Whenever stage `k`'s head moves and stage `k + 1` exists, `k + 1` is due
//! (`Run.propagate_due`); the merge queue takes the lowest due propagate before its
//! next task (`merge::start_merge`), and `OpKind::Propagate` merges stage `k`'s head
//! into stage `k + 1` as a merge candidate is merged. A conflict becomes a `sync` fix
//! task whose worktree holds the merge before its first session; a red propagate waits
//! for a head to move, the orchestrator or the user. Pure (design decision 2).

use proto::{BlockReason, RouteSpec, RunState, Size, TaskOrigin, TaskState, TestMode};

use super::dispatch::{block, history};
use super::fixes::{FixSpec, add_fix, next_fix_id};
use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, merge, next_op, stages};
use crate::run::contract::{sha7, sync_fix_acceptance, sync_fix_brief, sync_fix_title};
use crate::run::env::profile_env;
use crate::run::model::{
    FixOf, PropagateSpec, Run, StageLayout, StageMerge, StageRecord, SyncState, Task,
};

/// Decision 52's attention line names at most this many tests, its wake note fewer.
const ATTENTION_TESTS: usize = 5;
const WAKE_TESTS: usize = 3;

/// What a red propagate names when its job gave no test names: M8a's `check` of an
/// untiered profile (invented).
const NO_NAMES: &str = "the check failed";

fn stage_mut(run: &mut Run, n: u16) -> Option<&mut StageRecord> {
    run.stages.iter_mut().find(|s| s.n == n)
}

/// Decision 50: stage `n`'s head moved (called by `stages::set_stage_head`, the one
/// writer). The stage above is due; and a propagate into `n` that was red is due again
/// (decision 52: a move of either stage's head re-enters it).
pub(crate) fn head_moved(run: &mut Run, n: u16) {
    if run.stage_layout != StageLayout::Multi {
        return;
    }
    if run.stage(n + 1).is_some() {
        run.propagate_due.insert(n + 1);
    }
    if let Some(record) = stage_mut(run, n)
        && record.propagate_red.take().is_some()
    {
        record.propagate_note = None;
        run.propagate_due.insert(n);
    }
}

/// An unfinished `sync` fix task of stage `n`, which its propagate waits for (decision
/// 51).
fn sync_open(run: &Run, n: u16) -> bool {
    run.tasks
        .iter()
        .any(|t| t.sync.is_some() && t.stage() == n && !t.state.is_finished())
}

/// Whether the propagate into stage `n` may start now: both stages exist, `n` does not
/// already hold the lower head (`synced_from`), it is not red on that head, and no sync
/// task of `n` is open.
fn startable(run: &Run, n: u16) -> bool {
    let (Some(to), Some(from)) = (run.stage(n), n.checked_sub(1).and_then(|k| run.stage(k))) else {
        return false;
    };
    let head = Some(from.head.as_str());
    to.synced_from.as_deref() != head && to.propagate_red.as_deref() != head && !sync_open(run, n)
}

/// A `Propagate` is in flight: the merge queue is busy (decision 49).
pub(super) fn in_flight(run: &Run) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::Propagate(_)))
}

/// A propagate is in flight or may start: the queue is not idle (decision 17(b)).
pub(super) fn busy(run: &Run) -> bool {
    in_flight(run) || run.propagate_due.iter().any(|&n| startable(run, n))
}

/// Completion waits for every due propagate, so `integration` holds every stage's
/// work, and for a red one until the `finish` edit or `run cancel` (invented, as a red
/// tier 3 holds it: decision 19).
pub(super) fn holds_completion(run: &Run) -> bool {
    busy(run) || (!super::full::ending(run) && !attention(run).is_empty())
}

/// Decision 49, called first by `merge::start_merge` when no merge is in flight: the
/// lowest startable due stage gets its `Propagate`. A due stage that cannot start is
/// dropped: a head move or its sync task's merge enters it again. `true` when an op
/// was emitted.
pub(super) fn start(run: &mut Run, fx: &mut Vec<Effect>) -> bool {
    if run.stage_layout != StageLayout::Multi || run.state != RunState::Running {
        return false;
    }
    let due: Vec<u16> = run.propagate_due.iter().copied().collect();
    for n in due {
        run.propagate_due.remove(&n);
        if startable(run, n) {
            emit(run, n, fx);
            return true;
        }
    }
    false
}

fn emit(run: &mut Run, n: u16, fx: &mut Vec<Effect>) {
    let k = n - 1;
    let (Some(from), Some(to)) = (run.stage(k), run.stage(n)) else {
        return;
    };
    let (from_head, tasks) = (from.head.clone(), from.tasks_in.clone());
    let (to_branch, expected) = (to.branch.clone(), to.head.clone());
    let integration = run.integration_path();
    // Decision 50: tier 2 (a tiered profile) or M8a's `check`, as a candidate's.
    let tier = super::tiers::tier2_spec(run, n, &expected);
    let spec = PropagateSpec {
        root: run.root.clone(),
        env: profile_env(&run.profile, &integration),
        integration,
        from: k,
        to: n,
        from_head: from_head.clone(),
        to_branch,
        expected_to_head: expected,
        also_integration: n >= stages::highest(run),
        base_branch: run.base_branch.clone(),
        expected_base: run.base_sha.clone(),
        guarded: stages::guard_list(run),
        message: format!("anthrex: propagate stage-{k} into stage-{n}"),
        check: run.profile.check.clone().filter(|_| tier.is_none()),
        tier,
        timeout_secs: run.profile.check_timeout_secs,
        tasks,
    };
    let op = next_op(run);
    emit_op(run, op, None, OpKind::Propagate(Box::new(spec)), fx);
}

/// A `Propagate`'s result (decisions 50–53). `from: 0` is 9.2's base sync, whose
/// handler 9.2 adds here.
pub(super) fn done(
    run: &mut Run,
    spec: &PropagateSpec,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if spec.from == 0 {
        return;
    }
    if let OpResult::Merged {
        tier: Some(outcome),
        ..
    }
    | OpResult::CandidateRed {
        tier: Some(outcome),
        ..
    } = &result
    {
        super::tiers::run_facts(run, outcome, now);
    }
    let (k, n) = (spec.from, spec.to);
    match result {
        OpResult::Merged { commit, .. } => landed(run, spec, &commit, now),
        OpResult::Conflict { files, tree } => conflicted(run, spec, files, tree, now, fx),
        OpResult::CandidateRed { tier, .. } => {
            let tests = tier.map(|o| super::tiers::record(&o, now).failing);
            red(run, spec, tests.unwrap_or_default(), now);
        }
        // Decision 21: the refs are the run's; after a rebaseline it is due again.
        OpResult::RefMoved { reason } => {
            run.propagate_due.insert(n);
            merge::halt(run, reason, now);
        }
        // Ruling C-18: an executor failure is never red. It halts, retryable by a
        // plain `run resume`, since the next pass would only fail again (invented, as
        // a failed `CreateStageBranch`).
        OpResult::Failed { message } => {
            run.propagate_due.insert(n);
            let reason = format!("could not propagate stage {k} into stage {n}: {message}");
            merge::halt(run, reason, now);
            run.halt_retryable = true;
        }
        _ => {}
    }
}

/// `Merged`: stage `n` holds stage `k`'s head. Its head moves (and `integration` with
/// it when it is the highest), the commit joins its line as a propagate entry (never a
/// bisect culprit, decision 36), and it holds what stage `k` held.
fn landed(run: &mut Run, spec: &PropagateSpec, commit: &str, now: u64) {
    let (k, n) = (spec.from, spec.to);
    if let Some(record) = stage_mut(run, n) {
        record.propagate_red = None;
        record.propagate_note = None;
    }
    stages::task_merged(run, n, None, commit);
    if let Some(record) = stage_mut(run, n) {
        record.merges.push(StageMerge::Propagate {
            from: k,
            commit: commit.to_string(),
        });
        record.tasks_in.extend(spec.tasks.iter().cloned());
        record.synced_from = Some(spec.from_head.clone());
    }
    let text = format!(
        "stage {n}: propagated stage {k} at {} ({})",
        sha7(&spec.from_head),
        sha7(commit)
    );
    log(run, now, text);
}

/// Decision 51: a conflict adds the `sync` fix task to stage `n`, owning the conflicted
/// files exactly. When M8a's rules refuse it (or the result has no tree), the
/// propagate waits as a red one does, and the orchestrator is woken (invented).
fn conflicted(
    run: &mut Run,
    spec: &PropagateSpec,
    files: Vec<String>,
    tree: Option<String>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let (k, n) = (spec.from, spec.to);
    let what = format!("propagate of stage {k} into stage {n} conflicted");
    let Some(tree) = tree else {
        return refused(
            run,
            spec,
            format!("{what}: the conflicted tree is unknown"),
            now,
        );
    };
    let id = next_fix_id(run);
    let fix = FixSpec {
        origin: TaskOrigin::Sync,
        fixes: FixOf::Propagate {
            from: k,
            to: n,
            head: spec.from_head.clone(),
        },
        stage: n,
        title: sync_fix_title(k, n),
        brief: sync_fix_brief(&id, k, n, &files),
        acceptance: sync_fix_acceptance(),
        owns: files,
        size: Size::M,
        epic: None,
        route: RouteSpec::default(),
        test_mode: TestMode::Check,
        test_mode_reason: Some("a merge resolution; the tiers check it".to_string()),
        sync: Some(SyncState {
            onto: spec.from_head.clone(),
            base_tree: tree,
            tasks: spec.tasks.clone(),
            handed_back: false,
            to_head: spec.expected_to_head.clone(),
        }),
    };
    match add_fix(run, fix, now, fx) {
        Ok(id) => {
            let text = format!("{what}: added fix task {id}");
            log(run, now, text.clone());
            super::wake::note(run, text);
        }
        Err(message) => refused(
            run,
            spec,
            format!("{what}: fix task refused: {message}"),
            now,
        ),
    }
}

/// A conflict whose sync task could not be added: the stage waits as for a red
/// propagate, with `line` as its attention line.
fn refused(run: &mut Run, spec: &PropagateSpec, line: String, now: u64) {
    let (k, n) = (spec.from, spec.to);
    mark_red(run, spec, line.clone());
    log(run, now, line.clone());
    super::wake::note(run, format!("{line}; plan a fix in stage {k} or {n}"));
}

/// Decision 52: a red propagate has no single owner. Stage `n` records the lower head
/// it was red on; the attention line and the wake note follow, and nothing retries it
/// until either stage's head moves. A red result on heads that have moved since is
/// only recorded (as ruling C-18's stale red wakes nobody).
fn red(run: &mut Run, spec: &PropagateSpec, tests: Vec<String>, now: u64) {
    let (k, n) = (spec.from, spec.to);
    let names = |count: usize| {
        if tests.is_empty() {
            NO_NAMES.to_string()
        } else {
            let shown: Vec<&str> = tests.iter().take(count).map(String::as_str).collect();
            shown.join(", ")
        }
    };
    let line = format!(
        "propagate of stage {k} into stage {n} is red: {}",
        names(ATTENTION_TESTS)
    );
    let wake = format!(
        "propagate of stage {k} into stage {n} is red: {}; plan a fix in stage {k} or {n}",
        names(WAKE_TESTS)
    );
    log(run, now, line.clone());
    let current = run.stage_head(k) == Some(spec.from_head.as_str())
        && run.stage_head(n) == Some(spec.expected_to_head.as_str());
    if !current {
        return;
    }
    mark_red(run, spec, line);
    super::wake::note(run, wake);
}

fn mark_red(run: &mut Run, spec: &PropagateSpec, line: String) {
    if let Some(record) = stage_mut(run, spec.to) {
        record.propagate_red = Some(spec.from_head.clone());
        record.propagate_note = Some(line);
    }
}

/// Decision 52's attention lines: every stage whose propagate is red on the lower
/// stage's current head.
pub(crate) fn attention(run: &Run) -> Vec<String> {
    run.stages
        .iter()
        .filter_map(|s| {
            let lower = run.stage(s.n.checked_sub(1)?)?;
            if s.propagate_red.as_deref() != Some(lower.head.as_str()) {
                return None;
            }
            s.propagate_note.clone()
        })
        .collect()
}

/// A lost `Propagate` (reconciled `NotStarted`: it reached neither ref) is due again.
pub(super) fn lost(run: &mut Run, spec: &PropagateSpec) {
    run.propagate_due.insert(spec.to);
}

/// A `sync` fix task whose merge is not yet in its worktree (decision 51).
pub(super) fn sync_due(task: &Task) -> bool {
    task.sync.as_ref().is_some_and(|s| !s.handed_back)
}

/// Where a task's worktree starts: a sync task's at the upper stage's head its conflict
/// was found on, so the hand-back's merge is exactly the conflicted tree; any other at
/// its stage's head (decision 47).
pub(super) fn start_of(run: &Run, task: &Task) -> String {
    match &task.sync {
        Some(sync) if !sync.to_head.is_empty() => sync.to_head.clone(),
        _ => run.head_for(task).to_string(),
    }
}

/// Decision 51: before a sync task's first session, M8a's `HandBack` merges the lower
/// stage's head into its worktree (`merge-tree`, the markers and an engine-written
/// `MERGE_HEAD`, never `git merge`). Called where the session would start; `true` when
/// the session waits for it.
pub(super) fn hand_back_first(
    run: &mut Run,
    i: usize,
    start: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    let task = &run.tasks[i];
    let Some(sync) = task.sync.as_ref().filter(|s| !s.handed_back) else {
        return false;
    };
    let id = task.id().to_string();
    let waiting = super::schedule::op_in_flight(run, &id, |k| matches!(k, OpKind::HandBack { .. }));
    if waiting {
        return true;
    }
    let onto = sync.onto.clone();
    let kind = OpKind::HandBack {
        worktree: task.worktree.clone(),
        run_head: onto.clone(),
        task_head: Some(start.to_string()),
        list_merged: false,
    };
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
    let text = format!("merging {} into its worktree first", sha7(&onto));
    history(run, i, now, text);
    true
}

/// The sync task's `HandBack` result: its session starts at the commit the merge was
/// made onto; a failure blocks it (`environment`), as a failed prepare does.
pub(super) fn handed_back(
    run: &mut Run,
    i: usize,
    start: Option<String>,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].state != TaskState::Preparing {
        return;
    }
    match result {
        OpResult::HandedBack { files, onto, .. } => {
            let start = onto
                .or(start)
                .unwrap_or_else(|| start_of(run, &run.tasks[i]));
            if let Some(sync) = run.tasks[i].sync.as_mut() {
                sync.handed_back = true;
            }
            let text = if files.is_empty() {
                "the merge was clean".to_string()
            } else {
                format!("handed back with conflicts in {}", files.join(", "))
            };
            history(run, i, now, text);
            if run.state == RunState::Running {
                super::dispatch::launch_worker(run, i, start, now, fx);
            } else {
                run.tasks[i].ready_from = Some(start);
            }
        }
        OpResult::Failed { message } => {
            let text = format!("could not merge the lower stage into its worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// A lost sync hand-back: the next running pass (`launch_ready`) starts it again.
pub(super) fn hand_back_lost(run: &mut Run, i: usize, start: Option<String>) {
    if run.tasks[i].state == TaskState::Preparing {
        run.tasks[i].ready_from = start;
    }
}

/// Decision 51: a sync task merged into stage `n`, which now holds its `onto` and what
/// that head held; a newer lower head is due again.
pub(crate) fn sync_merged(run: &mut Run, n: u16, id: &str) {
    let Some(sync) = run.task(id).and_then(|t| t.sync.clone()) else {
        return;
    };
    if let Some(record) = stage_mut(run, n) {
        record.tasks_in.extend(sync.tasks);
        record.synced_from = Some(sync.onto);
    }
    run.propagate_due.insert(n);
}
