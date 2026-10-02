//! Decisions 29–31 (task M9.2.10): review comments become fix tasks and replies. Pure
//! (design decision 1); host text is data, and only ids, logins and paths are read
//! from it (decision 22).
//!
//! 1. **Intake** ([`intake`], from a view): each fresh comment of a thread is sorted.
//!    anthrex's own (known by the id its reply was answered with, never by its text,
//!    which anyone can paste; or the marker of a reply it sent and has no answer for),
//!    a bot's, and one in a resolved thread do not count, and are logged as ignored. A
//!    human's makes its author a candidate; the thread keeps its state until then.
//! 2. **Whose** ([`pass`]): a candidate counts when listed in `[delivery] reviewers` or
//!    when GitHub says it may write (`Permission`, one op per run at a time, cached
//!    per login). A login the allow-list would refuse is never asked: it does not
//!    count. A writer makes the thread `new` and joins it to the stage's batch; a
//!    non-writer's comments lose their text and lower nothing (the fix round's I1), and
//!    a created thread no author may write in is ignored. A thread a view shows
//!    resolved leaves its batch ([`resolved`]).
//! 3. **Batches**: a batch closes `review_batch_secs` after its last thread, and never
//!    while a thread waits for its author's permission. Past `review_fix_max` rounds
//!    its threads go to the user; a live orchestrator is woken once; otherwise each
//!    thread gets a fix task from the template (the fast path), held for approval when
//!    it owns files outside the stage (decision 26).
//! 4. **Replies** (decision 30, `reply.rs`): once a push carrying a review fix's merge
//!    has landed, each of its threads gets `Addressed in <sha7> by task <id>.` with its
//!    marker; a `reply_comment` is sent at once. Replies go before the stage's views,
//!    so anthrex knows the id of every comment it posted before a view can show it.
//!
//! Closing a batch (step 3) is `review_batch.rs`; the template's fix task and the
//! approval hold are `review_fix.rs`.

use proto::{DeliveryMode, HoldState, PrState};

use super::super::requests::log;
use super::super::{Effect, OpKind};
use super::watch::named;
use super::{emit, stage_mut};
use crate::host::RepoPermission;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::quote;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{Batch, ReplyDue, StageDelivery, ThreadRecord, ThreadState};
use crate::run::model::Run;

pub(crate) use crate::run::delivery::view_trim::{
    COMMENT_KEPT_CHARS, STAGE_TEXT_CHARS, THREAD_COMMENTS_KEPT,
};

/// The fix round's m4: a login GitHub has not answered about this long after its
/// comment was seen does not hold its stage's batch (it still counts once answered).
pub(crate) const PERMISSION_WAIT_SECS: u64 = 600;

/// One fresh comment of a view, as the intake reads it: its id and author, and its
/// body (only searched for the marker of a reply anthrex sent).
pub(super) struct Item {
    pub id: u64,
    pub login: String,
    pub bot: bool,
    pub body: String,
}

/// The fresh comments of one thread record of a view (decision 4's key).
pub(super) struct Intake {
    pub key: String,
    /// The record is new with this view.
    pub created: bool,
    pub resolved: bool,
    pub items: Vec<Item>,
}

const OWN: &str = "anthrex's own";

