//! Milestone 9.2 decision 42: the pull-request part of a stage node's content row,
//! after its tier 3 (`stage 2 of 3  tier 3 ✓  #142  ci ✓  2 threads`). Pure, and nothing
//! in it is agent or host text: a number, the CI mark (`theme::ci_look`, with its ASCII
//! twin) and counts.

use unicode_width::UnicodeWidthStr;

use crate::theme::{Role, ci_look};
use crate::tree::{Row, RowKind};
use crate::ui::tree_view::truncate_in;
use proto::{DeliveryMode, PrState, RunInfo, StageInfo};

/// Where the suffix's CI mark starts: the text after it is client-written too, so
/// `stage_text_in`'s one ` ci ` is this.
pub(crate) const CI_LEAD: &str = "  ci ";

/// The CI mark `row_suffix` writes and the role it is drawn in (review finding m2):
/// an open PR's only, `None` otherwise.
fn ci_mark(run: &RunInfo, stage: &StageInfo, ascii: bool) -> Option<(&'static str, Role)> {
    run.delivery
        .as_ref()
        .filter(|d| d.mode == DeliveryMode::Pr)?;
    let pr = stage.pr.as_ref().filter(|pr| pr.state == PrState::Open)?;
    Some(ci_look(pr.ci, ascii))
}

/// A `pr`-mode stage row's CI mark in `text` (its content text, as both painters draw
/// it), by byte range, with `theme::ci_look`'s role: the graph and the compact list
/// colour it. A stage row carries no host or agent text, so its one `  ci ` is the
/// suffix's and nothing on the row can forge the mark.
pub(crate) fn ci_mark_span(
    row: &Row<'_>,
    text: &str,
    ascii: bool,
) -> Option<(std::ops::Range<usize>, Role)> {
    let RowKind::Stage { run, stage } = &row.kind else {
        return None;
    };
    let (mark, role) = ci_mark(run, stage, ascii)?;
    let start = text.find(&format!("{CI_LEAD}{mark}"))? + CI_LEAD.len();
    Some((start..start + mark.len(), role))
}

/// The suffix `stage`'s row carries in a `pr`-mode run, empty in a local one:
/// `  #<n>  ci <mark>`, then `  <k> threads` (the new and tasked ones, the threads
/// still to address) and `  paused` while a lower stage's PR is closed; `  #<n>  merged`
/// or `  #<n>  closed` once it landed; `  skipped` for a stage with no changes, and
/// `  no PR yet` before its PR opens.
#[cfg(test)]
pub(crate) fn row_suffix(run: &RunInfo, stage: &StageInfo, ascii: bool) -> String {
    row_suffix_within(run, stage, ascii, usize::MAX)
}

/// [`row_suffix`] in at most `room` display columns where it can: the PR number's
/// digits are cut first (`#1234…`), so the CI mark, the thread count and the state word
/// after them stay whole (the final fix wave, review C M4). Where even a one-column
/// number leaves no room, the thread count goes, then `  paused`, never the CI mark
/// (milestone 9.19 review I1).
pub(crate) fn row_suffix_within(
    run: &RunInfo,
    stage: &StageInfo,
    ascii: bool,
    room: usize,
) -> String {
    let Some(delivery) = run.delivery.as_ref().filter(|d| d.mode == DeliveryMode::Pr) else {
        return String::new();
    };
    let Some(pr) = &stage.pr else {
        return if delivery.skipped_stages.contains(&stage.n) {
            "  skipped"
        } else {
            "  no PR yet"
        }
        .to_owned();
    };
    let mut parts: Vec<String> = Vec::new();
    match pr.state {
        PrState::Merged => parts.push("  merged".to_owned()),
        PrState::Closed => parts.push("  closed".to_owned()),
        PrState::Open => {
            parts.push(format!("{CI_LEAD}{}", ci_look(pr.ci, ascii).0));
            match pr.threads.new.saturating_add(pr.threads.tasked) {
                0 => {}
                1 => parts.push("  1 thread".to_owned()),
                k => parts.push(format!("  {k} threads")),
            }
            if pr.paused {
                parts.push("  paused".to_owned());
            }
        }
    }
    // `  #` and a one-column number, then the parts: drop the thread count, then
    // `  paused`, while they do not fit; the first part (the mark) always stays.
    let width = |parts: &[String]| {
        parts
            .iter()
            .map(|p| UnicodeWidthStr::width(p.as_str()))
            .sum::<usize>()
    };
    for drop in ["thread", "paused"] {
        if 3 + width(&parts) > room
            && let Some(at) = parts.iter().skip(1).position(|p| p.contains(drop))
        {
            parts.remove(at + 1);
        }
    }
    let rest = parts.concat();
    let number = format!("#{}", pr.number);
    let fixed = 2 + UnicodeWidthStr::width(rest.as_str());
    let number = if fixed + UnicodeWidthStr::width(number.as_str()) > room {
        truncate_in(&number, room.saturating_sub(fixed).max(1), ascii)
    } else {
        number
    };
    format!("  {number}{rest}")
}

#[cfg(test)]
#[path = "stage_pr_tests.rs"]
mod tests;
