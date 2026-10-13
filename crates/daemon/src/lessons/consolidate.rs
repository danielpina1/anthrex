//! Milestone 9.15 decisions 7, 8, 17 and 25: the store's lessons and the history's
//! known failures, consolidated on read. Pure: nothing here is written; a trial's end,
//! a lesson's credit and every known failure are derived again on each read.
//!
//! - **Known failures** (decisions 7 and 8): a signature `fp` seen in task records of
//!   at least `known_min` distinct runs inside one `known_window_days` span forms
//!   `kf-<fp>`. It ends after `known_clean_runs` later clean runs over its files, past
//!   `expiry_days` since it was last seen, when its `fp` is dismissed, or when one of
//!   its headlines names a flaky test as a whole word. An ended one is never returned.
//!   Each life forms on its own: only hits after the previous life ended count toward
//!   the next (task 4's review, I1).
//! - **Trial** (decision 17): an `on_trial` lesson with `episodic_trial_runs` run
//!   records after its trial start reads `kept` when it was credited green at least
//!   once, else `retired` with reason [`TRIAL_ENDED`].
//! - **Credit** (decision 25): a task record listing a lesson in `lessons_shown`
//!   counts green when merged and not reverted, not green when reverted or blocked,
//!   and nothing otherwise.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use config::Lessons;
use proto::{
    HistoryLine, LessonInfo, LessonLayer, LessonOrigin, LessonStatus, SigGate, SigRef, TaskOutcome,
    TaskRecord,
};

use super::fold::{Lesson, RUNS_MAX, Store};
use crate::run::globs::any_intersect;
use crate::run::history_io::effective_reverts;

/// Seconds in a day: the unit of every `_days` key.
pub const DAY_SECS: u64 = 86_400;
/// A known failure's files: the first this many of its tasks' `owns` (decision 7).
pub const KNOWN_FILES_MAX: usize = 8;
/// A known failure's distinct headlines kept (decision 32's `LessonInfo.headlines`).
pub const HEADLINES_MAX: usize = 3;
/// A known failure's title, in characters (decision 7).
pub const KNOWN_TITLE_MAX: usize = 120;
/// The derived reason of an episodic lesson whose trial ended with no green (decision
/// 17); never written to the store.
pub const TRIAL_ENDED: &str = "trial ended";

/// Every lesson of the store and every live known failure, credited, their trials
/// settled, ordered as `anthrex lessons list --all` shows them: `known`, `approved`,
/// `kept`, `on_trial`, `proposed`, `rejected`, `retired`; within a status the newest
/// `last_seen` first, then the id.
pub fn consolidate(store: &Store, history: &[HistoryLine], now: u64, cfg: &Lessons) -> Vec<Lesson> {
    let mut lessons: Vec<Lesson> = store.lessons.values().cloned().collect();
    lessons.extend(known(history, store, now, cfg));
    let credit = credit(history);
    let run_times = run_times(history);
    for lesson in &mut lessons {
        if let Some(&(green, outcomes)) = credit.get(lesson.id.as_str()) {
            lesson.green = green;
            lesson.outcomes = outcomes;
        }
        settle_trial(lesson, &run_times, cfg);
    }
    lessons.sort_by(|a, b| {
        (rank(a.status).cmp(&rank(b.status)))
            .then(b.last_seen.cmp(&a.last_seen))
            .then(a.id.cmp(&b.id))
    });
    lessons
}

/// Decisions 7 and 8: the live known failures, newest `last_seen` first, then by id.
/// Their credit is left at zero; [`consolidate`] fills it.
pub fn known(history: &[HistoryLine], store: &Store, now: u64, cfg: &Lessons) -> Vec<Lesson> {
    let tasks: Vec<&TaskRecord> = (history.iter())
        .filter_map(|line| match line {
            HistoryLine::Task(t) => Some(t),
            _ => None,
        })
        .collect();
    let flaky: Vec<&str> = (history.iter())
        .filter_map(|line| match line {
            HistoryLine::Flaky(f) => Some(f.test.as_str()),
            _ => None,
        })
        .collect();
    let mut hits: BTreeMap<&str, Vec<(&TaskRecord, &SigRef)>> = BTreeMap::new();
    for &task in &tasks {
        for sig in &task.signatures {
            hits.entry(sig.fp.as_str()).or_default().push((task, sig));
        }
    }
    let mut found: Vec<Lesson> = (hits.into_iter())
        .filter(|(fp, _)| !store.dismissed.contains(*fp))
        .filter_map(|(fp, mut hits)| {
            hits.sort_by_key(|(task, _)| task.at);
            known_failure(fp, &hits, &tasks, &flaky, now, cfg)
        })
        .collect();
    found.sort_by(|a, b| b.last_seen.cmp(&a.last_seen).then(a.id.cmp(&b.id)));
    found
}

