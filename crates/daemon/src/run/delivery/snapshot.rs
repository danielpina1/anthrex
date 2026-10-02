//! Decision 41: what the snapshot shows of a run's delivery (`RunInfo.delivery`,
//! `StageInfo.pr`), and 9.2's `TaskInfo.fixes` texts. Pure. Counts, logins, ids and
//! states only: no comment, review or log text ever reaches the snapshot
//! (decision 22).

use proto::{
    CheckRunInfo, CiState, DeliveryInfo, DeliveryMode, StagePrInfo, TaskOrigin, ThreadCounts,
};

use super::{CHECKS_MAX, CheckSeen, StageDelivery, ThreadState, quote};
use crate::run::contract::sha7;
use crate::run::model::{FixOf, Run, Task};
/// The one count of a run's stages (fix round 1, m4), re-exported for the delivery
/// engine.
pub(crate) use crate::run::snapshot_stages::stage_count;

/// `RunInfo.delivery`: `None` in local mode.
pub fn delivery_info(run: &Run) -> Option<DeliveryInfo> {
    let d = &run.delivery;
    if d.mode == DeliveryMode::Local {
        return None;
    }
    let repo = d.repo.as_ref();
    Some(DeliveryInfo {
        mode: d.mode,
        remote: repo.map(|r| r.remote.clone()).unwrap_or_default(),
        repo: repo.map(|r| r.full()).unwrap_or_default(),
        watching: d.watching,
        delivering: d.delivering(stage_count(run)),
        poll_secs: d.poll_base_secs,
        skipped_stages: (1..)
            .zip(&d.stages)
            .filter(|(_, s)| s.skipped)
            .map(|(n, _)| n)
            .collect(),
        // Ruling R-13: from the same lines as the run's attention (one helper).
        alerts: crate::run::engine::delivery::alerts(run),
    })
}

/// `StageInfo.pr`: stage `n`'s pull request, when it has one.
pub fn stage_pr_info(run: &Run, n: u16) -> Option<StagePrInfo> {
    let stage = run.delivery.stage(n)?;
    let pr = stage.pr.as_ref()?;
    let checks: Vec<CheckRunInfo> = pr
        .checks
        .iter()
        .take(CHECKS_MAX)
        .map(|c| CheckRunInfo {
            name: c.name.clone(),
            state: c.state,
            fix_task: fixing(stage, &pr.pushed_head, c),
        })
        .collect();
    Some(StagePrInfo {
        number: pr.number,
        url: pr.url.clone(),
        state: pr.state,
        base: pr.base.clone(),
        opened_at: pr.opened_at,
        head: pr.pushed_head.clone(),
        ci: ci_state(&pr.checks),
        checks,
        threads: thread_counts(stage),
        fix_tasks: fix_tasks(run, n),
        paused: stage.paused_by.is_some(),
        human_review_secs: stage.review_wait_secs,
        merged_at: pr.merged_at,
        merge_commit: pr.merge_commit.clone(),
    })
}

/// What a head's checks add up to: red if one is red, else pending if one is,
/// else green once there is one, else none.
pub(crate) fn ci_state(checks: &[CheckSeen]) -> CiState {
    let any = |state: CiState| checks.iter().any(|c| c.state == state);
    if any(CiState::Red) {
        CiState::Red
    } else if any(CiState::Pending) {
        CiState::Pending
    } else if any(CiState::Green) {
        CiState::Green
    } else {
        CiState::None
    }
}

/// The fix task working on a red check of `head`: the newest CI record of that head
/// with one whose Actions runs hold the check's run (any such record, for a check
/// with no Actions run).
fn fixing(stage: &StageDelivery, head: &str, check: &CheckSeen) -> Option<String> {
    if check.state != CiState::Red {
        return None;
    }
    stage
        .ci
        .iter()
        .rev()
        .filter(|r| r.head == head)
        .filter(|r| check.ci_run.is_none_or(|id| r.ci_runs.contains(&id)))
        .find_map(|r| r.fix_task.clone())
}

