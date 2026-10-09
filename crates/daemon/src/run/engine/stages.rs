//! Milestone 9.1 decisions 46–48 and 53, engine side: the layout fixed at approval,
//! the one writer of stage heads (and so of `run_head`), which merged work a task's
//! stage holds, the creation of stage branches, the guard list and the rebaseline.
//! Pure (design decision 2).

use std::collections::BTreeSet;

use proto::{RunState, TaskState};

use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, goal_rounds, merge, next_op};
use crate::run::contract::sha7;
use crate::run::model::{Run, StageLayout, StageMerge, StageRecord, Task};

/// What `run resume --rebaseline` read (decision 21; milestone 9.1 decision 47): the
/// base head, the `integration` head and, for a `Multi` run, each created stage's head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rebaseline {
    pub base: String,
    pub head: String,
    pub stages: Vec<(u16, String)>,
    /// Controller ruling C-15 (I-1): the commit `integration` was at when the driver
    /// moved it back to the highest stage's head, and the salvage ref that keeps it.
    pub salvaged: Option<(String, String)>,
}

impl From<(String, String)> for Rebaseline {
    fn from((base, head): (String, String)) -> Self {
        Rebaseline {
            base,
            head,
            ..Rebaseline::default()
        }
    }
}

/// The highest created stage's number (0 when none is).
pub(crate) fn highest(run: &Run) -> u16 {
    run.stages.iter().map(|s| s.n).max().unwrap_or(0)
}

/// Decision 47: the one writer of stage heads and of `run_head`, which is always the
/// head of the highest created stage. A `Single` run's one stage is `run_head` itself.
pub(crate) fn set_stage_head(run: &mut Run, n: u16, commit: &str) {
    let top = highest(run);
    if let Some(record) = run.stages.iter_mut().find(|s| s.n == n) {
        record.head = commit.to_string();
    }
    if run.stage_layout == StageLayout::Single || n >= top {
        run.run_head = commit.to_string();
    }
    // Decision 50: the stage above is due a propagate.
    super::propagate::head_moved(run, n);
}

/// Decision 47: a merge of task `id` landed on stage `n` at `commit`. The stage's head
/// moves, it holds the task's work (decision 48's `tasks_in`), the commit joins its
/// first-parent line, and it is the stage's last green candidate, which `Run` keeps
/// for the highest stage (decision 37's final check).
pub(crate) fn task_merged(run: &mut Run, n: u16, id: Option<&str>, commit: &str) {
    set_stage_head(run, n, commit);
    let top = n >= highest(run) || run.stage_layout == StageLayout::Single;
    if let Some(record) = run.stages.iter_mut().find(|s| s.n == n) {
        record.last_green_candidate = Some(commit.to_string());
        if let Some(id) = id {
            record.tasks_in.insert(id.to_string());
            record.merges.push(StageMerge::Task {
                id: id.to_string(),
                commit: commit.to_string(),
            });
        }
    }
    if top {
        run.last_green_candidate = Some(commit.to_string());
    }
    // Decision 51: a sync task's merge brings the lower stage's work with it.
    if let Some(id) = id {
        super::propagate::sync_merged(run, n, id);
    }
}

/// The stage a merge into `branch` lands on: 1 for a `Single` run, else the stage
/// whose branch it is (the highest when none is, which cannot happen).
pub(crate) fn stage_of_branch(run: &Run, branch: &str) -> u16 {
    match run.stage_layout {
        StageLayout::Single => 1,
        StageLayout::Multi => run
            .stages
            .iter()
            .find(|s| s.branch == branch)
            .map_or_else(|| highest(run), |s| s.n),
    }
}

/// Decision 53's guard list: `integration` alone for a `Single` run (M8a's), every
/// created stage branch and `integration` for a `Multi` one.
pub(crate) fn guard_list(run: &Run) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if run.stage_layout == StageLayout::Multi {
        out.extend(
            run.stages
                .iter()
                .map(|s| (s.branch.clone(), s.head.clone())),
        );
    }
    out.push((run.run_branch(), run.run_head.clone()));
    out
}