/// One `fp`'s known failure from its hits (sorted by `at`), or `None` when its current
/// life never formed or has ended.
fn known_failure(
    fp: &str,
    hits: &[(&TaskRecord, &SigRef)],
    tasks: &[&TaskRecord],
    flaky: &[&str],
    now: u64,
    cfg: &Lessons,
) -> Option<Lesson> {
    let hits = live(fp, hits, tasks, now, cfg)?;
    let (first, sig) = *hits.first()?;
    let (newest, newest_sig) = *hits.last()?;
    let last_seen = newest.at;
    let (headlines, files, mut runs) = gather(hits);
    if (headlines.iter()).any(|h| flaky.iter().any(|test| whole_word(h, test))) {
        return None;
    }
    let body = format!(
        "Seen {} times in {} runs (last {}, gate {}): {}",
        hits.len(),
        runs.len(),
        &common::text::format_utc(last_seen)[..10],
        gate_name(newest_sig.gate),
        sig.headline,
    );
    if runs.len() > RUNS_MAX {
        runs.drain(..runs.len() - RUNS_MAX);
    }
    Some(Lesson {
        id: format!("kf-{fp}"),
        layer: LessonLayer::Episodic,
        status: LessonStatus::Known,
        origin: LessonOrigin::Signature,
        title: format!("Known failure: {}", sig.headline)
            .chars()
            .take(KNOWN_TITLE_MAX)
            .collect(),
        body,
        files,
        created_at: first.at,
        last_seen,
        trial_start: None,
        runs,
        reconfirmed: 0,
        retired_reason: None,
        queued: None,
        green: 0,
        outcomes: 0,
        headlines,
    })
}

/// The hits of `fp`'s live known failure, or `None` when it has none.
///
/// Task 4's review, I1 (the controller's ruling): each life forms on its own. A life
/// forms from the hits after the previous life ended (decision 7: `known_min` runs
/// inside one `known_window_days` span), takes every later hit until it ends, and ends
/// at the `known_clean_runs`-th clean run after it formed (decision 8 a) or when a
/// hit comes, or `now` is, more than `expiry_days` after its last hit (8 b). The hits
/// after an end can only form the next life. Dismissal and flaky tests (8 c, d) end
/// every life, and are checked by the callers.
fn live<'h>(
    fp: &str,
    hits: &'h [(&'h TaskRecord, &'h SigRef)],
    tasks: &[&TaskRecord],
    now: u64,
    cfg: &Lessons,
) -> Option<&'h [(&'h TaskRecord, &'h SigRef)]> {
    let expiry = u64::from(cfg.expiry_days) * DAY_SECS;
    let hit_runs: HashSet<&str> = hits.iter().map(|(t, _)| t.run_id.as_str()).collect();
    let mut start = 0;
    loop {
        let formed = start + formed_at(&hits[start..], cfg)?;
        let formed_at = hits[formed].0.at;
        let mut end = formed + 1;
        let next_start = loop {
            let (_, files, _) = gather(&hits[start..end]);
            let superseded = superseded_at(fp, formed_at, &files, tasks, &hit_runs, cfg);
            let last = hits[end - 1].0.at;
            match hits.get(end) {
                Some((next, _))
                    if superseded.is_some_and(|at| next.at > at) || next.at - last > expiry =>
                {
                    break Some(end);
                }
                Some(_) => end += 1,
                None if superseded.is_some() || now.saturating_sub(last) > expiry => {
                    break None;
                }
                None => return Some(&hits[start..end]),
            }
        };
        start = next_start?;
    }
}

/// When the `known_clean_runs`-th distinct clean run after `formed_at` came (decision
/// 8 a): a merged task with a check, owning some of `files`, in a run that never
/// carried `fp`.
fn superseded_at(
    fp: &str,
    formed_at: u64,
    files: &[String],
    tasks: &[&TaskRecord],
    hit_runs: &HashSet<&str>,
    cfg: &Lessons,
) -> Option<u64> {
    let mut clean: Vec<&TaskRecord> = (tasks.iter().copied())
        .filter(|t| t.at > formed_at && !hit_runs.contains(t.run_id.as_str()))
        .filter(|t| {
            matches!(
                t.outcome,
                TaskOutcome::Merged | TaskOutcome::MergedWithoutApproval
            ) && t.gates.checks >= 1
                && any_intersect(&t.owns, files)
                && !t.signatures.iter().any(|s| s.fp == fp)
        })
        .collect();
    clean.sort_by_key(|t| t.at);
    let mut runs: HashSet<&str> = HashSet::new();
    clean.into_iter().find_map(|t| {
        runs.insert(t.run_id.as_str());
        (runs.len() >= cfg.known_clean_runs as usize).then_some(t.at)
    })
}

/// The distinct headlines (at most 3), files (at most 8) and runs of `hits`, first
/// seen first.
fn gather(hits: &[(&TaskRecord, &SigRef)]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut headlines: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();
    let mut runs: Vec<String> = Vec::new();
    for (task, sig) in hits {
        if headlines.len() < HEADLINES_MAX && !headlines.contains(&sig.headline) {
            headlines.push(sig.headline.clone());
        }
        for own in &task.owns {
            if files.len() < KNOWN_FILES_MAX && !files.contains(own) {
                files.push(own.clone());
            }
        }
        if !runs.contains(&task.run_id) {
            runs.push(task.run_id.clone());
        }
    }
    (headlines, files, runs)
}

