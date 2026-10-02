//! Decision 23 (task M9.2.8): a view of a stage PR, processed against its watermark in
//! the one step that answers it. New comments, reviews and review-thread comments
//! (their GitHub database ids above the watermark's, ruling R-2) become thread records
//! keyed by decision 4; a red CI run on the pushed head is recorded once per set of
//! failing checks (decision 27's trigger, ruling R-5); the PR's state, base and checks
//! are the host's; a head that is not the pushed one makes an adopt due (decision 24).
//! The new watermark is recorded in the same step, so a restart neither loses nor
//! repeats an event. What the records lead to (fix tasks, replies, landing) is later
//! tasks'. Host text is kept as data: check names are put on one line and cut before
//! they are kept or logged, and the log names threads by id only (decision 22). Pure.

use proto::{CiState, PrState};

use super::super::requests::log;
use super::stage_mut;
use super::watch::{cap_secs, interval, named, pr_mut};
use crate::host::{CheckRun, CheckStatus, PrView, Review, ReviewState};
use crate::run::contract::sha7;
use crate::run::delivery::{
    CHECKS_MAX, CI_RECORDS_MAX, CheckSeen, CiSeen, ThreadRecord, ThreadState,
};
use crate::run::model::Run;

/// A check name is cut to this many characters before it is kept or logged.
const CHECK_NAME_CHARS: usize = 100;

/// A check's state as the snapshot shows it (ruling R-5: unknown is pending).
fn check_state(c: &CheckRun) -> CiState {
    match (c.status, c.conclusion) {
        (CheckStatus::Completed, Some(x)) if x.is_red() => CiState::Red,
        (CheckStatus::Completed, Some(_)) => CiState::Green,
        _ => CiState::Pending,
    }
}

/// A check name from the host, on one line and cut (decision 22: host text).
fn check_name(c: &CheckRun) -> String {
    let name = proto::safe_text::one_line(&c.name);
    name.chars().take(CHECK_NAME_CHARS).collect()
}

fn seen_checks(view: &PrView) -> Vec<CheckSeen> {
    (view.checks.iter().take(CHECKS_MAX))
        .map(|c| CheckSeen {
            name: check_name(c),
            state: check_state(c),
            ci_run: c.ci_run,
        })
        .collect()
}

/// Decision 27's trigger (rulings R-5): on the pushed head, every check completed and
/// one red: the red checks' names, sorted. `None` for a stale head, a pending check or
/// no red one.
fn red_set(view: &PrView, pushed: &str) -> Option<Vec<String>> {
    if view.head_oid != pushed
        || view
            .checks
            .iter()
            .any(|c| check_state(c) == CiState::Pending)
    {
        return None;
    }
    let mut red: Vec<String> = (view.checks.iter())
        .filter(|c| check_state(c) == CiState::Red)
        .map(check_name)
        .collect();
    red.sort();
    red.dedup();
    (!red.is_empty()).then_some(red)
}

/// The view's items above the watermark, as thread records (decision 4's keys): a
/// conversation comment `c<id>`, a changes-requested review's body `r<id>`, a review
/// thread `t<id of its first comment>`, which a new comment in it updates.
fn new_threads(run: &mut Run, n: u16, view: &PrView, now: u64) -> Vec<String> {
    let Some(wm) = run.delivery.pr(n).map(|p| p.watermark.clone()) else {
        return Vec::new();
    };
    let mut added = Vec::new();
    let mut records: Vec<ThreadRecord> = Vec::new();
    let record = |key: String, author: &str, text: &str, id: u64| ThreadRecord {
        key,
        author: author.to_string(),
        path: None,
        line: None,
        diff_hunk: String::new(),
        text: text.to_string(),
        state: ThreadState::New,
        seen_at: now,
        last_comment_id: id,
    };
    for c in view.comments.iter().filter(|c| c.id > wm.issue_comment) {
        records.push(record(format!("c{}", c.id), &c.author.login, &c.body, c.id));
    }
    let changes = |r: &&Review| r.id > wm.review && r.state == ReviewState::ChangesRequested;
    for r in view.reviews.iter().filter(changes) {
        records.push(record(format!("r{}", r.id), &r.author.login, &r.body, r.id));
    }
    let stage = stage_mut(run, n);
    for t in &view.threads {
        let (Some(first), Some(last)) = (t.comments.first(), t.comments.last()) else {
            continue;
        };
        let fresh: Vec<_> = t
            .comments
            .iter()
            .filter(|c| c.id > wm.review_comment)
            .collect();
        let Some(newest) = fresh.last() else {
            continue;
        };
        let key = format!("t{}", first.id);
        if let Some(known) = stage.threads.iter_mut().find(|k| k.key == key) {
            known.last_comment_id = known.last_comment_id.max(last.id);
            known.text = newest.body.clone();
            continue;
        }
        let mut thread = record(key, &first.author.login, &fresh[0].body, last.id);
        thread.path = t.path.clone();
        thread.line = t.line;
        thread.diff_hunk = first.diff_hunk.clone();
        records.push(thread);
    }
    for r in records {
        if !stage.threads.iter().any(|k| k.key == r.key) {
            added.push(r.key.clone());
            stage.threads.push(r);
        }
    }
    added
}

