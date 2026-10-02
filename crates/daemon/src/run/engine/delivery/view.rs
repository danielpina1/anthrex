//! Decision 23 (task M9.2.8): a view of a stage PR, processed against its watermark in
//! the one step that answers it. New conversation comments (their GitHub database ids
//! above the watermark's, ruling R-2), reviews (by id, not yet processed) and
//! review-thread comments (by id, not yet in their thread's record) become thread
//! records keyed by decision 4 (the fix round's I1: a review's ids are given when it is
//! started, not when it is submitted, so they do not grow in the order reviews show);
//! a full page that may hide older items is an attention line (I2); a red CI run on the pushed head is recorded once per set of
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
use crate::host::{
    CheckRun, CheckStatus, IssueComment, PrView, Review, ReviewState, ReviewThread, THREAD_PAGE,
    ThreadComment, VIEW_PAGE,
};
use crate::run::contract::sha7;
use crate::run::delivery::{
    CHECKS_MAX, CheckSeen, PrRecord, REVIEWS_SEEN_MAX, SeenComment, StageDelivery, ThreadRecord,
    ThreadState, Watermark,
};
use crate::run::model::Run;

/// A check name is cut to this many characters before it is kept or logged.
const CHECK_NAME_CHARS: usize = 100;

/// A check's state as the snapshot shows it (ruling R-5: unknown is pending).
pub(super) fn check_state(c: &CheckRun) -> CiState {
    match (c.status, c.conclusion) {
        (CheckStatus::Completed, Some(x)) if x.is_red() => CiState::Red,
        (CheckStatus::Completed, Some(_)) => CiState::Green,
        _ => CiState::Pending,
    }
}

/// A check name from the host, on one line and cut (decision 22: host text).
pub(super) fn check_name(c: &CheckRun) -> String {
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

/// What a view holds that was not processed before (the fix round's I1).
struct Fresh<'a> {
    comments: Vec<&'a IssueComment>,
    /// Submitted reviews not yet processed (a pending one is not yet submitted).
    reviews: Vec<&'a Review>,
    /// Each thread with comments not yet in its record, and those comments.
    threads: Vec<(&'a ReviewThread, Vec<&'a ThreadComment>)>,
}

impl Fresh<'_> {
    fn is_empty(&self) -> bool {
        self.comments.is_empty() && self.reviews.is_empty() && self.threads.is_empty()
    }
}

fn fresh<'a>(stage: &StageDelivery, wm: &Watermark, view: &'a PrView) -> Fresh<'a> {
    let comments = (view.comments.iter()).filter(|c| c.id > wm.issue_comment);
    let submitted = |r: &&Review| r.state != ReviewState::Pending;
    let reviews =
        (view.reviews.iter().filter(submitted)).filter(|r| !wm.reviews_seen.contains(&r.id));
    let mut threads = Vec::new();
    for t in &view.threads {
        let Some(first) = t.comments.first() else {
            continue;
        };
        let key = format!("t{}", first.id);
        let known = stage.threads.iter().find(|k| k.key == key);
        let seen =
            |c: &&ThreadComment| known.is_some_and(|k| k.comments.iter().any(|s| s.id == c.id));
        let new: Vec<&ThreadComment> = t.comments.iter().filter(|c| !seen(c)).collect();
        if !new.is_empty() {
            threads.push((t, new));
        }
    }
    Fresh {
        comments: comments.collect(),
        reviews: reviews.collect(),
        threads,
    }
}

fn seen_comment(c: &ThreadComment) -> SeenComment {
    SeenComment {
        id: c.id,
        author: c.author.login.clone(),
        text: c.body.clone(),
    }
}

/// The fresh items as thread records (decision 4's keys): a conversation comment
/// `c<id>`, a changes-requested review's body `r<id>` (any other review is only marked
/// processed), a review thread `t<id of its first comment>`, which each later comment
/// in it extends (m5: every comment is kept). Returns the keys of the new records.
fn record(run: &mut Run, n: u16, fresh: &Fresh, now: u64) -> Vec<String> {
    let mut records: Vec<ThreadRecord> = Vec::new();
    let new = |key: String, author: &str, text: &str, id: u64| ThreadRecord {
        key,
        author: author.to_string(),
        path: None,
        line: None,
        diff_hunk: String::new(),
        text: text.to_string(),
        state: ThreadState::New,
        seen_at: now,
        last_comment_id: id,
        comments: Vec::new(),
    };
    for c in &fresh.comments {
        records.push(new(format!("c{}", c.id), &c.author.login, &c.body, c.id));
    }
    let changes = fresh
        .reviews
        .iter()
        .filter(|r| r.state == ReviewState::ChangesRequested);
    for r in changes {
        records.push(new(format!("r{}", r.id), &r.author.login, &r.body, r.id));
    }
    let stage = stage_mut(run, n);
    for (t, comments) in &fresh.threads {
        let (Some(first), Some(newest)) = (t.comments.first(), comments.last()) else {
            continue;
        };
        let last = comments.iter().map(|c| c.id).max().unwrap_or(0);
        let key = format!("t{}", first.id);
        if let Some(known) = stage.threads.iter_mut().find(|k| k.key == key) {
            known.last_comment_id = known.last_comment_id.max(last);
            known.text = newest.body.clone();
            known
                .comments
                .extend(comments.iter().map(|c| seen_comment(c)));
            continue;
        }
        let mut thread = new(key, &first.author.login, &comments[0].body, last);
        thread.path = t.path.clone();
        thread.line = t.line;
        thread.diff_hunk = first.diff_hunk.clone();
        thread.comments = comments.iter().map(|c| seen_comment(c)).collect();
        records.push(thread);
    }
    let mut added = Vec::new();
    for r in records {
        if !stage.threads.iter().any(|k| k.key == r.key) {
            added.push(r.key.clone());
            stage.threads.push(r);
        }
    }
    added
}