/// Decision 47 for a run from before milestone 9.1: its one stage, on `integration`
/// at `run_head`.
pub(super) fn ensure_first(run: &mut Run) {
    if run.stage_layout == StageLayout::Single && run.stages.is_empty() {
        let record = StageRecord::new(
            1,
            run.run_branch(),
            &run.run_head,
            BTreeSet::new(),
            run.created_at,
        );
        run.stages.push(record);
    }
}

/// Decision 46: the layout, fixed when the plan is first approved (the gate, `--yes`,
/// the fast path). `Multi` when any task has a stage above 1: its stage 1 is then
/// created from `run_head` by the next running pass ([`create_pass`]), so no stage is
/// recorded until its branch exists.
pub(super) fn fix_layout(run: &mut Run, now: u64) {
    // Milestone 9.3 decision 13: round 1's approval only; a later round's stages were
    // laid out when it started (`goal_rounds::widen`).
    if run.round() > 1 {
        return;
    }
    let multi = run.tasks.iter().any(|t| t.stage() > 1);
    if !multi {
        run.stage_layout = StageLayout::Single;
        return;
    }
    run.stage_layout = StageLayout::Multi;
    run.stages.clear();
    let top = run.tasks.iter().map(Task::stage).max().unwrap_or(1);
    log(
        run,
        now,
        format!(
            "approved with {top} stages: branches anthrex/{}/stage-<n>",
            run.id
        ),
    );
}

/// A `CreateStageBranch` is in flight.
pub(super) fn creating(run: &Run) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::CreateStageBranch { .. }))
}

/// Whether dependency `dep` of `task` in stage `n` is in that stage's head: a reported
/// task merged nothing, a cancelled implicit one is no dependency, a merge of the same
/// stage always is, and one of an earlier stage is when `n` holds it (decision 48). A
/// task of an earlier round is met (milestone 9.3 decision 13).
fn dep_in(run: &Run, task: &Task, dep: &str, n: u16, record: &StageRecord) -> bool {
    let Some(d) = run.task(dep) else {
        return true;
    };
    if goal_rounds::dep_met(run, task, d) {
        return true;
    }
    match d.state {
        TaskState::Reported | TaskState::Cancelled => true,
        TaskState::Merged => d.stage() == n || record.tasks_in.contains(dep),
        _ => false,
    }
}

/// Decision 48: task `i` may run in its stage: always in a `Single` run; in a `Multi`
/// one, once its stage branch exists and every merged dependency of an earlier stage
/// is in that stage's head.
pub(crate) fn ready_in_stage(run: &Run, i: usize) -> bool {
    // Milestone 9.2 decision 37: a stage paused by a closed PR below starts nothing.
    if super::delivery::stage_paused(run, run.tasks[i].stage()) {
        return false;
    }
    if run.stage_layout == StageLayout::Single {
        return true;
    }
    let task = &run.tasks[i];
    let n = task.stage();
    let Some(record) = run.stage(n) else {
        return false;
    };
    task.spec
        .deps
        .iter()
        .chain(&task.implicit_deps)
        .all(|d| match run.task(d) {
            // Milestone 9.3 decision 13: an earlier round's work is in every stage.
            Some(dep) if goal_rounds::dep_met(run, task, dep) => true,
            Some(dep) if dep.state == TaskState::Merged => {
                dep.stage() == n || record.tasks_in.contains(d)
            }
            _ => true,
        })
}