/// Step 1: decision 29's filters, applied to each fresh comment of a view.
pub(super) fn intake(run: &mut Run, n: u16, intakes: Vec<Intake>, now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let mut lines = Vec::new();
    for intake in intakes {
        let stage = stage_mut(run, n);
        let mut humans: Vec<String> = Vec::new();
        let mut reason = None;
        let mut quiet = Vec::new();
        for item in &intake.items {
            let sent = |r: &ReplyDue| r.sent && item.body.contains(&r.marker);
            let own = stage.own_comments.contains(&item.id) || stage.replies.iter().any(sent);
            let why = if own {
                stage.own_comments.insert(item.id);
                Some(OWN)
            } else if item.bot || item.login.ends_with("[bot]") {
                Some("a bot")
            } else if intake.resolved {
                Some("a resolved thread")
            } else {
                None
            };
            let Some(why) = why else {
                if !humans.contains(&item.login) {
                    humans.push(item.login.clone());
                }
                continue;
            };
            quiet.push(item.id);
            reason.get_or_insert(why);
            let who = quote::login(&item.login);
            lines.push(format!(
                "{}: ignored a comment by @{who}: {why}",
                named(n, &pr)
            ));
        }
        let Some(t) = stage.threads.iter_mut().find(|t| t.key == intake.key) else {
            continue;
        };
        // What does not count never reaches an agent: its text is not kept.
        for c in t.comments.iter_mut().filter(|c| quiet.contains(&c.id)) {
            c.text.clear();
        }
        if !t.comments.is_empty() {
            let newest = t.comments.iter().rev().find(|c| !c.text.is_empty());
            t.text = newest.map(|c| c.text.clone()).unwrap_or_default();
        }
        if humans.is_empty() {
            if intake.created {
                let reason = reason.unwrap_or(OWN).to_string();
                t.state = ThreadState::Ignored { reason };
            }
            continue;
        }
        // The fix round's I1: the thread keeps what it was until a fresh author is known
        // to write (`whose`); a created one is `new` and does not count yet.
        if t.candidates.is_empty() {
            t.waiting_since = now;
        }
        for login in humans {
            if !t.candidates.contains(&login) {
                t.candidates.push(login);
            }
        }
    }
    for line in lines {
        log(run, now, line);
    }
}

/// Whether `login` may write: listed in `[delivery] reviewers`, or GitHub's answer,
/// cached; `None` while unknown. A login the allow-list would refuse to ask about is
/// not a writer (the task M9.2.4 review: never a halt).
pub(super) fn writes(run: &Run, login: &str) -> Option<bool> {
    run.delivery.writes(login)
}

/// A `Permission` answer: cached for the run, by login.
pub(super) fn permission(run: &mut Run, user: &str, permission: RepoPermission) {
    let d = &mut run.delivery;
    d.permissions.insert(user.to_ascii_lowercase(), permission);
    d.permission_retry_at = None;
}

/// A failed `Permission` op is asked again `wait` seconds later.
pub(super) fn permission_failed(run: &mut Run, now: u64, wait: u64) {
    run.delivery.permission_retry_at = Some(now.saturating_add(wait));
}

pub(super) fn thread_mut<'a>(run: &'a mut Run, n: u16, key: &str) -> Option<&'a mut ThreadRecord> {
    stage_mut(run, n).threads.iter_mut().find(|t| t.key == key)
}

/// Step 2 for each fresh author whose write access is known, and at most one
/// `Permission` op for the first unknown login. A writer makes the thread count; a
/// non-writer's comments lose their text, and lower nothing (the fix round's I1).
fn whose(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let mut ask: Option<String> = None;
    for n in 1..=stage_count(run) {
        let Some(stage) = run.delivery.stage(n) else {
            continue;
        };
        let waiting: Vec<(String, Vec<String>)> = (stage.threads.iter())
            .filter(|t| !t.candidates.is_empty())
            .map(|t| (t.key.clone(), t.candidates.clone()))
            .collect();
        for (key, candidates) in waiting {
            let verdict = |want: Option<bool>| -> Vec<String> {
                (candidates.iter())
                    .filter(|c| writes(run, c) == want)
                    .cloned()
                    .collect()
            };
            let (writers, others, unknown) =
                (verdict(Some(true)), verdict(Some(false)), verdict(None));
            if !others.is_empty() {
                not_a_writer(run, n, &key, &others, now);
            }
            if let Some(login) = writers.first() {
                counts(run, n, &key, login, now);
            }
            if let Some(login) = unknown.first() {
                ask.get_or_insert_with(|| login.clone());
            }
            if let Some(t) = thread_mut(run, n, &key) {
                t.candidates = unknown;
                // A created thread no fresh author may write in is ignored.
                if t.candidates.is_empty() && t.state == ThreadState::New && !t.counted {
                    t.state = ThreadState::Ignored {
                        reason: "no write access".into(),
                    };
                    t.comments.iter_mut().for_each(|c| c.text.clear());
                    t.text.clear();
                }
            }
        }
    }
    let asking = run.pending_ops.values().any(|p| {
        matches!(
            &p.kind,
            OpKind::Host {
                op: HostOp::Permission { .. },
                ..
            }
        )
    });
    let due = run.delivery.permission_retry_at.is_none_or(|t| t <= now);
    if let Some(user) = ask.filter(|_| !asking && due) {
        emit(run, HostOp::Permission { user }, fx);
    }
}