/// The fix round's I2: a class that came back as a full page whose oldest item is
/// above what was seen before may hide older ones, and a thread read to its page may
/// hide newer comments. Each is an attention line keyed `<n>/page/<what>`, replaced
/// on every view, so it clears once a view is not full.
fn pages(run: &mut Run, n: u16, wm: &Watermark, view: &PrView, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let full = |ids: &[u64], above: u64| {
        ids.len() >= VIEW_PAGE && ids.iter().min().is_some_and(|&oldest| oldest > above)
    };
    let mut lines: Vec<(String, String)> = Vec::new();
    let mut class = |what: &str, ids: Vec<u64>, above: u64| {
        if full(&ids, above) {
            let line = format!(
                "PR #{}: more than {VIEW_PAGE} new {what} since the last view; the older ones were not read",
                pr.number
            );
            lines.push((format!("{n}/page/{what}"), line));
        }
    };
    class(
        "comments",
        view.comments.iter().map(|c| c.id).collect(),
        wm.issue_comment,
    );
    class(
        "reviews",
        view.reviews.iter().map(|r| r.id).collect(),
        wm.review,
    );
    let firsts = (view.threads.iter()).filter_map(|t| t.comments.first().map(|c| c.id));
    class("review threads", firsts.collect(), wm.review_comment);
    for t in view
        .threads
        .iter()
        .filter(|t| t.comments.len() >= THREAD_PAGE)
    {
        let key = format!("t{}", t.comments[0].id);
        let line = format!(
            "PR #{}: thread {key} has {THREAD_PAGE} comments or more; the newer ones were not read",
            pr.number
        );
        lines.push((format!("{n}/page/{key}"), line));
    }
    let prefix = format!("{n}/page/");
    let alerts = &mut run.delivery.alerts;
    let before: Vec<String> = alerts
        .keys()
        .filter(|k| k.starts_with(&prefix))
        .cloned()
        .collect();
    for key in &before {
        alerts.remove(key);
    }
    let mut raised = Vec::new();
    for (key, line) in lines {
        if !before.contains(&key) {
            raised.push(line.clone());
        }
        alerts.insert(key, line);
    }
    for line in raised {
        log(run, now, format!("{}: {line}", named(n, &pr)));
    }
}

/// Decision 23: a view of stage `n`'s PR, processed in this one step, which also
/// records what it processed (so a restart neither loses nor repeats an event), the
/// PR's state, base and checks, and when it is next due.
pub(super) fn viewed(run: &mut Run, n: u16, view: PrView, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    if view.number != pr.number {
        // Fix round m7: logged, and polled again on the backed-off interval.
        let text = format!(
            "{}: the host answered for PR #{}; ignored",
            named(n, &pr),
            view.number
        );
        log(run, now, text);
        return super::watch::view_failed(run, n, false, now);
    }
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let wm = pr.watermark.clone();
    let fresh = fresh(stage, &wm, &view);
    let checks = seen_checks(&view);
    let changed = view.state != pr.state
        || view.head_oid != wm.head
        || Some(view.mergeable) != wm.mergeable
        || checks != pr.checks
        || !fresh.is_empty();
    let added = record(run, n, &fresh, now);
    let reviewed: Vec<u64> = fresh.reviews.iter().map(|r| r.id).collect();
    pages(run, n, &wm, &view, now);
    let red = super::ci_trigger::red_of(&view, &pr.pushed_head);
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
    let Some(record) = pr_mut(run, n) else {
        return;
    };
    apply(record, &view, checks, &reviewed);
    let ci_line = (red.as_ref())
        .and_then(|red| super::ci_trigger::seen(&mut record.watermark, &view.head_oid, red, now));
    record.unchanged_views = k;
    record.last_view_at = Some(now);
    record.next_poll_at = now.saturating_add(wait);
    // Decision 24: a head that is not the pushed one is adopted, while the PR is open.
    let foreign = open && !view.head_oid.is_empty() && view.head_oid != pr.pushed_head;
    stage_mut(run, n).remote_head = foreign.then(|| view.head_oid.clone());
    if !added.is_empty() {
        let text = format!("{}: new threads {}", named(n, &pr), added.join(", "));
        log(run, now, text);
    }
    if let (Some(checks), Some(red)) = (ci_line, red) {
        let text = format!(
            "{}: CI red at {}: {checks}",
            named(n, &pr),
            sha7(&view.head_oid)
        );
        log(run, now, text);
        // Task M9.2.9: decision 27 starts here.
        super::ci_trigger::red(run, n, &view.head_oid, red, now);
    }
    super::ci_trigger::viewed(run, n, &view, now);
}

/// The view's state, base (the host's: task M9.2.7's review m3), landing fields,
/// checks and watermark ids; `reviewed` joins the processed reviews (I1), the newest
/// [`REVIEWS_SEEN_MAX`] kept.
fn apply(record: &mut PrRecord, view: &PrView, checks: Vec<CheckSeen>, reviewed: &[u64]) {
    record.state = view.state;
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
    wm.reviews_seen.extend(reviewed.iter().copied());
    while wm.reviews_seen.len() > REVIEWS_SEEN_MAX {
        wm.reviews_seen.pop_first();
    }
}