/// Decision 23: a view of stage `n`'s PR, processed against its watermark in this one
/// step, which also records the new watermark (so a restart neither loses nor repeats
/// an event), the PR's state, base and checks, and when it is next due.
pub(super) fn viewed(run: &mut Run, n: u16, view: PrView, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    if view.number != pr.number {
        return;
    }
    let wm = &pr.watermark;
    let checks = seen_checks(&view);
    let above = view.comments.iter().any(|c| c.id > wm.issue_comment)
        || view.reviews.iter().any(|r| r.id > wm.review)
        || (view.threads.iter().flat_map(|t| &t.comments)).any(|c| c.id > wm.review_comment);
    let changed = view.state != pr.state
        || view.head_oid != wm.head
        || Some(view.mergeable) != wm.mergeable
        || checks != pr.checks
        || above;
    let added = new_threads(run, n, &view, now);
    let red = red_set(&view, &pr.pushed_head);
    let k = if changed {
        0
    } else {
        pr.unchanged_views.saturating_add(1)
    };
    let open = view.state == PrState::Open;
    let wait = if open {
        interval(run, k)
    } else {
        cap_secs(run)
    };
    let mut ci_line = None;
    {
        let Some(record) = pr_mut(run, n) else {
            return;
        };
        record.state = view.state;
        // Review m3 (task M9.2.7): the base is the host's (an adopted PR's real base).
        record.base = view.base_ref.clone();
        record.merged_at = view.merged_at.or(record.merged_at);
        if let Some(commit) = &view.merge_commit {
            record.merge_commit = Some(commit.oid.clone());
        }
        record.checks = checks;
        let wm = &mut record.watermark;
        wm.head = view.head_oid.clone();
        wm.mergeable = Some(view.mergeable);
        wm.issue_comment = (view.comments.iter().map(|c| c.id)).fold(wm.issue_comment, u64::max);
        wm.review = (view.reviews.iter().map(|r| r.id)).fold(wm.review, u64::max);
        let thread_ids = view.threads.iter().flat_map(|t| &t.comments).map(|c| c.id);
        wm.review_comment = thread_ids.fold(wm.review_comment, u64::max);
        if let Some(red) = red
            && wm.ci.get(&view.head_oid).is_none_or(|s| s.failing != red)
        {
            ci_line = Some(red.join(", "));
            wm.ci.insert(
                view.head_oid.clone(),
                CiSeen {
                    failing: red,
                    at: now,
                },
            );
            while wm.ci.len() > CI_RECORDS_MAX {
                let oldest = (wm.ci.iter()).min_by_key(|(_, s)| s.at);
                let Some(head) = oldest.map(|(h, _)| h.clone()) else {
                    break;
                };
                wm.ci.remove(&head);
            }
        }
        record.unchanged_views = k;
        record.last_view_at = Some(now);
        record.next_poll_at = now.saturating_add(wait);
    }
    // Decision 24: a head that is not the pushed one is adopted, while the PR is open.
    let foreign = open && !view.head_oid.is_empty() && view.head_oid != pr.pushed_head;
    stage_mut(run, n).remote_head = foreign.then(|| view.head_oid.clone());
    if !added.is_empty() {
        let text = format!("{}: new threads {}", named(n, &pr), added.join(", "));
        log(run, now, text);
    }
    if let Some(checks) = ci_line {
        let text = format!(
            "{}: CI red at {}: {checks}",
            named(n, &pr),
            sha7(&view.head_oid)
        );
        log(run, now, text);
    }
}
