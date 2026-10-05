use super::super::requests::log;
use super::super::{Effect, complete, wake};
use super::watch::named;
use crate::run::contract::sha7;
use crate::run::delivery::PrRecord;
use crate::run::delivery::snapshot::stage_count;
use crate::run::model::Run;

/// The head a merged PR was merged at: the head its last view reported, else the head
/// anthrex pushed.
pub(super) fn merged_head(pr: &PrRecord) -> String {
    match pr.watermark.head.as_str() {
        "" => pr.pushed_head.clone(),
        reported => reported.to_string(),
    }
}

/// Milestone 9.7 decision 2 (DH §1.1): merged stage `n` with no live stage above it
/// cancels its unfinished fix tasks, which nothing could deliver any more. A stage
/// below a live one keeps them: propagate carries their commits up. A task whose merge
/// is in flight is cancelled only if that merge does not land ([`merged_late`]).
pub(super) fn cancel_fixes(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    if (n + 1..=stage_count(run)).any(|m| super::sync::live(run, m)) {
        return;
    }
    let why = format!("stage {n} PR merged");
    for i in 0..run.tasks.len() {
        let t = &run.tasks[i];
        if t.stage() == n && t.fixes.is_some() && !t.state.is_finished() {
            complete::cancel_task(run, i, &why, now, fx);
        }
    }
}

/// Milestone 9.7 decision 3 (DH §1.1): a fix task whose deferred cancel arrived too
/// late merged into stage `n` after the host merged the stage's PR, so the merge
/// cannot hold the new stage head: it goes up with a live stage above, or it is not
/// delivered, with an attention line. Nothing is dropped silently.
pub(in crate::run::engine) fn merged_late(run: &mut Run, n: u16, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let at = merged_head(&pr);
    if head != at {
        unpushed(run, n, &pr, &head, &at, now);
    }
}

/// Stage `n` was merged at `at`, the host's head, which does not hold its local head
/// `head` (a held stage, a push after the merge, a fix merged since): that work did not
/// land. The next stage that still delivers carries it up (9.1's propagate); with none,
/// an attention line (invented).
pub(super) fn unpushed(run: &mut Run, n: u16, pr: &PrRecord, head: &str, at: &str, now: u64) {
    let above = (n + 1..=stage_count(run)).find(|&m| super::sync::live(run, m));
    let line = match above {
        Some(m) => format!(
            "{}: merged at {}, without {}, so its commits go up with stage {m}",
            named(n, pr),
            sha7(at),
            sha7(head)
        ),
        None => {
            let line = format!(
                "stage {n} PR #{} was merged at {}, without {}; that work is not delivered (anthrex run cancel gives up)",
                pr.number,
                sha7(at),
                sha7(head)
            );
            run.delivery
                .alerts
                .insert(format!("{n}/unlanded"), line.clone());
            wake::note(run, line.clone());
            line
        }
    };
    log(run, now, line);
}

/// I-1: the fix tasks whose replies were dropped because their fix missed the merge at
/// `at`, as `(task, thread)`: one attention line for the stage (invented).
pub(super) fn missed_merge(
    run: &mut Run,
    n: u16,
    pr: &PrRecord,
    at: &str,
    missed: &[(String, String)],
    now: u64,
) {
    let mut tasks: Vec<&str> = Vec::new();
    for (task, _) in missed {
        if !tasks.contains(&task.as_str()) {
            tasks.push(task);
        }
    }
    let threads: Vec<&str> = missed.iter().map(|(_, t)| t.as_str()).collect();
    let (tasks_word, threads_word, verb) = match (tasks.len(), threads.len()) {
        (1, 1) => ("fix task", "thread", "gets"),
        (1, _) => ("fix task", "threads", "get"),
        (_, 1) => ("fix tasks", "thread", "gets"),
        _ => ("fix tasks", "threads", "get"),
    };
    let line = format!(
        "PR #{} was merged at {} before {tasks_word} {} reached it: the fix missed the merge, so {threads_word} {} {verb} no reply",
        pr.number,
        sha7(at),
        tasks.join(", "),
        threads.join(", ")
    );
    log(run, now, line.clone());
    run.delivery.alerts.insert(format!("{n}/missed"), line);
}
