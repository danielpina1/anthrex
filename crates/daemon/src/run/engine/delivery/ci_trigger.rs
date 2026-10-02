//! Decision 27's trigger (task M9.2.9): a view whose pushed head has every check done
//! and one red starts a `CiRecord`, once per set of red jobs (the controller's ruling:
//! a re-run's jobs have new ids, so the red after it is seen); a green view ends what
//! waited on it. Pure (design decision 2).

use proto::{CiState, PrState};

use super::super::requests::log;
use super::stage_mut;
use super::view::{check_name, check_state};
use super::watch::named;
use crate::host::{Conclusion, PrView};
use crate::run::contract::sha7;
use crate::run::delivery::{CI_RECORDS_MAX, CiPhase, CiRecord, CiSeen, Watermark};
use crate::run::model::Run;

/// Decision 27 step 1: at most this many Actions runs' logs per red.
const RUNS_MAX: usize = 5;
/// A check's URL is cut to this many characters in its line.
const URL_CHARS: usize = 300;

/// One red check of a view (decision 27's trigger), as the record keeps it.
#[derive(Debug, Clone)]
pub(super) struct RedCheck {
    name: String,
    conclusion: Conclusion,
    ci_run: Option<u64>,
    url: String,
    /// `<run>/<job>` from its URL; a check without a job, its name.
    job: String,
}

/// The job of an Actions check's URL (`/actions/runs/<run>/job/<job>`).
fn job_of(c: &crate::host::CheckRun) -> String {
    let job = c.url.split("/job/").nth(1).map(|rest| {
        rest.chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
    });
    match (c.ci_run, job) {
        (Some(run), Some(job)) if !job.is_empty() => format!("{run}/{job}"),
        _ => check_name(c),
    }
}

/// Decision 27's trigger (ruling R-5): on the pushed head, every check completed and one
/// red: the red checks, by name. `None` for a stale head, a pending check or no red.
pub(super) fn red_of(view: &PrView, pushed: &str) -> Option<Vec<RedCheck>> {
    let pending = view
        .checks
        .iter()
        .any(|c| check_state(c) == CiState::Pending);
    if view.head_oid != pushed || pending {
        return None;
    }
    let mut red: Vec<RedCheck> = (view.checks.iter())
        .filter(|c| check_state(c) == CiState::Red)
        .map(|c| RedCheck {
            name: check_name(c),
            conclusion: c.conclusion.unwrap_or(Conclusion::Failure),
            ci_run: c.ci_run,
            url: c.url.clone(),
            job: job_of(c),
        })
        .collect();
    red.sort_by(|a, b| (&a.name, &a.job).cmp(&(&b.name, &b.job)));
    (!red.is_empty()).then_some(red)
}

/// Recorded once per set of red jobs of a head (the controller's ruling: a re-run's
/// jobs have new ids, so its red is seen again); the line to log when it is new.
pub(super) fn seen(wm: &mut Watermark, head: &str, red: &[RedCheck], now: u64) -> Option<String> {
    let mut jobs: Vec<String> = red.iter().map(|c| c.job.clone()).collect();
    jobs.sort();
    let mut names: Vec<String> = red.iter().map(|c| c.name.clone()).collect();
    names.dedup();
    let known = wm.ci.get(head).is_some_and(|s| {
        if s.jobs.is_empty() {
            s.failing == names
        } else {
            s.jobs == jobs
        }
    });
    if known {
        return None;
    }
    let line = names.join(", ");
    let entry = CiSeen {
        failing: names,
        at: now,
        jobs,
    };
    wm.ci.insert(head.to_string(), entry);
    while wm.ci.len() > CI_RECORDS_MAX {
        let oldest = (wm.ci.iter()).min_by_key(|(_, s)| s.at);
        let Some(head) = oldest.map(|(h, _)| h.clone()) else {
            break;
        };
        wm.ci.remove(&head);
    }
    Some(line)
}

/// GitHub's spelling of a conclusion, as a check's line shows it.
fn conclusion_text(c: Conclusion) -> &'static str {
    match c {
        Conclusion::Success => "SUCCESS",
        Conclusion::Failure => "FAILURE",
        Conclusion::TimedOut => "TIMED_OUT",
        Conclusion::Cancelled => "CANCELLED",
        Conclusion::ActionRequired => "ACTION_REQUIRED",
        Conclusion::StartupFailure => "STARTUP_FAILURE",
        Conclusion::Neutral => "NEUTRAL",
        Conclusion::Skipped => "SKIPPED",
        Conclusion::Stale => "STALE",
        Conclusion::Error => "ERROR",
    }
}