/// Decision 48, every running pass of a `Multi` run: stage 1 from `run_head` first;
/// then stage `n + 1` from the highest stage `n`'s head once one of its unfinished tasks
/// would be runnable there. One at a time, and never beside a merge, so `integration`
/// (the highest stage's alias) is always at the head a new stage starts from.
pub(super) fn create_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    // Milestone 9.6 decision 23: no stage before round 1's documents commit lands.
    if run.stage_layout != StageLayout::Multi
        || run.state != RunState::Running
        || creating(run)
        || merge::merging(run)
        || super::design_commit::holds_stages(run)
    {
        return;
    }
    let top = highest(run);
    let next = top + 1;
    // Milestone 9.3 decision 13: a `pr` round above landed PRs fetches the base first.
    if goal_rounds::awaits_base(run, next) {
        return;
    }
    let from = if top == 0 {
        run.run_head.clone()
    } else {
        let Some(record) = run.stage(top) else {
            return;
        };
        let live = |t: &&Task| !t.state.is_finished();
        let ready = run.tasks.iter().filter(live).any(|t| {
            t.stage() == next
                && t.spec
                    .deps
                    .iter()
                    .chain(&t.implicit_deps)
                    .all(|d| dep_in(run, t, d, next, record))
        });
        // A stage whose tasks all finished unstarted (cancelled) is still created,
        // empty, when a later stage has work: stages stay contiguous (decision 44).
        let passed = !run.tasks.iter().filter(live).any(|t| t.stage() == next)
            && run.tasks.iter().filter(live).any(|t| t.stage() > next);
        if !ready && !passed {
            return;
        }
        record.head.clone()
    };
    // Milestone 9.6 (task M9.6.15): a later round's documents commit is its first stage.
    if super::design_commit::round_stage(run, next, &from, now, fx) {
        return;
    }
    let kind = OpKind::CreateStageBranch {
        root: run.root.clone(),
        branch: run.stage_branch(next),
        from,
    };
    let op = next_op(run);
    emit_op(run, op, None, kind, fx);
}

/// A `CreateStageBranch`'s result. Created, the stage is recorded: it starts with the
/// work its parent holds, and it is now the highest, so `run_head` (and `integration`,
/// already there) is its head. A branch found elsewhere halts the run (decision 21);
/// a failure halts it too, retryable by a plain `run resume`, since the next pass
/// would only fail again.
pub(super) fn created(run: &mut Run, kind: &OpKind, result: OpResult, now: u64) {
    let OpKind::CreateStageBranch { branch, from, .. } = kind else {
        return;
    };
    match result {
        OpResult::StageCreated => {
            let n = branch
                .rsplit_once("/stage-")
                .and_then(|(_, n)| n.parse::<u16>().ok())
                .unwrap_or_else(|| highest(run) + 1);
            let parent = run.stage(n.saturating_sub(1));
            // Milestone 9.3 decision 13: a widened run's stage 1 starts at `run_head`,
            // which holds every task merged before it.
            let merged = || {
                let merged = run.tasks.iter().filter(|t| t.state == TaskState::Merged);
                merged.map(|t| t.id().to_string()).collect()
            };
            let tasks_in = parent.map_or_else(merged, |p| p.tasks_in.clone());
            let mut record = StageRecord::new(n, branch.clone(), from, tasks_in, now);
            record.synced_from = parent.map(|_| from.clone());
            // Milestone 9.3 decision 13: the round whose stages hold it.
            record.round = run.round_of_stage(n);
            run.stages.push(record);
            set_stage_head(run, n, from);
            log(run, now, format!("stage {n}: {branch} at {}", sha7(from)));
        }
        OpResult::RefMoved { reason } => merge::halt(run, reason, now),
        OpResult::Failed { message } => {
            merge::halt(run, format!("could not create {branch}: {message}"), now);
            run.halt_retryable = true;
        }
        _ => {}
    }
}

/// `run resume --rebaseline` (decision 21): the refs the driver read become the run's.
/// A `Multi` run takes every stage head from its ref, then `run_head` from the highest
/// (decision 47); the `integration` head it read is for a `Single` run. Returns the log
/// text, which names the `run_head` it set (controller ruling C-15) and, when the
/// driver moved `integration` back, both commits and the salvage ref.
pub(super) fn rebaseline(
    run: &mut Run,
    read: &Rebaseline,
    now: u64,
    fx: &mut Vec<Effect>,
) -> String {
    let before: Vec<(u16, String)> = run.stages.iter().map(|s| (s.n, s.head.clone())).collect();
    rebaseline_heads(run, read);
    // Ruling C-27 (3): a stage whose head the rebaseline moved forgets what it knew of
    // its old line.
    for (n, was) in before {
        if run.stage(n).is_some_and(|s| s.head != was) {
            moved_line(run, n, now, fx);
        }
    }
    let mut text = format!(
        " with --rebaseline: base {} at {}, run head {}",
        run.base_branch,
        sha7(&run.base_sha),
        sha7(&run.run_head)
    );
    if let Some((old, salvage)) = &read.salvaged {
        text.push_str(&format!(
            "; {} moved back from {} to {} ({} kept at {salvage})",
            run.run_branch(),
            sha7(old),
            sha7(&run.run_head),
            sha7(old)
        ));
    }
    text
}