fn thread_counts(stage: &StageDelivery) -> ThreadCounts {
    let mut counts = ThreadCounts::default();
    for t in &stage.threads {
        let slot = match t.state {
            ThreadState::New => &mut counts.new,
            ThreadState::Tasked { .. } => &mut counts.tasked,
            ThreadState::Replied { .. } => &mut counts.replied,
            ThreadState::Ignored { .. } => &mut counts.ignored,
        };
        *slot = slot.saturating_add(1);
    }
    counts
}

/// `"<id> <origin> <state label>"` of stage `n`'s fix tasks, the unfinished first,
/// each group in plan order.
fn fix_tasks(run: &Run, n: u16) -> Vec<String> {
    let fixes: Vec<&Task> = run
        .tasks
        .iter()
        .filter(|t| t.stage() == n && t.origin != TaskOrigin::Plan)
        .collect();
    let (open, done): (Vec<&Task>, Vec<&Task>) =
        fixes.into_iter().partition(|t| !t.state.is_finished());
    open.into_iter()
        .chain(done)
        .map(|t| format!("{} {} {}", t.id(), origin_label(t.origin), t.state.label()))
        .collect()
}

/// 9.1's `TaskOrigin` as the wire spells it.
pub(crate) fn origin_label(origin: TaskOrigin) -> &'static str {
    match origin {
        TaskOrigin::Plan => "plan",
        TaskOrigin::Bisect => "bisect",
        TaskOrigin::Sync => "sync",
        TaskOrigin::Ci => "ci",
        TaskOrigin::Review => "review",
    }
}

/// Decision 41's texts of 9.2's three `FixOf` variants: `CI run <id>` (`CI runs <a>,
/// <b>`), `thread by @<login>` (`<k> threads by @<a>, @<b>`) and `sync with
/// <base>@<sha7>`. `None` for 9.1's own, whose texts `engine::fix_text` builds.
pub fn fix_text(run: &Run, fixes: &FixOf) -> Option<String> {
    match fixes {
        // Fix round 1 (m1): a CI fix with no Actions run (an external check) names its
        // head, a review fix with no thread its PR, so the text is never empty.
        FixOf::Ci { ci_runs, head, .. } if ci_runs.is_empty() => {
            Some(format!("CI on {}", sha7(head)))
        }
        FixOf::Ci { ci_runs, .. } => {
            let ids: Vec<String> = ci_runs.iter().map(u64::to_string).collect();
            let word = if ids.len() == 1 { "run" } else { "runs" };
            Some(format!("CI {word} {}", ids.join(", ")))
        }
        FixOf::Review { pr, threads, .. } if threads.is_empty() => {
            Some(format!("review of PR #{pr}"))
        }
        FixOf::Review { stage, threads, .. } => {
            let mut logins: Vec<&str> = Vec::new();
            for login in threads.iter().map(|r| author(run, *stage, r)) {
                if !logins.contains(&login) {
                    logins.push(login);
                }
            }
            let by = logins
                .iter()
                .map(|l| format!("@{l}"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(match threads.len() {
                1 => format!("thread by {by}"),
                k => format!("{k} threads by {by}"),
            })
        }
        FixOf::Base { base_sha, .. } => {
            Some(format!("sync with {}@{}", run.base_branch, sha7(base_sha)))
        }
        FixOf::Bisect { .. } | FixOf::Propagate { .. } => None,
    }
}

/// The author of thread `<pr>:<key>` of stage `stage`, as decision 22 shows a login:
/// `<unknown>` when it is not a login, or the thread is not recorded.
fn author<'a>(run: &'a Run, stage: u16, thread: &str) -> &'a str {
    let key = thread.split_once(':').map_or(thread, |(_, key)| key);
    run.delivery
        .stage(stage)
        .and_then(|s| s.threads.iter().find(|t| t.key == key))
        .map_or("<unknown>", |t| quote::login(&t.author))
}