/// A new red on stage `n`'s pushed head `head`: a new record, or, for a head whose
/// record waits on its re-run, the red after it (step 3). Any other red of a head
/// already handled adds nothing.
pub(super) fn red(run: &mut Run, n: u16, head: &str, red: Vec<RedCheck>, now: u64) {
    let mut runs: Vec<u64> = Vec::new();
    for run_id in red.iter().filter_map(|c| c.ci_run) {
        if !runs.contains(&run_id) && runs.len() < RUNS_MAX {
            runs.push(run_id);
        }
    }
    let external = (red.iter().filter(|c| c.ci_run.is_none()))
        .map(|c| {
            let url: String = proto::safe_text::one_line(&c.url)
                .chars()
                .take(URL_CHARS)
                .collect();
            let conclusion = conclusion_text(c.conclusion);
            format!("{}: {conclusion} ({url})", c.name)
        })
        .collect();
    let mut checks: Vec<String> = red.iter().map(|c| c.name.clone()).collect();
    checks.dedup();
    let infra = |c: &RedCheck| {
        matches!(
            c.conclusion,
            Conclusion::Cancelled | Conclusion::StartupFailure
        )
    };
    let fresh = CiRecord {
        ci_runs: runs,
        checks,
        jobs: red.iter().map(|c| c.job.clone()).collect(),
        external,
        infra_only: red.iter().all(infra),
        ..CiRecord::new(head)
    };
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let stage = stage_mut(run, n);
    if let Some(known) = stage.ci.iter_mut().rev().find(|r| r.head == head) {
        if known.phase != CiPhase::Rerunning {
            return;
        }
        // Step 3: the red after the re-run; the re-runs stay recorded.
        let (reruns, answered) = (known.reruns.clone(), known.reruns_answered.clone());
        *known = CiRecord {
            reruns,
            reruns_answered: answered,
            ..fresh
        };
        let text = format!(
            "{}: CI red again at {} after its re-run",
            named(n, &pr),
            sha7(head)
        );
        return log(run, now, text);
    }
    stage.ci.push(fresh);
    while stage.ci.len() > CI_RECORDS_MAX {
        let done = |r: &CiRecord| matches!(r.phase, CiPhase::Tasked | CiPhase::ToUser);
        let at = stage.ci.iter().position(done).unwrap_or(0);
        stage.ci.remove(at);
    }
}

/// A view of stage `n`'s PR that shows its pushed head all green ends what was waiting
/// on it: a record waiting on its re-run, and the stage's CI attention lines.
pub(super) fn viewed(run: &mut Run, n: u16, view: &PrView, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let green = view.head_oid == pr.pushed_head
        && view.checks.iter().all(|c| check_state(c) == CiState::Green);
    let landed = view.state != PrState::Open;
    if !(green || landed) {
        return;
    }
    let prefix = format!("{n}/ci/");
    run.delivery.alerts.retain(|k, _| !k.starts_with(&prefix));
    let stage = stage_mut(run, n);
    let before = stage.ci.len();
    stage
        .ci
        .retain(|r| !(r.head == view.head_oid && r.phase == CiPhase::Rerunning));
    if stage.ci.len() < before && green {
        let text = format!(
            "{}: CI green at {} after its re-run",
            named(n, &pr),
            sha7(&pr.pushed_head)
        );
        log(run, now, text);
    }
}

/// The final fix wave's I-2 and I-3: CI is judged again on stage `n`'s pushed head (a
/// fix some later red joined finished, the PR was reopened, the stage was unpaused).
/// The red seen there is forgotten, with the finished records on that head, and the PR
/// is due a view now, so a red raises its fix or its alert again. Nothing is forgotten
/// while a record on that head is still being worked on.
pub(super) fn rearm(run: &mut Run, n: u16, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let head = pr.pushed_head.clone();
    let done = |r: &CiRecord| matches!(r.phase, CiPhase::Tasked | CiPhase::ToUser);
    let stage = stage_mut(run, n);
    if stage.ci.iter().any(|r| r.head == head && !done(r)) {
        return;
    }
    stage.ci.retain(|r| r.head != head);
    let seen = super::watch::pr_mut(run, n).and_then(|p| {
        p.next_poll_at = p.next_poll_at.min(now);
        p.watermark.ci.remove(&head)
    });
    if seen.is_some() {
        let text = format!("{}: CI at {} is judged again", named(n, &pr), sha7(&head));
        log(run, now, text);
    }
}
