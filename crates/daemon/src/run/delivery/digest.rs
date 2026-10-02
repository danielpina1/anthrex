//! Decision 29: `run_status`'s `delivery` block (Interfaces "The `delivery` block of
//! `run_status` (exact keys)"), its trimming steps past `DIGEST_MAX_BYTES`, and the form
//! the fingerprint reads. Pure.
//!
//! Comment text is the only untrusted text here. It is quoted (`quote::comment`,
//! decision 22), only from a writer or a listed reviewer (the controller's ruling: the
//! same rule as the review intake, `RunDelivery::writes`), and only for a thread that
//! counted and is `new` or in the stage's open batch. A thread that has not counted is
//! not listed. Everything else is counts, ids, logins (shown as decision 22 shows
//! them), paths and states.

use proto::{CiCategory, CiState, DeliveryMode, PrState, TaskOrigin};
use serde_json::{Value, json};

use super::snapshot::{ci_state, stage_count};
use super::{CiPhase, CiRecord, PrRecord, StageDelivery, ThreadRecord, ThreadState, quote};
use crate::run::contract::sha7;
use crate::run::model::Run;
use crate::run::orch::json::{fold_all, label, shrink_strings};

/// A thread's quoted comments, in characters, and what trimming leaves of them.
pub const COMMENT_MAX: usize = 2000;
pub const COMMENT_TRIMMED: usize = 300;
/// A stage's threads shown, and what trimming leaves.
pub const THREADS_SHOWN: usize = 20;
pub const THREADS_TRIMMED: usize = 10;
/// A stage's fix task ids shown per origin, the newest (not in Interfaces).
pub const FIX_TASKS_SHOWN: usize = 20;

/// The fix round's last-resort cuts (the controller's ruling, I1): the threads and
/// fix task ids a stage keeps, and every string's length.
pub const THREADS_LAST: usize = 3;
pub const FIX_TASKS_LAST: usize = 3;
pub const STRINGS_LAST: usize = 40;

/// How much of the block to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shape {
    /// A thread's quoted comments cut to this many characters; `None` leaves the
    /// `comment` key out (the fingerprint's form).
    pub comment_max: Option<usize>,
    /// Threads shown per stage; `None` shows every one (the fingerprint's form).
    pub threads: Option<usize>,
    /// Whether stages whose PR merged or closed are shown.
    pub resolved: bool,
    /// Only the threads the orchestrator must decide on ([`current`]).
    pub current_only: bool,
    /// Every string of the block cut to this many characters.
    pub strings: Option<usize>,
    /// Fix task ids shown per origin, the newest.
    pub fix_tasks: usize,
    /// Each stage only as `stage`, `pr`, `state` and `ci`, with `threads_omitted`.
    pub compact: bool,
}

impl Shape {
    /// What `run_status` shows before any trimming.
    pub const FULL: Shape = Shape {
        comment_max: Some(COMMENT_MAX),
        threads: Some(THREADS_SHOWN),
        resolved: true,
        current_only: false,
        strings: None,
        fix_tasks: FIX_TASKS_SHOWN,
        compact: false,
    };
    /// Milestone 9 decision 16's fingerprint reads every field but `comment`, so a new
    /// thread or a state change wakes a waiting `run_status`, a comment's text does not.
    pub const FINGERPRINT: Shape = Shape {
        comment_max: None,
        threads: None,
        ..Shape::FULL
    };
    /// The untrimmed digest's shape, or the fingerprint's.
    pub fn digest(for_fingerprint: bool) -> Shape {
        if for_fingerprint {
            Shape::FINGERPRINT
        } else {
            Shape::FULL
        }
    }
    /// Decision 29's trimming steps, in order: comments cut to 300, then each stage's
    /// threads to 10, then the stages whose PR merged or closed dropped (the brief's
    /// "then resolved stages").
    pub const TRIMMED: [Shape; 3] = [
        Shape {
            comment_max: Some(COMMENT_TRIMMED),
            ..Shape::FULL
        },
        Shape {
            comment_max: Some(COMMENT_TRIMMED),
            threads: Some(THREADS_TRIMMED),
            ..Shape::FULL
        },
        Shape {
            comment_max: Some(COMMENT_TRIMMED),
            threads: Some(THREADS_TRIMMED),
            resolved: false,
            ..Shape::FULL
        },
    ];
    /// The step taken only before the digest's general string cut, which would cut a
    /// quote's closing fence off: no comments at all (not in Interfaces).
    pub const NO_COMMENTS: Shape = Shape {
        comment_max: None,
        threads: Some(THREADS_TRIMMED),
        resolved: false,
        ..Shape::FULL
    };
    /// The fix round's steps before any unfinished task is dropped (the controller's
    /// ruling, I1), in order: only the threads to decide on, then 3 a stage, then every
    /// string cut to 40 characters, then 3 fix task ids per origin, and last each stage
    /// as its PR, state and CI with a count of the threads left out. The last form is
    /// bounded whatever the run holds: at most `STAGES_MAX` stages of a few numbers.
    pub const LAST: [Shape; 5] = [
        Shape {
            current_only: true,
            ..Shape::NO_COMMENTS
        },
        Shape {
            current_only: true,
            threads: Some(THREADS_LAST),
            ..Shape::NO_COMMENTS
        },
        Shape {
            current_only: true,
            threads: Some(THREADS_LAST),
            strings: Some(STRINGS_LAST),
            ..Shape::NO_COMMENTS
        },
        Shape {
            current_only: true,
            threads: Some(THREADS_LAST),
            strings: Some(STRINGS_LAST),
            fix_tasks: FIX_TASKS_LAST,
            ..Shape::NO_COMMENTS
        },
        Shape {
            current_only: true,
            threads: Some(0),
            strings: Some(STRINGS_LAST),
            fix_tasks: 0,
            compact: true,
            ..Shape::NO_COMMENTS
        },
    ];
}

