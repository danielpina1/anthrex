//! The OTLP receiver's token check (milestone 9 decision 14a): only points carrying
//! their run's token are metered; the rest are dropped and counted per run for the log.

use std::collections::{BTreeMap, BTreeSet};

use super::{Shared, UsagePoint};

/// Decision 14a: keeps the points of each live run whose token `presented` carries
/// (`Bearer <token>`), and whether any run's did. The points of a live run without it
/// are dropped and counted; a run that is not live is left to [`record`], which drops it.
pub(super) fn authorize(
    shared: &Shared,
    mut points: Vec<UsagePoint>,
    presented: Option<&str>,
) -> (Vec<UsagePoint>, bool) {
    let sink = &shared.sink;
    let runs: BTreeSet<String> = points.iter().map(|p| p.run_id.clone()).collect();
    let mut allowed = BTreeSet::new();
    let mut refused = BTreeMap::new();
    for run in runs {
        if !sink.is_live(&run) {
            continue;
        }
        let expected = sink.token(&run).map(|token| format!("Bearer {token}"));
        match (expected, presented) {
            (Some(expected), Some(presented)) if same(&expected, presented) => {
                allowed.insert(run);
            }
            _ => {
                refused.insert(run, 0u64);
            }
        }
    }
    points.retain(|p| {
        if let Some(count) = refused.get_mut(&p.run_id) {
            *count += 1;
            return false;
        }
        true
    });
    if !refused.is_empty() {
        let mut held = crate::lock(&shared.ledger);
        held.drops.retain(|run, _| sink.is_live(run));
        for (run, n) in refused {
            let total = held.drops.entry(run.clone()).or_default();
            if *total == 0 {
                tracing::warn!(run = %run, "OTLP points without the run's token were dropped");
            }
            *total += n;
            tracing::debug!(run = %run, dropped = *total, "OTLP points dropped for their token");
        }
    }
    (points, !allowed.is_empty())
}

/// Compares two tokens in time independent of where they differ.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}
