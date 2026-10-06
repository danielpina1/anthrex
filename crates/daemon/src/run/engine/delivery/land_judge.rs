use super::super::requests::log;
use super::super::{Effect, complete, wake};
use super::stage_mut;
use super::watch::named;
use crate::host::Contains;
use crate::run::contract::sha7;
use crate::run::delivery::PrRecord;
use crate::run::delivery::snapshot::stage_count;
use crate::run::model::Run;
use proto::PrState;

/// The head a merged PR was merged at: the head its last view reported, else the head
/// anthrex pushed.
pub(super) fn merged_head(pr: &PrRecord) -> String {
    match pr.watermark.head.as_str() {
        "" => pr.pushed_head.clone(),
        reported => reported.to_string(),
    }
}

/// Stage `n`'s PR merged (its landing processed) and no stage above it still delivers:
/// nothing it gains can be delivered any more (the final fix wave, FW-1 and FW-2).
pub(in crate::run::engine) fn spent(run: &Run, n: u16) -> bool {
    run.delivery.stage(n).and_then(|s| s.landed) == Some(PrState::Merged)
        && !(n + 1..=stage_count(run)).any(|m| super::sync::live(run, m))
}

/// Milestone 9.7 decision 2 (DH §1.1): stage `n` merged; each merged stage `m ≤ n` with
/// no live stage above it (FW-2: a lower stage's kept fix tasks too, once the stage
/// above merges) cancels its unfinished fix tasks, which nothing could deliver any
/// more. A stage below a live one keeps them: propagate carries their commits up. A
/// task whose merge is in flight is cancelled only if that merge does not land
/// ([`merged_late`]).
pub(super) fn cancel_fixes(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let spent: Vec<u16> = (1..=n).filter(|&m| spent(run, m)).collect();
    for m in spent {
        let why = format!("stage {m} PR merged");
        for i in 0..run.tasks.len() {
            let t = &run.tasks[i];
            if t.stage() == m && t.fixes.is_some() && !t.state.is_finished() {
                complete::cancel_task(run, i, &why, now, fx);
            }
        }
    }
}

/// FW-1 (review A I1): a task's merge landed on stage `n`. When the stage's PR already
/// merged, the merge cannot hold it: it is judged as a late merge ([`merged_late`]),
/// whatever added the task.
pub(in crate::run::engine) fn task_merged(run: &mut Run, n: u16, now: u64) {
    let landed = run.delivery.stage(n).and_then(|s| s.landed);
    if super::pr(run) && landed == Some(PrState::Merged) {
        merged_late(run, n, now);
    }
}

/// Milestone 9.7 decision 3 (DH §1.1): a fix task whose deferred cancel arrived too
/// late merged into stage `n` after the host merged the stage's PR, so the merge
/// cannot hold the new stage head: it goes up with a live stage above, or it is not
/// delivered, with an attention line. Nothing is dropped silently.
///
/// An undecided stage (milestone 9.7 BR-6) is decided as not delivered at once, its
/// held judgment released for the new head. A late review fix's replies are queued
/// first (FW-7), so the judgment drops them with the "missed the merge" line rather
/// than leaving them unready on a merged stage.
pub(in crate::run::engine) fn merged_late(run: &mut Run, n: u16, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let head = run.stage_head(n).unwrap_or_default().to_string();
    let at = merged_head(&pr);
    super::reply::queue_replies(run, n);
    if take_undecided(run, n).is_some() || head != at {
        judge(run, n, &pr, &head, &at, false, now);
    }
}

/// Ruling R1: the `contains` checks that may fail in a row before the stage decides.
const CHECK_TRIES: u8 = 3;

/// Milestone 9.7 decision 4: stage `n` merged at `at`, which is neither its local head
/// `head` nor a confirmed one. The verdict, the replies and the line wait for the base
/// fetch to say whether `at` holds `head` (`sync::contains_due`).
pub(super) fn hold(run: &mut Run, n: u16, head: String, at: String) {
    let stage = stage_mut(run, n);
    stage.undecided = Some((head, at));
    stage.undecided_fails = 0;
}

/// Stage `n`'s undecided pair, cleared with its failed checks.
fn take_undecided(run: &mut Run, n: u16) -> Option<(String, String)> {
    let stage = stage_mut(run, n);
    stage.undecided_fails = 0;
    stage.undecided.take()
}

/// The judgment of stage `n`'s merge at `at` against local head `head` (`land::merged`'s,
/// held while undecided): the replies, the "not delivered" line, the missed replies.
pub(super) fn judge(
    run: &mut Run,
    n: u16,
    pr: &PrRecord,
    head: &str,
    at: &str,
    delivered: bool,
    now: u64,
) {
    let missed = super::reply::landed(run, n, at, delivered);
    if !delivered {
        unpushed(run, n, pr, head, at, now);
    }
    if !missed.is_empty() {
        missed_merge(run, n, pr, at, &missed, now);
    }
}

/// Whether `c` still asks about stage `c.stage`'s undecided pair: an answer about any
/// other pair is stale (FW-9: the one comparison `decided` and `check_failed` share).
fn still_asked(run: &Run, c: &Contains) -> bool {
    (run.delivery
        .stage(c.stage)
        .and_then(|s| s.undecided.as_ref()))
    .is_some_and(|(head, merged)| *head == c.head && *merged == c.merged)
}

/// Milestone 9.7 decision 7: the base fetch's answer for stage `contains.stage`. Only
/// `Some(true)` delivers. An answer about another pair than the stage's undecided one
/// is stale and ignored (the next pass asks again). A stage with no PR record left is
/// only cleared (FW-11): nothing to judge, and nothing waits on it.
pub(super) fn decided(run: &mut Run, contains: &Contains, answer: Option<bool>, now: u64) {
    if !still_asked(run, contains) {
        return;
    }
    let n = contains.stage;
    take_undecided(run, n);
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let (head, at) = (&contains.head, &contains.merged);
    judge(run, n, &pr, head, at, answer == Some(true), now);
}

/// Ruling R1 (BR-4): the base fetch carrying `contains` failed. The stage stays
/// undecided and is asked again after the wait; the third failure in a row decides it
/// as not delivered, saying it could not be checked.
pub(super) fn check_failed(run: &mut Run, contains: &Contains, now: u64) {
    if !still_asked(run, contains) {
        return;
    }
    let n = contains.stage;
    let stage = stage_mut(run, n);
    stage.undecided_fails = stage.undecided_fails.saturating_add(1);
    if stage.undecided_fails < CHECK_TRIES {
        return;
    }
    let text = format!(
        "stage {n}: could not check whether {} is in the merge; treating it as not delivered",
        sha7(&contains.head)
    );
    log(run, now, text);
    decided(run, contains, None, now);
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
