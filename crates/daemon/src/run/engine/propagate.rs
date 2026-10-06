//! Milestone 9.1 decisions 49–53, engine side: each stage's head flows into the stage
//! above it. Whenever stage `k`'s head moves and stage `k + 1` exists, `k + 1` is due
//! (`Run.propagate_due`); the merge queue takes the lowest due propagate before its
//! next task (`merge::start_merge`), and `OpKind::Propagate` merges stage `k`'s head
//! into stage `k + 1` as a merge candidate is merged. A conflict becomes a `sync` fix
//! task whose worktree holds the merge before its first session; a red propagate waits
//! for a head to move, the orchestrator or the user. Pure (design decision 2).

use proto::{RouteSpec, RunState, Size, TaskOrigin, TaskState, TestMode};

use super::fixes::{FixSpec, add_fix, next_fix_id};
use super::requests::log;
use super::{Effect, OpId, OpKind, OpResult, emit_op, merge, next_op, stages};
use crate::run::contract::{sha7, sync_fix_acceptance, sync_fix_brief, sync_fix_title};
use crate::run::env::profile_env;
use crate::run::model::{
    FixOf, PropagateSpec, Run, StageLayout, StageMerge, StageRecord, SyncState,
};

// Decision 51's sync task: its hand-back, its claims and its merge.
#[path = "propagate_sync.rs"]
mod sync;
pub(crate) use sync::sync_merged;
pub(super) use sync::{
    hand_back_first, hand_back_lost, handed_back, lost_merge, record_hand_back, start_of, sync_due,
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

/// Stage `n` does not hold the lower stage's head (`synced_from`): it needs a
/// propagate. Controller ruling C-21 (1a): this, not only a recorded due entry, is what
/// makes a stage due, so a sync task that finishes without merging leaves it due again.
/// A record without `synced_from` (one built by hand, or cleared by a rebaseline, ruling
/// C-22) holds the lower head only when it is still at what it was created from, and
/// that is the lower head.
fn needs(run: &Run, n: u16) -> bool {
    let (Some(to), Some(from)) = (run.stage(n), n.checked_sub(1).and_then(|k| run.stage(k))) else {
        return false;
    };
    match to.synced_from.as_deref() {
        Some(synced) => synced != from.head,
        None => to.head != to.created_from || to.created_from != from.head,
    }
}

/// Whether the propagate into stage `n` may start now: it needs one, it is not red on
/// the lower head, and no sync task of `n` is open.
fn startable(run: &Run, n: u16) -> bool {
    let Some(from) = n.checked_sub(1).and_then(|k| run.stage(k)) else {
        return false;
    };
    let red = run
        .stage(n)
        .is_some_and(|to| to.propagate_red.as_deref() == Some(from.head.as_str()));
    needs(run, n) && !red && !sync_open(run, n)
}

/// A `Propagate` is in flight: the merge queue is busy (decision 49).
pub(super) fn in_flight(run: &Run) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::Propagate(_)))
}

/// A propagate is in flight or may start: the queue is not idle (decision 17(b)).
pub(super) fn busy(run: &Run) -> bool {
    in_flight(run) || run.stages.iter().any(|s| startable(run, s.n))
}

/// Controller ruling C-21's invariant: the merged tasks the highest stage does not hold,
/// by the stage they merged into, lowest first. Empty for a `Single` run, whose one
/// stage holds every merge.
pub(crate) fn undelivered(run: &Run) -> Vec<(u16, Vec<String>)> {
    let top = stages::highest(run);
    let Some(record) = run
        .stage(top)
        .filter(|_| run.stage_layout == StageLayout::Multi)
    else {
        return Vec::new();
    };
    let mut out: Vec<(u16, Vec<String>)> = Vec::new();
    let missing = run
        .tasks
        .iter()
        .filter(|t| t.state == TaskState::Merged && !record.tasks_in.contains(t.id()));
    for task in missing {
        match out.iter_mut().find(|(k, _)| *k == task.stage()) {
            Some((_, ids)) => ids.push(task.id().to_string()),
            None => out.push((task.stage(), vec![task.id().to_string()])),
        }
    }
    out.sort_by_key(|(k, _)| *k);
    out
}

/// Controller ruling C-21: a run completes delivered only when the highest stage holds
/// every merged task. Completion waits for a propagate in flight, and, unless `run
/// cancel` gives up, for every due one, a red one, and the invariant; the `finish` edit
/// does not end the wait.
pub(super) fn holds_completion(run: &Run) -> bool {
    if in_flight(run) {
        return true;
    }
    if run.cancelled {
        return false;
    }
    busy(run) || !attention(run).is_empty() || !undelivered(run).is_empty()
}

