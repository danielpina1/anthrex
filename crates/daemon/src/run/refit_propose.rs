//! Milestone 9.5 decisions 8 and 11: line-threshold proposals, `--apply` and
//! `--dismiss` (milestone 9.8 decision 30: the class-route proposals are gone). Part of `run::refit` (split for the 600-line rule), which
//! re-exports its public items. Pure.

use proto::{HistoryLine, SizeThresholds, TuningChange, TuningFile, TuningProposal};

use super::{SizeClass, moved, percentile, qualifies, threshold_samples};

pub(super) fn thresholds_of(file: &TuningFile) -> SizeThresholds {
    file.thresholds.unwrap_or_default()
}

/// Decision 8, for S or M.
fn threshold_proposal(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
    class: SizeClass,
) -> Option<TuningProposal> {
    let t = &cfg.tuning.table;
    let samples = threshold_samples(lines, class, t);
    if !qualifies(&samples, t) {
        return None;
    }
    let mut changed: Vec<u64> = (samples.iter())
        .filter_map(|r| r.diff.map(|d| u64::from(d.added) + u64::from(d.removed)))
        .collect();
    let p = percentile(&mut changed, t.threshold_percentile)?;
    let current = thresholds_of(file);
    let (step, cur) = match class {
        SizeClass::S => (5, current.s_lines),
        SizeClass::M => (10, current.m_lines),
        SizeClass::Hub | SizeClass::Brainstorm | SizeClass::DocReview => return None,
    };
    let lines_new = u32::try_from(p.div_ceil(step).max(1) * step).unwrap_or(u32::MAX);
    if !moved(lines_new.into(), cur.into(), t.min_change_percent)
        || (class == SizeClass::M && lines_new <= current.s_lines)
    {
        return None;
    }
    let c = class.label();
    Some(TuningProposal {
        id: format!("thresholds.{}", class.key()),
        text: format!(
            "{c} line threshold {cur} → {lines_new} (p{} of {} merged {c} tasks)",
            t.threshold_percentile,
            samples.len()
        ),
        current: cur.to_string(),
        proposed: lines_new.to_string(),
        change: TuningChange::Threshold {
            class: class.key().to_string(),
            lines: lines_new,
        },
    })
}

/// The current proposals, in order `thresholds.s`, `thresholds.m`; one dismissed at
/// its proposed value is left out.
pub fn proposals(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
) -> Vec<TuningProposal> {
    let classes = [SizeClass::S, SizeClass::M];
    let thresholds = classes.map(|c| threshold_proposal(lines, file, cfg, c));
    (thresholds.into_iter())
        .flatten()
        .filter(|p| file.dismissed.get(&p.id) != Some(&p.proposed))
        .collect()
}

/// The proposals `ids` names, each current, or the refusal for the first that is not.
fn named<'a>(
    current: &'a [TuningProposal],
    ids: &[String],
) -> Result<Vec<&'a TuningProposal>, String> {
    (ids.iter())
        .map(|id| {
            (current.iter()).find(|p| &p.id == id).ok_or_else(|| {
                format!("no current proposal {id}; run anthrex run stats to see the proposals")
            })
        })
        .collect()
}

/// `--apply`: each named current proposal's change, written into `[thresholds]`. An unknown id refuses the whole request.
pub fn apply(
    file: &TuningFile,
    current: &[TuningProposal],
    ids: &[String],
) -> Result<TuningFile, String> {
    let mut out = file.clone();
    for p in named(current, ids)? {
        match &p.change {
            TuningChange::Threshold { class, lines } => {
                let mut t = thresholds_of(&out);
                match class.as_str() {
                    "s" => t.s_lines = *lines,
                    _ => t.m_lines = *lines,
                }
                out.thresholds = Some(t);
            }
        }
    }
    Ok(out)
}

/// `--dismiss`: `dismissed[id] = <proposed value>` for each named current proposal. An
/// unknown id refuses the whole request.
pub fn dismiss(
    file: &TuningFile,
    current: &[TuningProposal],
    ids: &[String],
) -> Result<TuningFile, String> {
    let mut out = file.clone();
    for p in named(current, ids)? {
        out.dismissed.insert(p.id.clone(), p.proposed.clone());
    }
    Ok(out)
}
