//! Milestone 9.5 decision 7: the critical-path weights history supports. Part of
//! `run::refit` (split for the 600-line rule, milestone 9.6 task M9.6.13), which uses
//! its items. Pure.

use config::Tuning;
use proto::{HistoryLine, PathWeights};

use super::{SizeClass, active_secs, budget_samples, median_of, moved, qualifies};

pub(super) fn fit_weights(lines: &[HistoryLine], t: &Tuning, now: u64) -> Option<PathWeights> {
    let measured = |class| {
        let samples = budget_samples(lines, class, t);
        qualifies(&samples, t)
            .then(|| median_of(&samples, |r| active_secs(&r.phases)))
            .flatten()
    };
    let (s, m, hub) = (
        measured(SizeClass::S),
        measured(SizeClass::M),
        measured(SizeClass::Hub),
    );
    let m_secs = m.or(hub).or(s.map(|s| s.saturating_mul(3)))?;
    let hub_secs = hub.unwrap_or(m_secs);
    let s_secs = s.unwrap_or(m_secs.div_ceil(3));
    let derived = [(s, "S"), (m, "M"), (hub, "hub")]
        .into_iter()
        .filter(|(secs, _)| secs.is_none())
        .map(|(_, label)| label.to_string())
        .collect();
    Some(PathWeights {
        s_secs,
        m_secs,
        hub_secs,
        derived,
        at: now,
    })
}

/// A weight moved by `pct`, or which classes are derived changed.
pub(super) fn weights_moved(new: &PathWeights, cur: &PathWeights, pct: u32) -> bool {
    new.derived != cur.derived
        || moved(new.s_secs, cur.s_secs, pct)
        || moved(new.m_secs, cur.m_secs, pct)
        || moved(new.hub_secs, cur.hub_secs, pct)
}

pub(super) fn weights_text(w: &PathWeights) -> String {
    let part = |label: &str, secs: u64| {
        let derived = w.derived.iter().any(|d| d == label);
        format!("{label} {secs}s{}", if derived { " (derived)" } else { "" })
    };
    format!(
        "{}, {}, {}",
        part("S", w.s_secs),
        part("M", w.m_secs),
        part("hub", w.hub_secs)
    )
}