/// The `delivery` block of `run`: `{"mode": "local"}` in local mode.
pub fn block(run: &Run, shape: Shape) -> Value {
    let d = &run.delivery;
    if d.mode == DeliveryMode::Local {
        return json!({"mode": "local"});
    }
    let count = stage_count(run);
    let stages: Vec<Value> = (1..=count).filter_map(|n| stage(run, n, shape)).collect();
    let mut block = json!({
        "mode": label(&d.mode),
        "repo": d.repo.as_ref().map(|r| r.full()),
        "watching": d.watching,
        "delivering": d.delivering(count),
        "stages": stages,
    });
    if let Some(max) = shape.strings {
        shrink_strings(&mut block, max);
    }
    fold_all(&mut block);
    block
}

fn stage(run: &Run, n: u16, shape: Shape) -> Option<Value> {
    let empty = StageDelivery::default();
    let s = run.delivery.stage(n).unwrap_or(&empty);
    let pr = s.pr.as_ref();
    let resolved = pr.is_some_and(|p| matches!(p.state, PrState::Merged | PrState::Closed));
    if resolved && !shape.resolved {
        return None;
    }
    let ci = pr.map(|p| ci_state(&p.checks));
    if shape.compact {
        let listed = s.threads.iter().filter(|t| listed(t)).count();
        return Some(json!({
            "stage": n,
            "pr": pr.map(|p| p.number),
            "state": pr.map(|p| label(&p.state)),
            "ci": ci.map(|c| label(&c)),
            "threads_omitted": listed,
        }));
    }
    let threads = match pr {
        Some(pr) => threads(run, s, pr, shape),
        None => Vec::new(),
    };
    Some(json!({
        "stage": n,
        "pr": pr.map(|p| p.number),
        "url": pr.map(|p| p.url.as_str()),
        "state": pr.map(|p| label(&p.state)),
        "ci": ci.map(|c| label(&c)),
        // The fix round's m3: a CI line only while the head's CI is red.
        "ci_line": pr.filter(|_| ci == Some(CiState::Red)).and_then(|p| ci_line(run, s, p)),
        "threads": threads,
        "fix_tasks": {
            "ci": fix_tasks(run, n, TaskOrigin::Ci, shape.fix_tasks),
            "review": fix_tasks(run, n, TaskOrigin::Review, shape.fix_tasks),
            "sync": fix_tasks(run, n, TaskOrigin::Sync, shape.fix_tasks),
        },
        "paused": s.paused_by.is_some(),
    }))
}

/// Whether thread `t` is one the orchestrator must decide on: it counted and is `new`,
/// or it is in the stage's open batch.
fn current(s: &StageDelivery, t: &ThreadRecord) -> bool {
    let batched = s.batch.as_ref().is_some_and(|b| b.threads.contains(&t.key));
    t.counted && (t.state == ThreadState::New || batched)
}

/// Whether thread `t` is listed at all: a `new` thread that has not counted (its
/// authors' write access is still being asked) stays out until it counts (decision 29:
/// what does not count never reaches an agent; the fix round's I2). An ignored thread
/// is listed, never with text.
fn listed(t: &ThreadRecord) -> bool {
    t.counted || t.state != ThreadState::New
}

/// The stage's threads: the [`current`] ones first, in the order the PR showed them,
/// then the other listed ones newest first (only the first with `current_only`).
fn threads(run: &Run, s: &StageDelivery, pr: &PrRecord, shape: Shape) -> Vec<Value> {
    let first = s.threads.iter().filter(|t| current(s, t));
    let rest =
        (s.threads.iter().rev()).filter(|t| listed(t) && !current(s, t) && !shape.current_only);
    let all = first.chain(rest);
    let shown: Vec<&ThreadRecord> = match shape.threads {
        Some(max) => all.take(max).collect(),
        None => all.collect(),
    };
    shown
        .into_iter()
        .map(|t| {
            let mut entry = json!({
                "thread": format!("{}:{}", pr.number, t.key),
                "file": t.path,
                "line": t.line,
                "author": quote::login(&t.author),
                "state": state_label(&t.state),
            });
            if let Some(max) = shape.comment_max {
                entry["comment"] = json!(comment(run, s, t, max));
            }
            entry
        })
        .collect()
}