/// Controller ruling C-21 (1c): the invariant's attention lines, while the run waits to
/// complete on it (every task finished, or the `finish` edit).
pub(crate) fn undelivered_lines(run: &Run) -> Vec<String> {
    let waiting = run.finish_edit || run.tasks.iter().all(|t| t.state.is_finished());
    if run.state.is_terminal() || run.cancelled || !waiting {
        return Vec::new();
    }
    let top = stages::highest(run);
    undelivered(run)
        .into_iter()
        .map(|(k, ids)| {
            format!(
                "stage {k}'s merged work is not in stage {top} yet: {}; it cannot be delivered until it is (anthrex run cancel gives up)",
                ids.join(", ")
            )
        })
        .collect()
}

/// Decision 49, called first by `merge::start_merge` when no merge is in flight: the
/// lowest stage that may start gets its `Propagate` (none once `run cancel` gives up).
/// `propagate_due` keeps the stages that still need one. `true` when an op was emitted.
pub(super) fn start(run: &mut Run, fx: &mut Vec<Effect>) -> bool {
    if run.stage_layout != StageLayout::Multi || run.state != RunState::Running {
        return false;
    }
    let due: Vec<u16> = run.propagate_due.iter().copied().collect();
    for n in due {
        if !needs(run, n) {
            run.propagate_due.remove(&n);
        }
    }
    if run.cancelled {
        return false;
    }
    let all: Vec<u16> = run.stages.iter().map(|s| s.n).collect();
    // Milestone 9.2 decision 33: a due base sync of the stage goes first.
    let base_due = |n: u16| run.delivery.base_sync_due.contains_key(&n);
    let free = |n| startable(run, n) && !super::delivery::stage_busy(run, n) && !base_due(n);
    let Some(n) = all.into_iter().find(|&n| free(n)) else {
        return false;
    };
    run.propagate_due.remove(&n);
    emit(run, n, fx);
    true
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

/// A `Propagate`'s result (decisions 50–53). `from: 0` is 9.2's base sync, handled by
/// `delivery::sync` after its tier-2 records.
pub(super) fn done(
    run: &mut Run,
    (op, spec): (OpId, &PropagateSpec),
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
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
        // Ruling C-23: a propagate's tier 2 is a tier job (decisions 50 and 57).
        super::history::tier_job(run, (op, None, spec.to), outcome, now, fx);
    }
    if spec.from == 0 {
        return super::delivery::sync::done(run, (op, spec), result, now, fx);
    }
    let (k, n) = (spec.from, spec.to);
    match result {
        OpResult::Merged { commit, .. } => landed(run, spec, &commit, now),
        OpResult::AlreadyHeld => held(run, spec, now),
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
/// bisect culprit, decision 36), and it holds what stage `k` held. On a stage whose
/// PR merged, the landing is judged late, as a task's merge is (FW-O1).
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
    // FW-O1: a propagate landing on a stage whose PR merged is judged as a task merge.
    super::delivery::land_judge::task_merged(run, n, now);
}

/// Controller ruling C-22 (2): stage `n` already holds stage `k`'s head. No commit was
/// made: its head and line stay, and it records that it holds that head and its work.
fn held(run: &mut Run, spec: &PropagateSpec, now: u64) {
    let (k, n) = (spec.from, spec.to);
    let Some(record) = stage_mut(run, n) else {
        return;
    };
    record.propagate_red = None;
    record.propagate_note = None;
    record.tasks_in.extend(spec.tasks.iter().cloned());
    record.synced_from = Some(spec.from_head.clone());
    let text = format!(
        "stage {n}: already holds stage {k} at {}",
        sha7(&spec.from_head)
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
    // Ruling C-27 (M-3): each conflicted path is owned exactly, never as a glob that
    // could name other files; a path no `owns` entry can name holds the propagate red.
    let mut owns = Vec::with_capacity(files.len());
    for file in &files {
        let entry = crate::run::globs::escape_path(file);
        if let Err(why) = crate::run::globs::validate_glob(&entry) {
            let line =
                format!("{what}: the conflicted path {file} cannot be owned exactly ({why})");
            return refused(run, spec, line, now);
        }
        owns.push(entry);
    }
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
        owns,
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
            handed: Vec::new(),
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
/// A lost base sync (`from: 0`) is due again as a base sync (9.2 ruling R-7).
pub(super) fn lost(run: &mut Run, spec: &PropagateSpec) {
    if spec.from == 0 {
        return super::delivery::sync::lost(run, spec);
    }
    run.propagate_due.insert(spec.to);
}
