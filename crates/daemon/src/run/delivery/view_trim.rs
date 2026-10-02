//! What the run keeps of review text (task M9.2.10), in `run.json` and in the journal.
//! A stage's threads keep the text of their newest [`THREAD_COMMENTS_KEPT`] comments,
//! each cut to [`COMMENT_KEPT_CHARS`], within [`STAGE_TEXT_CHARS`] in all; every id is
//! kept (newness is by id). The engine's `review::trim` holds `run.json` to it.
//!
//! The fix round's ruling on concern 2: the journal never carries what `run.json`
//! refuses, so a `PrViewed` answer is cut to the same caps before it is journaled
//! ([`journaled`]; the engine is handed the same cut view, so a replay sees what the
//! live run saw). Reviews are cut first (a changes-requested body is decision-bearing),
//! then conversation comments, then threads, newest first in each. A body the budget
//! empties becomes [`CUT_MARK`], never `""`, so a changes-requested review still reads
//! as one; and anthrex's marker at the end of a cut body is kept, so a reply it sent is
//! still known by it. A thread comment's diff hunk is kept for its first comment only
//! (the engine reads no other). Pure (design decision 1).

use crate::host::PrView;
use crate::run::delivery::ops::HostResult;
use crate::run::engine::OpResult;

/// A thread keeps the text of its newest this many comments.
pub const THREAD_COMMENTS_KEPT: usize = 20;
/// Each kept comment is cut to this many characters.
pub const COMMENT_KEPT_CHARS: usize = 4_000;
/// A stage (and a view) keeps at most this many characters of review text in all.
pub const STAGE_TEXT_CHARS: usize = 200_000;
/// What a kept body the budget emptied reads.
pub const CUT_MARK: &str = "…";
/// The longest marker tail kept after a cut (a marker is about 60 characters).
const MARKER_TAIL_MAX: usize = 200;
const MARKER_OPEN: &str = "<!-- anthrex:reply ";

/// `result` as it is journaled and handed to the engine: a view cut to the caps,
/// anything else as it is.
pub fn journaled(result: OpResult) -> OpResult {
    match result {
        OpResult::Host(HostResult::PrViewed(mut view)) => {
            trim_view(&mut view);
            OpResult::Host(HostResult::PrViewed(view))
        }
        other => other,
    }
}

/// The final fix wave's A4: `result` as its journal `done` line keeps it. A CI log's
/// text never goes there (only its path and size): a replayed answer has an empty tail,
/// and the engine fetches that log again.
pub fn journal_line(result: &OpResult) -> OpResult {
    let mut line = result.clone();
    if let OpResult::Host(HostResult::Logs(file)) = &mut line {
        file.tail.clear();
    }
    line
}

fn trim_view(view: &mut PrView) {
    let mut budget = STAGE_TEXT_CHARS;
    // The final fix wave (task 10's deferred item): a marker tail kept after a cut is
    // text too. Tails are paid for first, from the same budget, while it lasts: the
    // newest thread comments first, then conversation comments, then reviews (where
    // anthrex's replies are, in that order of likelihood); a body's tail that does not
    // fit is cut with the body.
    let mut paid = |body: &str| match marker_tail(body).map(|t| t.chars().count()) {
        Some(n) if n <= budget => {
            budget -= n;
            true
        }
        _ => false,
    };
    let thread_tails: Vec<Vec<bool>> = (view.threads.iter().rev())
        .map(|t| t.comments.iter().rev().map(|c| paid(&c.body)).collect())
        .collect();
    let comment_tails: Vec<bool> = view.comments.iter().rev().map(|c| paid(&c.body)).collect();
    let review_tails: Vec<bool> = view.reviews.iter().rev().map(|r| paid(&r.body)).collect();
    for (r, tail) in view.reviews.iter_mut().rev().zip(review_tails) {
        cut_body(&mut r.body, true, tail, &mut budget);
    }
    for (c, tail) in view.comments.iter_mut().rev().zip(comment_tails) {
        cut_body(&mut c.body, true, tail, &mut budget);
    }
    for (t, tails) in view.threads.iter_mut().rev().zip(thread_tails) {
        let len = t.comments.len();
        let each = t.comments.iter_mut().enumerate().rev().zip(tails);
        for ((i, c), tail) in each {
            cut_body(
                &mut c.body,
                len - i <= THREAD_COMMENTS_KEPT,
                tail,
                &mut budget,
            );
            if i == 0 {
                cut_text(&mut c.diff_hunk, &mut budget);
            } else {
                c.diff_hunk.clear();
            }
        }
    }
}

/// The marker anthrex ends its replies with, when `body` ends with one.
fn marker_tail(body: &str) -> Option<&str> {
    let at = body.rfind(MARKER_OPEN)?;
    let tail = &body[at..];
    (tail.ends_with("-->") && tail.chars().count() <= MARKER_TAIL_MAX).then_some(tail)
}

/// Cuts a body: to the comment cap within the budget when `keep`, else to nothing; a
/// kept body is never emptied, and a marker at its end whose room was `paid` is kept
/// after the cut (its characters are not counted again).
fn cut_body(body: &mut String, keep: bool, paid: bool, budget: &mut usize) {
    let max = if keep {
        COMMENT_KEPT_CHARS.min(*budget)
    } else {
        0
    };
    let tail = paid
        .then(|| marker_tail(body).map(str::to_string))
        .flatten();
    let count = body.chars().count();
    let text = count - tail.as_ref().map_or(0, |t| t.chars().count());
    if text <= max {
        *budget -= text;
        return;
    }
    let mut head: String = body.chars().take(max).collect();
    *budget -= head.chars().count();
    if keep && head.trim().is_empty() && !body.trim().is_empty() {
        head = CUT_MARK.to_string();
    }
    if let Some(tail) = tail {
        if !head.is_empty() {
            head.push_str("\n\n");
        }
        head.push_str(&tail);
    }
    *body = head;
}

/// Cuts plain text to the comment cap within the budget.
fn cut_text(text: &mut String, budget: &mut usize) {
    let max = COMMENT_KEPT_CHARS.min(*budget);
    if let Some((at, _)) = text.char_indices().nth(max) {
        text.truncate(at);
    }
    *budget -= text.chars().count().min(*budget);
}
