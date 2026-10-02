//! Milestone 9.2 decision 42: the pull-request part of a stage node's content row,
//! after its tier 3 (`stage 2/3  tier 3 ✓  #142  ci ✓  2 threads`). Pure, and nothing
//! in it is agent or host text: a number, the CI mark (`theme::ci_look`, with its ASCII
//! twin) and counts.

use crate::theme::ci_look;
use proto::{DeliveryMode, PrState, RunInfo, StageInfo};

/// The suffix `stage`'s row carries in a `pr`-mode run, empty in a local one:
/// `  #<n>  ci <mark>`, then `  <k> threads` (the new and tasked ones, the threads
/// still to address) and `  paused` while a lower stage's PR is closed; `  #<n>  merged`
/// or `  #<n>  closed` once it landed; `  skipped` for a stage with no changes, and
/// `  no PR yet` before its PR opens.
pub(crate) fn row_suffix(run: &RunInfo, stage: &StageInfo, ascii: bool) -> String {
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
    let mut out = format!("  #{}", pr.number);
    match pr.state {
        PrState::Merged => out.push_str("  merged"),
        PrState::Closed => out.push_str("  closed"),
        PrState::Open => {
            out.push_str(&format!("  ci {}", ci_look(pr.ci, ascii).0));
            match pr.threads.new.saturating_add(pr.threads.tasked) {
                0 => {}
                1 => out.push_str("  1 thread"),
                k => out.push_str(&format!("  {k} threads")),
            }
            if pr.paused {
                out.push_str("  paused");
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "stage_pr_tests.rs"]
mod tests;
