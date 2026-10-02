//! Decisions 29–31 (task M9.2.10): review comments become fix tasks and replies. Pure
//! (design decision 1); host text is data, and only ids, logins and paths are read
//! from it (decision 22).
//!
//! 1. **Intake** ([`intake`], from a view): each fresh comment of a thread is sorted.
//!    anthrex's own (known by the id its reply was answered with, never by its text,
//!    which anyone can paste; or the marker of a reply it sent and has no answer for),
//!    a bot's, and one in a resolved thread do not count, and are logged as ignored. A
//!    human's leaves the thread `new`, its authors the candidates.
//! 2. **Whose** ([`pass`]): a candidate counts when listed in `[delivery] reviewers` or
//!    when GitHub says it may write (`Permission`, one op per run at a time, cached
//!    per login). A login the allow-list would refuse is never asked: it does not
//!    count. A thread with no writer is ignored; one with a writer joins the stage's
//!    batch.
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
//! The template's fix task and the approval hold are `review_fix.rs`.

use proto::{DeliveryMode, HoldState, PrState};

use super::super::requests::log;
use super::super::{Effect, OpKind, wake};
use super::watch::named;
use super::{emit, stage_mut};
use crate::host::RepoPermission;
use crate::host::remote::owner_ok;
use crate::run::delivery::ops::HostOp;
use crate::run::delivery::quote;
use crate::run::delivery::snapshot::stage_count;
use crate::run::delivery::{Batch, PrRecord, ReplyDue, StageDelivery, ThreadRecord, ThreadState};
use crate::run::model::{FixOf, Run};

/// The ruling carried from task M9.2.8: what `run.json` keeps of review text. A thread
/// keeps the text of its newest this many comments, each cut to
/// [`COMMENT_KEPT_CHARS`]; every comment's id stays (newness is by id).
pub(crate) const THREAD_COMMENTS_KEPT: usize = 20;
pub(crate) const COMMENT_KEPT_CHARS: usize = 4_000;
/// A stage keeps at most this many characters of review text in all, newest threads
/// first; a thread that is no longer `new` keeps none (nothing quotes it again).
pub(crate) const STAGE_TEXT_CHARS: usize = 200_000;

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
        t.state = ThreadState::New;
        t.candidates = humans;
        t.counted = false;
        t.batch = 0;
    }
    for line in lines {
        log(run, now, line);
    }
}

/// Whether `login` may write: listed in `[delivery] reviewers`, or GitHub's answer,
/// cached; `None` while unknown. A login the allow-list would refuse to ask about is
/// not a writer (the task M9.2.4 review: never a halt).
pub(super) fn writes(run: &Run, login: &str) -> Option<bool> {
    let d = &run.delivery;
    if d.limits
        .reviewers
        .iter()
        .any(|r| r.eq_ignore_ascii_case(login))
    {
        return Some(true);
    }
    if !owner_ok(login) {
        return Some(false);
    }
    let known = d.permissions.get(&login.to_ascii_lowercase());
    known.map(|p| p.writes())
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

fn thread_mut<'a>(run: &'a mut Run, n: u16, key: &str) -> Option<&'a mut ThreadRecord> {
    stage_mut(run, n).threads.iter_mut().find(|t| t.key == key)
}

