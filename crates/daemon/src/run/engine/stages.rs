//! Milestone 9.1 decisions 46–48 and 53, engine side: the layout fixed at approval,
//! the one writer of stage heads (and so of `run_head`), which merged work a task's
//! stage holds, the creation of stage branches, the guard list and the rebaseline.
//! Pure (design decision 2).

use std::collections::BTreeSet;

use proto::{RunState, TaskState};

use super::requests::log;
use super::{Effect, OpKind, OpResult, emit_op, merge, next_op};
use crate::run::contract::sha7;
use crate::run::model::{Run, StageLayout, StageMerge, StageRecord, Task};

/// What `run resume --rebaseline` read (decision 21; milestone 9.1 decision 47): the
/// base head, the `integration` head and, for a `Multi` run, each created stage's head.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rebaseline {
    pub base: String,
    pub head: String,
    pub stages: Vec<(u16, String)>,
}

impl From<(String, String)> for Rebaseline {
    fn from((base, head): (String, String)) -> Self {
        Rebaseline {
            base,
            head,
            stages: Vec::new(),
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

/// Whether dependency `dep` of a task in stage `n` is in that stage's head: a reported
/// task merged nothing, a cancelled implicit one is no dependency, a merge of the same
/// stage always is, and one of an earlier stage is when `n` holds it (decision 48).
fn dep_in(run: &Run, dep: &str, n: u16, record: &StageRecord) -> bool {
    let Some(d) = run.task(dep) else {
        return true;
    };
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
pub(super) fn create_pass(run: &mut Run, fx: &mut Vec<Effect>) {
    if run.stage_layout != StageLayout::Multi
        || run.state != RunState::Running
        || creating(run)
        || merge::merging(run)
    {
        return;
    }
    let top = highest(run);
    let next = top + 1;
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
                    .all(|d| dep_in(run, d, next, record))
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
            let tasks_in = parent.map(|p| p.tasks_in.clone()).unwrap_or_default();
            let mut record = StageRecord::new(n, branch.clone(), from, tasks_in, now);
            record.synced_from = parent.map(|_| from.clone());
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
/// (decision 47); the `integration` head it read is for a `Single` run.
pub(super) fn rebaseline(run: &mut Run, read: &Rebaseline) {
    run.base_sha = read.base.clone();
    run.base_moved = None;
    match run.stage_layout {
        StageLayout::Single => set_stage_head(run, 1, &read.head),
        StageLayout::Multi => {
            for (n, head) in &read.stages {
                set_stage_head(run, *n, head);
            }
        }
    }
}