/// Ruling C-27 (3), for stage `n` whose head the rebaseline moved: its first-parent
/// record ends at the new head (up to and including the merge that is the new head;
/// empty when the head is the stage's creation point, or a commit the engine never
/// wrote there, which becomes the stage's `floor`, ruling C-28 (1)), a bisect in flight ends `rebaselined` (its pending probe dropped, so a
/// late result is ignored, and no fix task is ever made from it), and a green or red
/// tier 3 on a commit no longer on the line is forgotten, so tier 3 is due for the new
/// head by the ordinary rules.
fn moved_line(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    forget_line(run, n, now, fx);
    log(
        run,
        now,
        format!(
            "stage {n}: rebaselined to {}",
            sha7(&run.stage(n).map(|s| s.head.clone()).unwrap_or_default())
        ),
    );
}

/// [`moved_line`] without its log line: also milestone 9.2's adoption of a user's
/// commit (decision 24, ruling R-9), whose head the engine never wrote on the line.
pub(super) fn forget_line(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    super::bisect::rebaselined(run, n, now, fx);
    let Some(record) = run.stages.iter_mut().find(|s| s.n == n) else {
        return;
    };
    let commit = |m: &StageMerge| match m {
        StageMerge::Task { commit, .. } | StageMerge::Propagate { commit, .. } => commit.clone(),
    };
    match record.merges.iter().position(|m| commit(m) == record.head) {
        // Ruling C-28 (1): a truncation to a recorded merge keeps the floor.
        Some(k) => record.merges.truncate(k + 1),
        None => {
            record.merges.clear();
            // Ruling C-28 (1): a head the engine never wrote on the line is its new
            // floor; the creation point needs none.
            record.floor = (record.head != record.created_from).then(|| record.head.clone());
        }
    }
    let on_line = |c: &String, record: &StageRecord| {
        *c == record.created_from
            || record.floor.as_ref() == Some(c)
            || *c == record.head
            || record.merges.iter().any(|m| commit(m) == *c)
    };
    if record
        .full
        .green_at
        .as_ref()
        .is_some_and(|c| !on_line(c, record))
    {
        record.full.green_at = None;
    }
    if record
        .full
        .red_at
        .as_ref()
        .is_some_and(|c| !on_line(c, record))
    {
        record.full.red_at = None;
        record.full.note = None;
    }
    // Milestone 9.9: an accepted red is forgotten with the line it was accepted on.
    if (record.full.accepted_red.as_ref()).is_some_and(|c| !on_line(c, record)) {
        record.full.accepted_red = None;
    }
}

fn rebaseline_heads(run: &mut Run, read: &Rebaseline) {
    run.base_sha = read.base.clone();
    run.base_moved = None;
    match run.stage_layout {
        StageLayout::Single => set_stage_head(run, 1, &read.head),
        StageLayout::Multi => {
            let before: Vec<(u16, String)> =
                run.stages.iter().map(|s| (s.n, s.head.clone())).collect();
            for (n, head) in &read.stages {
                set_stage_head(run, *n, head);
            }
            // Controller ruling C-22 (2): a stage whose head, or whose lower stage's
            // head, the user moved may no longer hold the lower head: its propagate
            // runs again (and is recorded as held when nothing is missing).
            let moved = |run: &Run, n: u16| {
                let was = before
                    .iter()
                    .find(|(m, _)| *m == n)
                    .map(|(_, h)| h.as_str());
                run.stage(n).is_some_and(|s| Some(s.head.as_str()) != was)
            };
            let cleared: Vec<u16> = run
                .stages
                .iter()
                .map(|s| s.n)
                .filter(|&n| n > 1 && (moved(run, n) || moved(run, n - 1)))
                .collect();
            for n in cleared {
                if let Some(record) = run.stages.iter_mut().find(|s| s.n == n) {
                    record.synced_from = None;
                }
            }
        }
    }
}