/// The index of the hit (of `hits`, sorted by `at`) that first completed `known_min`
/// distinct runs inside one `known_window_days` span.
fn formed_at(hits: &[(&TaskRecord, &SigRef)], cfg: &Lessons) -> Option<usize> {
    let window = u64::from(cfg.known_window_days) * DAY_SECS;
    (0..hits.len()).find(|&j| {
        let end = hits[j].0.at;
        let runs: BTreeSet<&str> = hits[..=j]
            .iter()
            .filter(|(t, _)| end - t.at <= window)
            .map(|(t, _)| t.run_id.as_str())
            .collect();
        runs.len() >= cfg.known_min as usize
    })
}

/// Whether `word` occurs in `text` with no letter, digit or `_` on either side.
fn whole_word(text: &str, word: &str) -> bool {
    if word.is_empty() {
        return false;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

fn gate_name(gate: SigGate) -> &'static str {
    match gate {
        SigGate::Check => "check",
        SigGate::Tier1 => "tier1",
        SigGate::Tier2 => "tier2",
        SigGate::Proof => "proof",
    }
}

/// Decision 25: `(green, outcomes)` per lesson id shown to a finished task. A merge is
/// reverted when a revert in effect names its commit, its task, or its run's accept
/// merge (as `run stats` joins them).
fn credit(history: &[HistoryLine]) -> BTreeMap<&str, (u32, u32)> {
    let reverts = effective_reverts(history);
    let commits: HashSet<&str> = reverts.iter().map(|r| r.reverted.as_str()).collect();
    let reverted = |t: &TaskRecord| {
        t.merge_commit
            .as_deref()
            .is_some_and(|c| commits.contains(c))
            || (reverts.iter()).any(|r| {
                r.run_id == t.run_id && r.task_id.as_deref().is_none_or(|id| id == t.task_id)
            })
    };
    let mut credit: BTreeMap<&str, (u32, u32)> = BTreeMap::new();
    for line in history {
        let HistoryLine::Task(t) = line else { continue };
        let green = match t.outcome {
            TaskOutcome::Merged | TaskOutcome::MergedWithoutApproval => !reverted(t),
            TaskOutcome::Blocked => false,
            TaskOutcome::Cancelled | TaskOutcome::Unfinished | TaskOutcome::Reported => {
                continue;
            }
        };
        let shown: BTreeSet<&str> = t.lessons_shown.iter().map(String::as_str).collect();
        for id in shown {
            let entry = credit.entry(id).or_default();
            entry.0 += u32::from(green);
            entry.1 += 1;
        }
    }
    credit
}

/// The `at` of each run record, one per run.
fn run_times(history: &[HistoryLine]) -> Vec<u64> {
    let mut runs: BTreeMap<&str, u64> = BTreeMap::new();
    for line in history {
        if let HistoryLine::Run(r) = line {
            runs.insert(r.run_id.as_str(), r.at);
        }
    }
    runs.into_values().collect()
}

/// Decision 17 for one lesson.
fn settle_trial(lesson: &mut Lesson, run_times: &[u64], cfg: &Lessons) {
    let Some(start) = lesson.trial_start else {
        return;
    };
    if lesson.status != LessonStatus::OnTrial {
        return;
    }
    let since = run_times.iter().filter(|&&at| at > start).count();
    if since < cfg.episodic_trial_runs as usize {
        return;
    }
    if lesson.green > 0 {
        lesson.status = LessonStatus::Kept;
    } else {
        lesson.status = LessonStatus::Retired;
        lesson.retired_reason = Some(TRIAL_ENDED.to_string());
    }
}

/// The list order of a status.
fn rank(status: LessonStatus) -> u8 {
    match status {
        LessonStatus::Known => 0,
        LessonStatus::Approved => 1,
        LessonStatus::Kept => 2,
        LessonStatus::OnTrial => 3,
        LessonStatus::Proposed => 4,
        LessonStatus::Rejected => 5,
        LessonStatus::Retired => 6,
    }
}

impl Lesson {
    /// What the wire shows of the lesson (decision 32).
    pub fn info(&self) -> LessonInfo {
        LessonInfo {
            id: self.id.clone(),
            layer: self.layer,
            status: self.status,
            origin: self.origin,
            title: self.title.clone(),
            body: self.body.clone(),
            files: self.files.clone(),
            created_at: self.created_at,
            last_seen: self.last_seen,
            runs: self.runs.clone(),
            green: self.green,
            outcomes: self.outcomes,
            reconfirmed: self.reconfirmed,
            headlines: self.headlines.clone(),
            retired_reason: self.retired_reason.clone(),
        }
    }

    /// `1000 × green ÷ outcomes`, rounded down; `None` while unmeasured (decision 22's
    /// usefulness, carried by `LessonBrief`).
    pub fn usefulness(&self) -> Option<u16> {
        (self.outcomes > 0)
            .then(|| (1000 * u64::from(self.green) / u64::from(self.outcomes)) as u16)
    }
}

#[cfg(test)]
#[path = "consolidate_tests.rs"]
mod tests;