fn state_label(state: &ThreadState) -> &'static str {
    match state {
        ThreadState::New => "new",
        ThreadState::Tasked { .. } => "tasked",
        ThreadState::Replied { .. } => "replied",
        ThreadState::Ignored { .. } => "ignored",
    }
}

/// Thread `t`'s writers' comments, each quoted with its author's label, oldest first,
/// in at most `max` characters: the newest are kept when they do not all fit.
fn comment(run: &Run, s: &StageDelivery, t: &ThreadRecord, max: usize) -> Option<String> {
    if !current(s, t) {
        return None;
    }
    let writes = |login: &str| run.delivery.writes(login) == Some(true);
    let texts: Vec<(&str, &str)> = if t.comments.is_empty() {
        vec![(t.author.as_str(), t.text.as_str())]
    } else {
        (t.comments.iter())
            .map(|c| (c.author.as_str(), c.text.as_str()))
            .collect()
    };
    let mut left = max;
    let mut quoted = Vec::new();
    for (login, text) in texts.into_iter().rev() {
        if text.is_empty() || !writes(login) {
            continue;
        }
        let Some(block) = quote::comment_within(login, text, left) else {
            break;
        };
        left -= block.chars().count();
        quoted.push(block);
    }
    if quoted.is_empty() {
        return None;
    }
    quoted.reverse();
    Some(quoted.concat())
}

/// `<what> failed at <sha7>: <what anthrex is doing>` for the newest red CI record of
/// the PR's pushed head, e.g. `test failed at 1a2b3c4: fix task fix3 working`; `None`
/// when that head has none (task M9.2.13's wording past the Interfaces' example).
fn ci_line(run: &Run, s: &StageDelivery, pr: &PrRecord) -> Option<String> {
    let rec = s.ci.iter().rev().find(|r| r.head == pr.pushed_head)?;
    let what = match rec.category {
        Some(c) if c != CiCategory::Unknown => label(&c),
        _ => "CI".into(),
    };
    Some(format!(
        "{what} failed at {}: {}",
        sha7(&rec.head),
        doing(run, rec)
    ))
}

fn doing(run: &Run, rec: &CiRecord) -> String {
    match rec.phase {
        CiPhase::Logs => "reading the failed logs".into(),
        CiPhase::Summarising => "summarising the log".into(),
        CiPhase::Rerunning => "re-running the failed jobs".into(),
        CiPhase::Reproducing => "reproducing it locally".into(),
        CiPhase::Bisecting => "bisecting the stage".into(),
        CiPhase::Tasked => match rec.fix_task.as_deref() {
            Some(id) => {
                let state = run.task(id).map_or("gone", |t| t.state.label());
                format!("fix task {id} {state}")
            }
            None => "a fix task was made".into(),
        },
        CiPhase::ToUser => "handed to the user".into(),
    }
}

/// The ids of stage `n`'s tasks of `origin`, in plan order, the newest `keep`.
fn fix_tasks(run: &Run, n: u16, origin: TaskOrigin, keep: usize) -> Vec<String> {
    let ids: Vec<String> = (run.tasks.iter())
        .filter(|t| t.stage() == n && t.origin == origin)
        .map(|t| t.id().to_string())
        .collect();
    let skip = ids.len().saturating_sub(keep);
    ids.into_iter().skip(skip).collect()
}

/// Decision 29's trimming steps ([`Shape::TRIMMED`]) on the digest's `delivery` key,
/// taken before anything else in the digest is cut, each rebuilt from the run so a
/// quote is cut before it is fenced; stops as soon as `fits`. True when it fits.
pub fn trim(digest: &mut Value, run: &Run, fits: impl Fn(&Value) -> bool) -> bool {
    for shape in Shape::TRIMMED {
        if fits(digest) || run.delivery.mode == DeliveryMode::Local {
            break;
        }
        digest["delivery"] = block(run, shape);
    }
    fits(digest)
}

/// [`Shape::NO_COMMENTS`] on the digest's `delivery` key (a local block stays).
pub fn drop_comments(digest: &mut Value, run: &Run) {
    if run.delivery.mode != DeliveryMode::Local {
        digest["delivery"] = block(run, Shape::NO_COMMENTS);
    }
}

/// The fix round's [`Shape::LAST`] steps on the digest's `delivery` key, taken before
/// any unfinished task is dropped; stops as soon as `fits`. True when it fits.
pub fn last_steps(digest: &mut Value, run: &Run, fits: impl Fn(&Value) -> bool) -> bool {
    for shape in Shape::LAST {
        if fits(digest) || run.delivery.mode == DeliveryMode::Local {
            break;
        }
        digest["delivery"] = block(run, shape);
    }
    fits(digest)
}
