//! Decision 31 (task M9.2.10): a task the orchestrator (or the user) adds with
//! `PlanTask.addresses` is a review fix. Each ref (`<pr>:<key>`) must be a `new`
//! thread of an open stage PR of the run, and the task's `stage` that PR's stage; the
//! task then takes origin `review` and `Task.fixes = FixOf::Review { stage, pr,
//! threads }`, and its threads become `tasked` by it. In `pr` mode no task is added to
//! a stage whose PR is merged or closed. The approval rule of decision 26 is the
//! engine's, once the batch is accepted (`engine/delivery/review.rs`). Only refs and
//! ids are read here, never comment text. Pure (design decision 1).

use proto::{DeliveryMode, PrState, TaskOrigin};

use super::ThreadState;
use crate::run::model::{FixOf, Run, Task};
use crate::run::orch::EditSource;
use crate::run::plan::PlanError;

fn error(task: &str, text: String) -> PlanError {
    PlanError::new(Some(task), "", "31", text)
}

/// `<pr>:<key>` as its PR number and key.
fn parse(reference: &str) -> Option<(u64, &str)> {
    let (pr, key) = reference.split_once(':')?;
    Some((pr.parse().ok()?, key))
}

/// Decision 31's checks for a task this batch adds, and its review fix when it names
/// threads: the errors, or nothing (the task and its threads updated in place).
pub fn apply(run: &mut Run, task: &mut Task, source: &EditSource) -> Vec<PlanError> {
    let id = task.spec.id.clone();
    let n = task.spec.stage;
    if run.delivery.mode != DeliveryMode::Pr {
        return unknown_refs(&id, n, &task.spec.addresses);
    }
    if let Some(pr) = run.delivery.pr(n).filter(|p| p.state != PrState::Open) {
        let state = match pr.state {
            PrState::Merged => "merged",
            _ => "closed",
        };
        let text = format!("stage {n}'s PR is {state}; add the task to a later stage");
        return vec![error(&id, text)];
    }
    if task.spec.addresses.is_empty() {
        return Vec::new();
    }
    if matches!(source, EditSource::Planner { .. }) {
        let text = format!("task {id}: a sub-planner cannot address review threads");
        return vec![error(&id, text)];
    }
    let Some(pr) = run.delivery.pr(n).map(|p| p.number) else {
        return unknown_refs(&id, n, &task.spec.addresses);
    };
    let mut errors = Vec::new();
    let mut keys: Vec<&str> = Vec::new();
    for reference in &task.spec.addresses {
        let new = |key: &str| {
            (run.delivery.stage(n)).is_some_and(|s| {
                // The fix round's m1: a thread that counts (a writer's), not one
                // still waiting for its author's permission.
                (s.threads.iter()).any(|t| t.key == key && t.state == ThreadState::New && t.counted)
            })
        };
        match parse(reference) {
            Some((p, key)) if p == pr && new(key) && !keys.contains(&key) => keys.push(key),
            _ => errors.extend(unknown_refs(&id, n, std::slice::from_ref(reference))),
        }
    }
    if !errors.is_empty() {
        return errors;
    }
    let keys: Vec<String> = keys.into_iter().map(str::to_string).collect();
    let stage = &mut run.delivery.stages[usize::from(n) - 1];
    let mut batch = 0;
    for t in (stage.threads.iter_mut()).filter(|t| keys.contains(&t.key)) {
        t.state = ThreadState::Tasked { task: id.clone() };
        batch = batch.max(t.batch);
    }
    // Decision 31: a review round is a closed batch that produced a fix task.
    if batch > stage.round_batch {
        stage.round_batch = batch;
        stage.review_rounds += 1;
    }
    task.origin = TaskOrigin::Review;
    task.fixes = Some(FixOf::Review {
        stage: n,
        pr,
        threads: keys.iter().map(|k| format!("{pr}:{k}")).collect(),
    });
    Vec::new()
}

/// `task <id>: addresses <ref>, which is not a new thread of stage <n>'s PR`, per ref.
fn unknown_refs(id: &str, n: u16, refs: &[String]) -> Vec<PlanError> {
    (refs.iter())
        .map(|r| {
            let text =
                format!("task {id}: addresses {r}, which is not a new thread of stage {n}'s PR");
            error(id, text)
        })
        .collect()
}
