//! The run's tier-3 attention lines (decisions 19 and 38, ruling C-18). A child of
//! `full.rs`, split out to keep that file under 600 lines (milestone 9.2's budget).
//! Pure.

use super::{ATTENTION_TESTS, first, infra_at_head, infra_held, lacks_green, red_at_head};
use crate::run::model::Run;

/// The run's tier-3 attention lines: while it runs, each stage red on its head with its
/// note (decision 38's line); once it completed red (`final_check_failed`), decision
/// 19's `tier 3 red on stage <n>: <tests>` in place of M8a's final-check line.
pub(crate) fn attention(run: &Run) -> Vec<String> {
    let mut lines = red_lines(run);
    if run.final_check_failed || !run.state.is_terminal() {
        // Ruling C-18: a stage held after the executor's failures.
        for s in run
            .stages
            .iter()
            .filter(|s| lacks_green(run, s) && infra_held(s))
        {
            if let Some(i) = infra_at_head(s) {
                lines.push(format!(
                    "stage {}: could not run tier 3 ({}); anthrex run resume retries",
                    s.n, i.line
                ));
            }
        }
    }
    lines
}

fn red_lines(run: &Run) -> Vec<String> {
    let red = run
        .stages
        .iter()
        .filter(|s| red_at_head(s) && lacks_green(run, s));
    if run.final_check_failed {
        return red
            .map(|s| {
                // Ruling C-18: only a record of the red commit itself names tests.
                let failing = s
                    .full
                    .last
                    .as_ref()
                    .filter(|t| !t.ok && s.full.red_at.as_deref() == Some(t.commit.as_str()))
                    .map(|t| t.failing.clone())
                    .unwrap_or_default();
                match (&s.full.note, failing.is_empty()) {
                    (Some(note), true) => note.clone(),
                    _ => format!(
                        "tier 3 red on stage {}: {}",
                        s.n,
                        first(&failing, ATTENTION_TESTS)
                    ),
                }
            })
            .collect();
    }
    if run.state.is_terminal() {
        return Vec::new();
    }
    red.filter_map(|s| s.full.note.clone()).collect()
}
