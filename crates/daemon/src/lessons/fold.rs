//! Milestone 9.15 decision 4: the lessons store folded into its lessons and its
//! distill entries. Pure and total: lines apply in file order; a line of an unknown
//! `type`, or one that fails to parse, is skipped and counted in [`Store::problems`];
//! a `verdict` or `retired` naming no lesson is ignored.
//!
//! The fold keeps only what the log says. `kept`, a trial's end and known failures are
//! derived on read by consolidation (decisions 7, 8 and 17), never stored.

use std::collections::{BTreeMap, BTreeSet};

use proto::{DistillCause, LessonLayer, LessonLine, LessonOrigin, LessonStatus};
use sha2::{Digest, Sha256};

/// The most origin runs a lesson keeps, newest last.
pub const RUNS_MAX: usize = 10;

/// The title fingerprint's hex characters (decision 15).
const FP_CHARS: usize = 16;

/// The store as the log leaves it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    /// Distilled lessons by id (`l-<title_fp>`).
    pub lessons: BTreeMap<String, Lesson>,
    /// Distill entries by their `queued` record id.
    pub queued: BTreeMap<String, Queued>,
    /// Dismissed known failures' fingerprints (decision 48).
    pub dismissed: BTreeSet<String>,
    /// One per line that was skipped.
    pub problems: Vec<String>,
}

/// One distilled lesson as the log leaves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lesson {
    pub id: String,
    pub layer: LessonLayer,
    /// `on_trial`, `proposed`, `approved`, `rejected` or `retired`.
    pub status: LessonStatus,
    pub origin: LessonOrigin,
    pub title: String,
    pub body: String,
    pub files: Vec<String>,
    pub created_at: u64,
    pub last_seen: u64,
    /// An episodic lesson's trial start; restarted by a reconfirmation.
    pub trial_start: Option<u64>,
    /// The runs it was distilled from, at most [`RUNS_MAX`], newest last.
    pub runs: Vec<String>,
    pub reconfirmed: u32,
    pub retired_reason: Option<String>,
    /// The `queued` record id it was first distilled from (the graph's
    /// `from_signature` edges).
    pub queued: Option<String>,
}

/// One distill entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub task_id: Option<String>,
    pub cause: DistillCause,
    pub fps: Vec<String>,
    /// No `distilled` and no `gave_up` yet.
    pub open: bool,
    pub attempts: u32,
}

/// Decision 15: the first 16 hex characters of SHA-256 over the title lowercased, split
/// on every non-alphanumeric character, empty pieces dropped, joined by one space.
pub fn title_fp(title: &str) -> String {
    let lower = title.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let digest = Sha256::digest(words.join(" ").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..FP_CHARS].to_string()
}

/// Every line of `text` that parses, the last line of each `record_id` kept in that
/// line's place (as `read_history` does), and one problem per non-blank line that does
/// not parse (an unknown `type` or a torn line), named `<origin> line <n>: <error>`.
pub fn parse(text: &str, origin: &str) -> (Vec<LessonLine>, Vec<String>) {
    let mut lines: Vec<Option<LessonLine>> = Vec::new();
    let mut last: BTreeMap<String, usize> = BTreeMap::new();
    let mut problems = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<LessonLine>(raw) {
            Ok(line) => {
                if let Some(earlier) = last.insert(line.record_id().to_string(), lines.len()) {
                    lines[earlier] = None;
                }
                lines.push(Some(line));
            }
            Err(error) => problems.push(format!("{origin} line {}: {error}", n + 1)),
        }
    }
    (lines.into_iter().flatten().collect(), problems)
}

/// [`parse`] then [`fold`], the parse's problems kept.
pub fn fold_text(text: &str, origin: &str) -> Store {
    let (lines, problems) = parse(text, origin);
    Store {
        problems,
        ..fold(&lines)
    }
}

/// Decision 4: `lines` applied in order.
pub fn fold(lines: &[LessonLine]) -> Store {
    let mut store = Store::default();
    for line in lines {
        apply(&mut store, line);
    }
    store
}