/// The thread counts: `login` may write. It joins its stage's open batch (decision 31).
/// A thread that had stopped being `new` (tasked, replied, ignored) is `new` again, and
/// may be replied to again (the fix round's m3).
fn counts(run: &mut Run, n: u16, key: &str, login: &str, now: u64) {
    if let Some(t) = thread_mut(run, n, key) {
        t.state = ThreadState::New;
        t.counted = true;
        t.author = login.to_string();
        t.batch = 0;
        t.replies = 0;
    }
    let stage = stage_mut(run, n);
    let batch = stage.batch.get_or_insert_with(|| Batch {
        started_at: now,
        last_at: now,
        threads: Vec::new(),
    });
    if !batch.threads.iter().any(|k| k == key) {
        batch.threads.push(key.to_string());
    }
    batch.last_at = now;
}

/// `logins` may not write: their comments in the thread keep no text, and each is
/// logged. The thread keeps its state and the text of everyone else.
fn not_a_writer(run: &mut Run, n: u16, key: &str, logins: &[String], now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    if let Some(t) = thread_mut(run, n, key) {
        let theirs = |author: &str| logins.iter().any(|l| l.eq_ignore_ascii_case(author));
        for c in t.comments.iter_mut().filter(|c| theirs(&c.author)) {
            c.text.clear();
        }
        if !t.comments.is_empty() {
            let newest = t.comments.iter().rev().find(|c| !c.text.is_empty());
            t.text = newest.map(|c| c.text.clone()).unwrap_or_default();
        } else if theirs(&t.author) {
            t.text.clear();
        }
    }
    for login in logins {
        let who = quote::login(login);
        let line = format!(
            "{}: ignored a comment by @{who}: no write access",
            named(n, &pr)
        );
        log(run, now, line);
    }
}

pub(super) fn plural(k: usize, one: &str, many: &str) -> String {
    if k == 1 {
        format!("{k} {one}")
    } else {
        format!("{k} {many}")
    }
}

/// The author of thread `key` as a label shows it (decision 22).
pub(super) fn author(run: &Run, n: u16, key: &str) -> String {
    let stage = run.delivery.stage(n);
    let t = stage.and_then(|s| s.threads.iter().find(|t| t.key == key));
    quote::login(t.map_or("", |t| t.author.as_str())).to_string()
}

/// The cap's and a refused fix's attention lines last while their thread is `new` and
/// their stage has not landed (the final fix wave's B m-2: a merged PR's threads are
/// no one's to address); a dropped reply's while the PR is open (the fix round's m3).
fn settle(run: &mut Run, n: u16) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let open = run.delivery.pr(n).is_some_and(|p| p.state == PrState::Open);
    let landed = run
        .delivery
        .pr(n)
        .is_some_and(|p| p.state == PrState::Merged);
    let new =
        |key: &str| (stage.threads.iter()).any(|t| t.key == key && t.state == ThreadState::New);
    let stale: Vec<String> = (run.delivery.alerts.keys())
        .filter(|k| {
            let rest = k.strip_prefix(&format!("{n}/"));
            let dropped = rest.is_some_and(|r| r.starts_with("reply/"));
            let key = rest.and_then(|r| r.strip_prefix("cap/").or(r.strip_prefix("review/")));
            key.is_some_and(|key| landed || !new(key)) || (dropped && !open)
        })
        .cloned()
        .collect();
    for key in stale {
        run.delivery.alerts.remove(&key);
    }
}