/// Step 2 for every thread whose candidates are all known, and at most one
/// `Permission` op for the first unknown login.
fn whose(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let mut ask: Option<String> = None;
    for n in 1..=stage_count(run) {
        let Some(stage) = run.delivery.stage(n) else {
            continue;
        };
        let waiting: Vec<(String, Vec<String>)> = (stage.threads.iter())
            .filter(|t| t.state == ThreadState::New && !t.counted && !t.candidates.is_empty())
            .map(|t| (t.key.clone(), t.candidates.clone()))
            .collect();
        for (key, candidates) in waiting {
            let verdicts: Vec<Option<bool>> = candidates.iter().map(|c| writes(run, c)).collect();
            if let Some(k) = verdicts.iter().position(|v| *v == Some(true)) {
                counts(run, n, &key, &candidates[k], now);
            } else if let Some(k) = verdicts.iter().position(Option::is_none) {
                ask.get_or_insert_with(|| candidates[k].clone());
            } else {
                not_a_writer(run, n, &key, &candidates, now);
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
fn counts(run: &mut Run, n: u16, key: &str, login: &str, now: u64) {
    if let Some(t) = thread_mut(run, n, key) {
        t.counted = true;
        t.author = login.to_string();
        t.candidates.clear();
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

/// No candidate may write: the thread is ignored, and each author logged.
fn not_a_writer(run: &mut Run, n: u16, key: &str, candidates: &[String], now: u64) {
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    if let Some(t) = thread_mut(run, n, key) {
        t.state = ThreadState::Ignored {
            reason: "no write access".into(),
        };
        t.candidates.clear();
        t.comments.iter_mut().for_each(|c| c.text.clear());
        t.text.clear();
    }
    for login in candidates {
        let who = quote::login(login);
        let line = format!(
            "{}: ignored a comment by @{who}: no write access",
            named(n, &pr)
        );
        log(run, now, line);
    }
}

/// Whether the run's orchestrator window is live (M9 decision 13): a planned run's
/// review batch is its to decide; otherwise the engine's template decides.
fn orchestrator_live(run: &Run) -> bool {
    run.orch.orchestrator.as_ref().is_some_and(|o| o.live)
}

fn plural(k: usize, one: &str, many: &str) -> String {
    if k == 1 {
        format!("{k} {one}")
    } else {
        format!("{k} {many}")
    }
}

/// Step 3: stage `n`'s batch closes when quiet for `review_batch_secs`, and no thread
/// of the stage still waits for its author's permission.
fn close(run: &mut Run, n: u16, now: u64, fx: &mut Vec<Effect>) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let Some(batch) = stage.batch.clone() else {
        return;
    };
    let asking = (stage.threads.iter())
        .any(|t| t.state == ThreadState::New && !t.counted && !t.candidates.is_empty());
    let quiet_at = batch
        .last_at
        .saturating_add(run.delivery.limits.review_batch_secs);
    if asking || now < quiet_at {
        return;
    }
    let stage = stage_mut(run, n);
    stage.batch = None;
    stage.batches += 1;
    let b = stage.batches;
    let mut keys = Vec::new();
    for t in stage.threads.iter_mut() {
        let live = t.state == ThreadState::New && t.counted;
        if live && batch.threads.contains(&t.key) {
            t.batch = b;
            keys.push(t.key.clone());
        }
    }
    let Some(pr) = run.delivery.pr(n).cloned() else {
        return;
    };
    let paused = run.delivery.stage(n).is_some_and(|s| s.paused_by.is_some());
    if keys.is_empty() || pr.state != PrState::Open || paused {
        return;
    }
    let rounds = run.delivery.stage(n).map_or(0, |s| s.review_rounds);
    let max = run.delivery.limits.review_fix_max;
    if rounds >= max {
        return over_cap(run, n, &pr, &keys, rounds + 1, now);
    }
    let text = format!("{}: review batch {b}: {}", named(n, &pr), keys.join(", "));
    log(run, now, text);
    if orchestrator_live(run) {
        let mut logins: Vec<String> = Vec::new();
        for key in &keys {
            let login = author(run, n, key);
            if !logins.contains(&login) {
                logins.push(login);
            }
        }
        let from: Vec<String> = logins.iter().map(|l| format!("@{l}")).collect();
        let threads = plural(keys.len(), "new review thread", "new review threads");
        wake::note(
            run,
            format!(
                "PR #{} (stage {n}) has {threads} from {}; read them in run_status and add fix tasks, reply, or escalate",
                pr.number,
                from.join(", ")
            ),
        );
        return;
    }
    fast_path(run, n, &pr, &keys, b, now, fx);
}

/// The author of thread `key` as a label shows it (decision 22).
pub(super) fn author(run: &Run, n: u16, key: &str) -> String {
    let stage = run.delivery.stage(n);
    let t = stage.and_then(|s| s.threads.iter().find(|t| t.key == key));
    quote::login(t.map_or("", |t| t.author.as_str())).to_string()
}

/// Decision 31's cap: `review_fix_max` rounds were had; each thread is the user's.
fn over_cap(run: &mut Run, n: u16, pr: &PrRecord, keys: &[String], round: u32, now: u64) {
    for key in keys {
        let login = author(run, n, key);
        let line = format!(
            "PR #{}: review round {round} is over the cap; thread {}:{key} by @{login} is yours",
            pr.number, pr.number
        );
        log(run, now, line.clone());
        run.delivery.alerts.insert(format!("{n}/cap/{key}"), line);
    }
}

/// Decision 31's fast path (and the fallback for a planned run whose orchestrator is
/// not live): one fix task per thread from the template; a thread whose fix task
/// already exists never gets a second one. A batch that made one is a review round.
fn fast_path(
    run: &mut Run,
    n: u16,
    pr: &PrRecord,
    keys: &[String],
    b: u32,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let mut made = false;
    for key in keys {
        let reference = format!("{}:{key}", pr.number);
        let tasked = (run.tasks.iter())
            .filter(|t| t.state != proto::TaskState::Cancelled)
            .any(|t| matches!(&t.fixes, Some(FixOf::Review { threads, .. }) if threads.contains(&reference)));
        let Some(thread) = (!tasked)
            .then(|| {
                run.delivery
                    .stage(n)?
                    .threads
                    .iter()
                    .find(|t| &t.key == key)
            })
            .flatten()
            .cloned()
        else {
            continue;
        };
        match super::review_fix::add_review_fix(run, n, pr, &thread, now, fx) {
            Ok(id) => {
                made = true;
                if let Some(t) = thread_mut(run, n, key) {
                    t.state = ThreadState::Tasked { task: id.clone() };
                }
                super::review_fix::hold_outside(run, n, &id, now);
            }
            Err(message) => {
                let login = author(run, n, key);
                let line = format!(
                    "PR #{}: thread {reference} by @{login} is yours; its fix task was refused: {message}",
                    pr.number
                );
                log(run, now, line.clone());
                run.delivery
                    .alerts
                    .insert(format!("{n}/review/{key}"), line);
            }
        }
    }
    let stage = stage_mut(run, n);
    if made && b > stage.round_batch {
        stage.round_batch = b;
        stage.review_rounds += 1;
    }
}

/// The cap's and a refused fix's attention lines last while their thread is `new`.
fn settle(run: &mut Run, n: u16) {
    let Some(stage) = run.delivery.stage(n) else {
        return;
    };
    let new =
        |key: &str| (stage.threads.iter()).any(|t| t.key == key && t.state == ThreadState::New);
    let stale: Vec<String> = (run.delivery.alerts.keys())
        .filter(|k| {
            let rest = k.strip_prefix(&format!("{n}/"));
            let key = rest.and_then(|r| r.strip_prefix("cap/").or(r.strip_prefix("review/")));
            key.is_some_and(|key| !new(key))
        })
        .cloned()
        .collect();
    for key in stale {
        run.delivery.alerts.remove(&key);
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
        close(run, n, now, fx);
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
