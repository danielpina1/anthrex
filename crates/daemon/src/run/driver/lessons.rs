//! Milestone 9.15 decision 21, the driver side: a run's start freezes the repository's
//! active lessons into the run ([`lessons_for_start`], called by `build_delivered`
//! only, concurrently with the tuning: ruling C7). A lesson that appears after the
//! start reaches only later runs.
//!
//! **Locks.** None: a read of the append-only store takes no lock (decision 3), and
//! the history is read as `run stats` reads it. Both reads, the fold and the
//! consolidation run on `spawn_blocking`, within [`LESSONS_START_BOUND`]; past it the
//! run starts with no lessons and logs [`LESSONS_SLOW`].

use std::path::Path;
use std::time::Duration;

use proto::LessonStatus;

use crate::lessons::consolidate::consolidate;
use crate::lessons::fold::fold;
use crate::lessons::{LESSONS_FILE, store};
use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::read_history;
use crate::run::model::{LessonBrief, LessonLimits, LogEntry, Run, RunLessons};
pub use common::bounds::LESSONS_START_BOUND;

/// At most this many active lessons are frozen into a run.
pub const FROZEN_MAX: usize = 50;

/// The run log's line for a start whose read did not end by [`LESSONS_START_BOUND`].
pub const LESSONS_SLOW: &str = "lessons: none (the lessons store could not be read in time)";
/// The run log's line for a start with `[lessons] enabled = false`.
pub const LESSONS_OFF: &str = "lessons: off";
/// The run log's line for a start with no active lesson.
pub const LESSONS_NONE: &str = "lessons: none";

/// Decision 21 for a start in the repository whose data directory is `repo_dir`: the
/// frozen lessons (always `Some` for a new start, ruling C2: the limits are frozen
/// even when lessons are off, since `risk_note` is independent of `enabled`) and the
/// run log's one `lessons:` line. A run with history off (an empty `repo_dir`) reads
/// nothing.
pub async fn lessons_for_start(
    repo_dir: &Path,
    cfg: &config::Lessons,
    now: u64,
) -> (RunLessons, String) {
    lessons_within(repo_dir, cfg, now, LESSONS_START_BOUND).await
}

/// [`lessons_for_start`] with its bound given (a test's).
pub async fn lessons_within(
    repo_dir: &Path,
    cfg: &config::Lessons,
    now: u64,
    bound: Duration,
) -> (RunLessons, String) {
    let frozen = |items: Vec<LessonBrief>| RunLessons {
        limits: LessonLimits::from(cfg),
        items,
        frozen_at: now,
    };
    if !cfg.enabled {
        return (frozen(Vec::new()), LESSONS_OFF.to_string());
    }
    if repo_dir.as_os_str().is_empty() {
        return (frozen(Vec::new()), LESSONS_NONE.to_string());
    }
    let (dir, config) = (repo_dir.to_path_buf(), cfg.clone());
    let work = tokio::task::spawn_blocking(move || active(&dir, &config, now));
    match tokio::time::timeout(bound, work).await {
        Ok(Ok((items, active, known))) => {
            let line = match active {
                0 => LESSONS_NONE.to_string(),
                n => format!("lessons: {n} active ({known} known failures)"),
            };
            (frozen(items), line)
        }
        Ok(Err(error)) => {
            tracing::warn!(%error, "the lessons read did not finish; the run starts without");
            (frozen(Vec::new()), LESSONS_SLOW.to_string())
        }
        Err(_) => {
            let secs = bound.as_secs_f64();
            tracing::warn!("the lessons read took over {secs} s; the run starts without");
            (frozen(Vec::new()), LESSONS_SLOW.to_string())
        }
    }
}

/// Decision 21: `run` takes the start's frozen lessons, and its log the `lessons:`
/// line, after the tuning lines (ruling C25).
pub fn freeze(run: &mut Run, (lessons, line): (RunLessons, String)) {
    let at = lessons.frozen_at;
    run.lessons = Some(lessons);
    run.log.push(LogEntry { at, text: line });
}

/// Decision 18's active statuses.
fn is_active(status: LessonStatus) -> bool {
    matches!(
        status,
        LessonStatus::Known | LessonStatus::Approved | LessonStatus::Kept | LessonStatus::OnTrial
    )
}

/// [`lessons_for_start`]'s blocking core: the first [`FROZEN_MAX`] active lessons in
/// decision 21's order (consolidation's: status, then `last_seen` newest first, then
/// id), how many are active, and how many of those are known failures.
fn active(repo_dir: &Path, cfg: &config::Lessons, now: u64) -> (Vec<LessonBrief>, usize, usize) {
    let (lines, _) = store::read_store(&repo_dir.join(LESSONS_FILE));
    let (history, _) = read_history(&repo_dir.join(HISTORY_FILE));
    let all: Vec<_> = consolidate(&fold(&lines), &history, now, cfg)
        .into_iter()
        .filter(|l| is_active(l.status))
        .collect();
    let known = all
        .iter()
        .filter(|l| l.status == LessonStatus::Known)
        .count();
    let count = all.len();
    let items = (all.into_iter().take(FROZEN_MAX))
        .map(|l| LessonBrief {
            usefulness: l.usefulness(),
            id: l.id,
            layer: l.layer,
            status: l.status,
            title: l.title,
            body: l.body,
            files: l.files,
            last_seen: l.last_seen,
        })
        .collect();
    (items, count, known)
}

#[cfg(test)]
#[path = "lessons_tests.rs"]
mod tests;