fn apply(store: &mut Store, line: &LessonLine) {
    match line {
        LessonLine::Queued(q) => {
            store.queued.entry(q.record_id.clone()).or_insert(Queued {
                record_id: q.record_id.clone(),
                at: q.at,
                run_id: q.run_id.clone(),
                task_id: q.task_id.clone(),
                cause: q.cause,
                fps: q.fps.clone(),
                open: true,
                attempts: 0,
            });
        }
        LessonLine::Attempt(a) => {
            if let Some(entry) = store.queued.get_mut(&a.queued) {
                entry.attempts += 1;
            }
        }
        LessonLine::Distilled(d) => {
            let run = store.queued.get_mut(&d.queued).map(|entry| {
                entry.open = false;
                entry.run_id.clone()
            });
            for item in &d.items {
                distil(store, item, d.at, &d.queued, run.as_deref());
            }
        }
        LessonLine::GaveUp(g) => {
            if let Some(entry) = store.queued.get_mut(&g.queued) {
                entry.open = false;
            }
        }
        LessonLine::Verdict(v) => {
            // Ruling C16: as `Decide`, only a proposed or on-trial lesson takes a verdict.
            if let Some(lesson) = store.lessons.get_mut(&v.id)
                && matches!(
                    lesson.status,
                    LessonStatus::Proposed | LessonStatus::OnTrial
                )
            {
                lesson.status = if v.approved {
                    LessonStatus::Approved
                } else {
                    LessonStatus::Rejected
                };
            }
        }
        LessonLine::Retired(r) => {
            // As `Forget`: an active or proposed lesson only; the first reason stays.
            if let Some(lesson) = store.lessons.get_mut(&r.id)
                && !matches!(
                    lesson.status,
                    LessonStatus::Rejected | LessonStatus::Retired
                )
            {
                lesson.status = LessonStatus::Retired;
                lesson.retired_reason = Some(r.reason.clone());
            }
        }
        LessonLine::Dismissed(d) => {
            store.dismissed.insert(d.fp.clone());
        }
    }
}

/// Decision 16's doors for one distilled item.
fn distil(store: &mut Store, item: &proto::LessonDraft, at: u64, queued: &str, run: Option<&str>) {
    let id = format!("l-{}", title_fp(&item.title));
    if let Some(lesson) = store.lessons.get_mut(&id) {
        match lesson.status {
            LessonStatus::Rejected | LessonStatus::Retired => {}
            _ => {
                lesson.reconfirmed += 1;
                lesson.last_seen = at;
                if lesson.layer == LessonLayer::Episodic {
                    lesson.trial_start = Some(at);
                }
                if let Some(run) = run {
                    add_run(&mut lesson.runs, run);
                }
            }
        }
        return;
    }
    let episodic = item.layer == LessonLayer::Episodic;
    let mut runs = Vec::new();
    if let Some(run) = run {
        add_run(&mut runs, run);
    }
    store.lessons.insert(
        id.clone(),
        Lesson {
            id,
            layer: item.layer,
            status: if episodic {
                LessonStatus::OnTrial
            } else {
                LessonStatus::Proposed
            },
            origin: LessonOrigin::Distilled,
            title: item.title.clone(),
            body: item.body.clone(),
            files: item.files.clone(),
            created_at: at,
            last_seen: at,
            trial_start: episodic.then_some(at),
            runs,
            reconfirmed: 0,
            retired_reason: None,
            queued: Some(queued.to_string()),
        },
    );
}

/// `run` added once, newest last, the oldest dropped past [`RUNS_MAX`].
fn add_run(runs: &mut Vec<String>, run: &str) {
    if runs.iter().any(|r| r == run) {
        return;
    }
    runs.push(run.to_string());
    if runs.len() > RUNS_MAX {
        runs.remove(0);
    }
}

#[cfg(test)]
#[path = "fold_tests.rs"]
mod tests;
