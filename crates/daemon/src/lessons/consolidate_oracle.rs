//! The test oracle of task M9.15.6's review, I1: the known-failure derivation as it was
//! before it was made linear (`94edd7b1`'s `live`, `superseded_at` and `formation`,
//! kept verbatim but for returning a range and counting its scans). The property test
//! `consolidate_tests_scale.rs` checks that [`super::known`] gives the same lessons.

use std::collections::{BTreeSet, HashSet};
use std::ops::Range;

use config::Lessons;
use proto::{HistoryLine, SigRef, TaskOutcome, TaskRecord};

use super::super::fold::{Lesson, Store};
use super::{DAY_SECS, KNOWN_FILES_MAX, count_scan, known_by, task_records};
use crate::run::globs::any_intersect;

/// [`super::known`] through the old life rule.
pub fn known(history: &[HistoryLine], store: &Store, now: u64, cfg: &Lessons) -> Vec<Lesson> {
    let tasks = task_records(history);
    known_by(history, &tasks, store, |fp, hits| {
        live(fp, hits, &tasks, now, cfg)
    })
}

fn live(
    fp: &str,
    hits: &[(&TaskRecord, &SigRef)],
    tasks: &[&TaskRecord],
    now: u64,
    cfg: &Lessons,
) -> Option<Range<usize>> {
    let expiry = u64::from(cfg.expiry_days) * DAY_SECS;
    let hit_runs: HashSet<&str> = hits.iter().map(|(t, _)| t.run_id.as_str()).collect();
    let mut after = 0;
    loop {
        let (first, formed) = formation(&hits[after..], cfg)?;
        let (start, formed) = (after + first, after + formed);
        let formed_at = hits[formed].0.at;
        let mut end = formed + 1;
        let next_start = loop {
            let files = files_of(&hits[start..end]);
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
                None => return Some(start..end),
            }
        };
        after = next_start?;
    }
}

fn superseded_at(
    fp: &str,
    formed_at: u64,
    files: &[String],
    tasks: &[&TaskRecord],
    hit_runs: &HashSet<&str>,
    cfg: &Lessons,
) -> Option<u64> {
    let mut clean: Vec<&TaskRecord> = (tasks.iter().copied())
        .inspect(|_| count_scan())
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

/// The old `gather`'s files.
fn files_of(hits: &[(&TaskRecord, &SigRef)]) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    for (task, _) in hits {
        for own in &task.owns {
            if files.len() < KNOWN_FILES_MAX && !files.contains(own) {
                files.push(own.clone());
            }
        }
    }
    files
}

fn formation(hits: &[(&TaskRecord, &SigRef)], cfg: &Lessons) -> Option<(usize, usize)> {
    let window = u64::from(cfg.known_window_days) * DAY_SECS;
    let expiry = u64::from(cfg.expiry_days) * DAY_SECS;
    let mut alive = 0;
    for j in 0..hits.len() {
        if j > 0 && hits[j].0.at - hits[j - 1].0.at > expiry {
            alive = j;
        }
        let end = hits[j].0.at;
        let first = (alive..=j)
            .find(|&k| end - hits[k].0.at <= window)
            .unwrap_or(j);
        let runs: BTreeSet<&str> = (hits[first..=j].iter())
            .map(|(t, _)| t.run_id.as_str())
            .collect();
        if runs.len() >= cfg.known_min as usize {
            return Some((first, j));
        }
    }
    None
}
