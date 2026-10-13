//! Milestone 9.15 decision 22: which of a run's frozen lessons a task's prompts show.
//! Pure, and in integer per-mille arithmetic so it is exact on every platform:
//! `score = 450·overlap + 350·usefulness + 200·recency`, each component 0–1000, kept at
//! [`MIN_SCORE`] or more. Only an active lesson is ever selected (decision 18): a
//! proposed one never is, whatever its score (Review focus 6).
//!
//! The items are rendered here (one line each: `one_line`d, marker-broken, clipped to
//! `item_chars`) and bounded so the block under either heading fits `block_chars`, so
//! the worker and the reviewer always show the same ids (ruling C8).

use proto::LessonStatus;
use proto::safe_text::one_line;

use super::consolidate::DAY_SECS;
use super::dossier::break_markers;
use crate::run::contract_lessons::{REVIEWER_HEADING, WORKER_HEADING};
use crate::run::globs::intersects;
use crate::run::model::{LessonBrief, LessonLimits};

/// The least score a lesson is shown at.
pub const MIN_SCORE: u32 = 350_000;
/// Usefulness when none is measured, and the floor of the measured median.
const UNMEASURED_FLOOR: u32 = 500;
/// A file-less semantic or procedural lesson's overlap.
const NO_FILES_GENERAL: u32 = 300;
/// Recency falls to zero over this many days.
const RECENCY_DAYS: u64 = 90;

/// What [`pick`] chose: the ids shown and their rendered lines, in order, and how many
/// more qualified but did not fit `block_chars`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Picked {
    pub ids: Vec<String>,
    pub items: Vec<String>,
    pub more: usize,
}

/// Decision 18's active statuses: the only ones selected.
fn selectable(status: LessonStatus) -> bool {
    matches!(
        status,
        LessonStatus::OnTrial | LessonStatus::Kept | LessonStatus::Approved | LessonStatus::Known
    )
}

/// `1000 ×` the share of the lesson's files that intersect an `owns` glob; a lesson
/// with no files: 300 when semantic or procedural, 0 when episodic.
pub fn overlap(lesson: &LessonBrief, owns: &[String]) -> u32 {
    if lesson.files.is_empty() {
        return match lesson.layer {
            proto::LessonLayer::Episodic => 0,
            _ => NO_FILES_GENERAL,
        };
    }
    let hits = (lesson.files.iter())
        .filter(|f| owns.iter().any(|o| intersects(f, o)))
        .count();
    (1000 * hits / lesson.files.len()) as u32
}

/// An unmeasured lesson's usefulness: the median of the measured ones' (the mean of
/// the middle two for an even count, rounded down), floored at 500; 500 when none is
/// measured.
pub fn unmeasured(items: &[LessonBrief]) -> u32 {
    let mut measured: Vec<u32> = (items.iter())
        .filter_map(|l| l.usefulness.map(u32::from))
        .collect();
    if measured.is_empty() {
        return UNMEASURED_FLOOR;
    }
    measured.sort_unstable();
    let n = measured.len();
    let median = match n % 2 {
        1 => measured[n / 2],
        _ => (measured[n / 2 - 1] + measured[n / 2]) / 2,
    };
    median.max(UNMEASURED_FLOOR)
}

/// Decision 22's score of `lesson` for a task owning `owns`, at `now`, with
/// `unmeasured` standing in for a missing usefulness.
pub fn score(lesson: &LessonBrief, owns: &[String], now: u64, unmeasured: u32) -> u32 {
    let usefulness = lesson.usefulness.map_or(unmeasured, u32::from).min(1000);
    let age_days = now.saturating_sub(lesson.last_seen) / DAY_SECS;
    let recency = (1000 * (RECENCY_DAYS - age_days.min(RECENCY_DAYS)) / RECENCY_DAYS) as u32;
    450 * overlap(lesson, owns) + 350 * usefulness + 200 * recency
}

/// `- [<id> · <layer>] <title> — <body>`, on one line, marker-broken, clipped to
/// `item_chars` characters.
fn line(lesson: &LessonBrief, item_chars: u32) -> String {
    let clean = |text: &str| break_markers(&one_line(text));
    let text = format!(
        "- [{} · {}] {} — {}",
        clean(&lesson.id),
        lesson.layer.name(),
        clean(&lesson.title),
        clean(&lesson.body)
    );
    // Clipping cannot make a new run of dashes: it only drops a tail.
    text.chars().take(item_chars as usize).collect()
}

/// The block: `heading`, one line per item, and `(<n> more not shown)` when some did
/// not fit; empty when nothing is shown.
pub fn render(heading: &str, picked: &Picked) -> String {
    if picked.items.is_empty() {
        return String::new();
    }
    let mut out = heading.to_string();
    for item in &picked.items {
        out.push('\n');
        out.push_str(item);
    }
    if picked.more > 0 {
        out.push_str(&format!("\n({} more not shown)", picked.more));
    }
    out
}

/// The block's length in characters for either heading, whichever is longer, so both
/// fit `block_chars` with the same items.
fn block_len(picked: &Picked) -> usize {
    [WORKER_HEADING, REVIEWER_HEADING]
        .map(|h| render(h, picked).chars().count())
        .into_iter()
        .max()
        .unwrap_or(0)
}

/// Decision 22: the lessons of `items` a task owning `owns` is shown, scored at `now`
/// (the run's freeze time, ruling C8): selectable and at [`MIN_SCORE`] or more, by
/// score descending then id, at most `max_items`, and the last dropped first until the
/// block fits `block_chars`.
pub fn pick(items: &[LessonBrief], owns: &[String], now: u64, limits: &LessonLimits) -> Picked {
    let fallback = unmeasured(items);
    let mut scored: Vec<(u32, &LessonBrief)> = (items.iter())
        .filter(|l| selectable(l.status))
        .map(|l| (score(l, owns, now, fallback), l))
        .filter(|(s, _)| *s >= MIN_SCORE)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    scored.truncate(limits.max_items as usize);
    let mut picked = Picked {
        ids: scored.iter().map(|(_, l)| l.id.clone()).collect(),
        items: (scored.iter())
            .map(|(_, l)| line(l, limits.item_chars))
            .collect(),
        more: 0,
    };
    while !picked.items.is_empty() && block_len(&picked) > limits.block_chars as usize {
        picked.items.pop();
        picked.ids.pop();
        picked.more += 1;
    }
    if picked.items.is_empty() {
        return Picked::default();
    }
    picked
}

#[cfg(test)]
#[path = "select_tests.rs"]
mod tests;