/// The fix round's m2: a `new` thread a view shows resolved is no longer the run's to
/// address. It leaves its open batch (an emptied batch goes), and no longer counts as
/// not addressed. `keys` are the view's resolved threads.
pub(super) fn resolved(run: &mut Run, n: u16, keys: &[String], now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let stage = stage_mut(run, n);
    let mut gone = Vec::new();
    for t in (stage.threads.iter_mut()).filter(|t| t.state == ThreadState::New) {
        if keys.contains(&t.key) {
            t.state = ThreadState::Ignored {
                reason: "a resolved thread".into(),
            };
            t.candidates.clear();
            gone.push(t.key.clone());
        }
    }
    if let Some(batch) = stage.batch.as_mut() {
        batch.threads.retain(|k| !gone.contains(k));
        if batch.threads.is_empty() {
            stage.batch = None;
        }
    }
    for key in gone {
        log(
            run,
            now,
            format!("{}: thread {key} was resolved", named(n, &pr)),
        );
    }
}

/// The review part of the delivery pass: who counts, the batches, and the replies.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.delivery.mode != DeliveryMode::Pr {
        return;
    }
    whose(run, now, fx);
    for n in 1..=stage_count(run) {
        if run.delivery.pr(n).is_none() {
            continue;
        }
        settle(run, n);
        super::review_batch::untask(run, n, now);
        super::review_batch::close(run, n, now, fx);
        super::reply::queue_replies(run, n);
        super::reply::send(run, n, now, fx);
    }
}

/// The review attention lines: each fix hold awaiting the user (decision 26), and each
/// open stage PR's threads of a closed batch that nothing addressed (decision 31).
pub(crate) fn attention(run: &Run) -> Vec<String> {
    let mut lines: Vec<String> = (run.orch.gate_holds.iter())
        .filter(|h| h.state == HoldState::Awaiting)
        .filter_map(|h| super::review_fix::hold_line(run, &h.id))
        .collect();
    for (n, s) in (1u16..).zip(&run.delivery.stages) {
        let Some(pr) = s.pr.as_ref().filter(|p| p.state == PrState::Open) else {
            continue;
        };
        let alerted = |key: &str| {
            let d = &run.delivery.alerts;
            d.contains_key(&format!("{n}/cap/{key}"))
                || d.contains_key(&format!("{n}/review/{key}"))
        };
        let k = (s.threads.iter())
            .filter(|t| t.state == ThreadState::New && t.counted && t.batch > 0)
            .filter(|t| !alerted(&t.key))
            .count();
        if k > 0 {
            let threads = plural(k, "thread", "threads");
            lines.push(format!("PR #{}: {threads} not addressed", pr.number));
        }
    }
    lines
}

/// The ruling carried from task M9.2.8: a stage keeps the text of its `new` threads
/// only, newest first, each thread its newest [`THREAD_COMMENTS_KEPT`] comments cut to
/// [`COMMENT_KEPT_CHARS`], within [`STAGE_TEXT_CHARS`] in all. Ids are never dropped.
pub(super) fn trim(stage: &mut StageDelivery) {
    let mut budget = STAGE_TEXT_CHARS;
    for t in stage.threads.iter_mut().rev() {
        let keep = t.state == ThreadState::New;
        cut(&mut t.text, keep, &mut budget);
        cut(&mut t.diff_hunk, keep, &mut budget);
        let mut kept = 0;
        for c in t.comments.iter_mut().rev() {
            let room = keep && kept < THREAD_COMMENTS_KEPT;
            if room && !c.text.is_empty() {
                kept += 1;
            }
            cut(&mut c.text, room, &mut budget);
        }
    }
}

fn cut(text: &mut String, keep: bool, budget: &mut usize) {
    let max = if keep {
        COMMENT_KEPT_CHARS.min(*budget)
    } else {
        0
    };
    if let Some((at, _)) = text.char_indices().nth(max) {
        text.truncate(at);
    }
    *budget -= text.chars().count().min(*budget);
}
