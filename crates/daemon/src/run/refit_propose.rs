//! Milestone 9.5 decisions 8, 9 and 11: line-threshold and class-route proposals,
//! `--apply` and `--dismiss`. Part of `run::refit` (split for the 600-line rule), which
//! re-exports its public items. Pure.

use config::{RouteList, RouteLists};
use proto::{
    ClassRoute, Effort, HistoryLine, SizeThresholds, TuningChange, TuningFile, TuningProposal,
};

use super::super::model::ClassRoutes;
use super::super::validate::strength_label;
use super::{
    M_ROUTE_LADDER, Quality, S_ROUTE_LADDER, SizeClass, moved, percentile, qualifies,
    route_samples, threshold_samples,
};

pub(super) fn effort_label(e: Effort) -> &'static str {
    match e {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

pub(super) fn route_text(r: ClassRoute) -> String {
    format!("{}/{}", strength_label(r.strength), effort_label(r.effort))
}

pub(super) fn list_of(lists: &RouteLists, class: SizeClass) -> &RouteList {
    match class {
        SizeClass::S => &lists.s,
        SizeClass::M => &lists.m,
        SizeClass::Hub => &lists.hub,
        SizeClass::Brainstorm => &lists.brainstorm,
        // The document reviewer has no list (decision 10: the orchestrator's peer).
        SizeClass::DocReview => &NO_LIST,
    }
}

static NO_LIST: RouteList = RouteList {
    candidates: Vec::new(),
    pick: config::Pick::First,
};

/// `list (config): <runtime>/<model> <effort>, …`, a candidate without `effort` taking
/// the class's.
pub(super) fn list_text(list: &RouteList, default: Effort) -> String {
    let candidates: Vec<String> = (list.candidates.iter())
        .map(|c| {
            let effort = effort_label(c.effort.unwrap_or(default));
            format!("{}/{} {effort}", c.runtime.label(), c.model)
        })
        .collect();
    format!("list (config): {}", candidates.join(", "))
}

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

/// The class's route without a list: an applied `route.<class>`, else M8a's default.
pub(super) fn current_route(file: &TuningFile, class: SizeClass) -> ClassRoute {
    let defaults = ClassRoutes::default();
    match class {
        SizeClass::S => file.routes.get("s").copied().unwrap_or(defaults.s),
        SizeClass::M => file.routes.get("m").copied().unwrap_or(defaults.m),
        SizeClass::Hub => defaults.hub,
        // Never asked: a design agent's route is decision 10's, not a class route.
        SizeClass::Brainstorm | SizeClass::DocReview => defaults.m,
    }
}

/// Decision 9, for S or M with no model list.
fn route_proposal(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
    class: SizeClass,
    quality: &Quality,
) -> Option<TuningProposal> {
    let t = &cfg.tuning.table;
    let ladder = match class {
        SizeClass::S => &S_ROUTE_LADDER,
        SizeClass::M => &M_ROUTE_LADDER,
        SizeClass::Hub | SizeClass::Brainstorm | SizeClass::DocReview => return None,
    };
    if !list_of(&cfg.tuning.routes, class).candidates.is_empty() {
        return None;
    }
    let cur = current_route(file, class);
    let samples = route_samples(lines, class, t, cur);
    if !qualifies(&samples, t) {
        return None;
    }
    let at = ladder.iter().position(|r| *r == cur)?;
    let n = samples.len() as u64;
    let escalated = samples.iter().filter(|r| r.max_rung >= 2).count() as u64;
    let c = class.label();
    let (to, why) = if escalated * 100 > n * u64::from(t.escalate_above_percent) {
        let pct = escalated * 100 / n;
        let why = format!("{escalated} of {n} {c} tasks, {pct}%, reached rung 2 or higher");
        (ladder.get(at + 1)?, why)
    } else if escalated == 0 && !samples.iter().any(|r| quality.fails(r)) {
        let why = format!("none of {n} {c} tasks reached rung 2 or higher or failed on quality");
        (ladder.get(at.checked_sub(1)?)?, why)
    } else {
        return None;
    };
    Some(TuningProposal {
        id: format!("route.{}", class.key()),
        text: format!(
            "{c} route {} → {} ({why})",
            route_text(cur),
            route_text(*to)
        ),
        current: route_text(cur),
        proposed: route_text(*to),
        change: TuningChange::Route {
            class: class.key().to_string(),
            route: *to,
        },
    })
}

/// The current proposals, in order `thresholds.s`, `thresholds.m`, `route.s`,
/// `route.m`; one dismissed at its proposed value is left out.
pub fn proposals(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
) -> Vec<TuningProposal> {
    let classes = [SizeClass::S, SizeClass::M];
    let thresholds = classes.map(|c| threshold_proposal(lines, file, cfg, c));
    let quality = Quality::of(lines);
    let routes = classes.map(|c| route_proposal(lines, file, cfg, c, &quality));
    (thresholds.into_iter().chain(routes))
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

/// `--apply`: each named current proposal's change, written into `[thresholds]` or
/// `[routes.<class>]`. An unknown id refuses the whole request.
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
            TuningChange::Route { class, route } => {
                out.routes.insert(class.clone(), *route);
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
