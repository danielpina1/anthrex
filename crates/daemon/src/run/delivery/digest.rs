//! Decision 29: `run_status`'s `delivery` block (Interfaces "The `delivery` block of
//! `run_status` (exact keys)"), its trimming steps past `DIGEST_MAX_BYTES`, and the form
//! the fingerprint reads. Pure.
//!
//! Comment text is the only untrusted text here. It is quoted (`quote::comment`,
//! decision 22), only from a writer or a listed reviewer (the controller's ruling: the
//! same rule as the review intake, `RunDelivery::writes`), and only for a thread that
//! counted and is `new` or in the stage's open batch. Everything else is counts, ids,
//! logins (shown as decision 22 shows them), paths and states.

use proto::{CiCategory, DeliveryMode, PrState, TaskOrigin};
use serde_json::{Value, json};

use super::snapshot::{ci_state, stage_count};
use super::{CiPhase, CiRecord, PrRecord, StageDelivery, ThreadRecord, ThreadState, quote};
use crate::run::contract::sha7;
use crate::run::model::Run;
use crate::run::orch::json::{fold_all, label};

/// A thread's quoted comments, in characters, and what trimming leaves of them.
pub const COMMENT_MAX: usize = 2000;
pub const COMMENT_TRIMMED: usize = 300;
/// A stage's threads shown, and what trimming leaves.
pub const THREADS_SHOWN: usize = 20;
pub const THREADS_TRIMMED: usize = 10;
/// A stage's fix task ids shown per origin, the newest (not in Interfaces).
pub const FIX_TASKS_SHOWN: usize = 20;

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
}

impl Shape {
    /// What `run_status` shows before any trimming.
    pub const FULL: Shape = Shape {
        comment_max: Some(COMMENT_MAX),
        threads: Some(THREADS_SHOWN),
        resolved: true,
    };
    /// Milestone 9 decision 16's fingerprint reads every field but `comment`, so a new
    /// thread or a state change wakes a waiting `run_status`, a comment's text does not.
    pub const FINGERPRINT: Shape = Shape {
        comment_max: None,
        threads: None,
        resolved: true,
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
            resolved: true,
        },
        Shape {
            comment_max: Some(COMMENT_TRIMMED),
            threads: Some(THREADS_TRIMMED),
            resolved: false,
        },
    ];
    /// The last step, taken only before the digest's general string cut, which would
    /// cut a quote's closing fence off: no comments at all (not in Interfaces).
    pub const NO_COMMENTS: Shape = Shape {
        comment_max: None,
        threads: Some(THREADS_TRIMMED),
        resolved: false,
    };
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
    let threads = match pr {
        Some(pr) => threads(run, s, pr, shape),
        None => Vec::new(),
    };
    Some(json!({
        "stage": n,
        "pr": pr.map(|p| p.number),
        "url": pr.map(|p| p.url.as_str()),
        "state": pr.map(|p| label(&p.state)),
        "ci": pr.map(|p| label(&ci_state(&p.checks))),
        "ci_line": pr.and_then(|p| ci_line(run, s, p)),
        "threads": threads,
        "fix_tasks": {
            "ci": fix_tasks(run, n, TaskOrigin::Ci),
            "review": fix_tasks(run, n, TaskOrigin::Review),
            "sync": fix_tasks(run, n, TaskOrigin::Sync),
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

/// The stage's threads: the `new` and batched ones first, then the rest newest first,
/// each group in the order the PR showed them.
fn threads(run: &Run, s: &StageDelivery, pr: &PrRecord, shape: Shape) -> Vec<Value> {
    let open = |t: &&ThreadRecord| t.state == ThreadState::New || current(s, t);
    let first = s.threads.iter().filter(open);
    let rest = s.threads.iter().rev().filter(|t| !open(t));
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

/// The ids of stage `n`'s tasks of `origin`, in plan order, the newest
/// [`FIX_TASKS_SHOWN`].
fn fix_tasks(run: &Run, n: u16, origin: TaskOrigin) -> Vec<String> {
    let ids: Vec<String> = (run.tasks.iter())
        .filter(|t| t.stage() == n && t.origin == origin)
        .map(|t| t.id().to_string())
        .collect();
    let skip = ids.len().saturating_sub(FIX_TASKS_SHOWN);
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
