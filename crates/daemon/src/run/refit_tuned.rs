//! Milestone 9.5 decisions 6, 11 and 12: what a run freezes at start ([`Tuned`]) and
//! the report `run stats` renders. Part of `run::refit` (split for the 600-line rule),
//! which re-exports its public items. Pure.

use std::path::Path;

use config::{ConfiguredBudgets, RouteLists, Tuning};
use proto::{
    Budget, ClassBudget, ClassTuning, HistoryLine, PathWeights, RefitState, SizeThresholds,
    TuningFile, TuningReport,
};

use super::super::model::ClassRoutes;
use super::propose::{current_route, effort_label, list_of, list_text, route_text, thresholds_of};
use super::{
    SizeClass, as_budget, budget_samples, budget_text, default_budget, proposals, qualifies,
    weights_text,
};

/// What a run freezes at start (decision 12); `Tuned::default()` is today exactly.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tuned {
    /// A class's refit budget, `Some` only when it is used: the refit is on, the file
    /// has one, and config does not set the class explicitly (ruling RH-5; an explicit
    /// M also holds hub).
    pub budget_s: Option<Budget>,
    pub budget_m: Option<Budget>,
    pub budget_hub: Option<Budget>,
    pub configured: ConfiguredBudgets,
    pub weights: Option<PathWeights>,
    pub thresholds: SizeThresholds,
    pub routes: ClassRoutes,
    pub lists: RouteLists,
    /// Decision 12's start lines.
    pub log: Vec<String>,
}

impl Tuned {
    /// The budget a task of `class` gets with no `[task.budget]` (decision 6): the
    /// refit when used, else the configured value; hub, its refit else M's.
    pub fn effective(&self, cfg: &config::Orchestrator, class: SizeClass) -> Budget {
        let m = self.budget_m.unwrap_or(cfg.budget_m);
        match class {
            SizeClass::S => self.budget_s.unwrap_or(cfg.budget_s),
            SizeClass::M => m,
            SizeClass::Hub => self.budget_hub.unwrap_or(m),
        }
    }
}

fn configured(c: ConfiguredBudgets, class: SizeClass) -> bool {
    match class {
        SizeClass::S => c.s,
        SizeClass::M | SizeClass::Hub => c.m,
    }
}

/// The class's refit in the file, while the refit is on.
fn written<'a>(file: &'a TuningFile, t: &Tuning, class: SizeClass) -> Option<&'a ClassBudget> {
    file.budgets.get(class.key()).filter(|_| t.refit_budgets)
}

/// Decision 12: the file and config a run starts with, and its start lines.
pub fn tuned(file: &TuningFile, cfg: &config::Orchestrator) -> Tuned {
    let t = &cfg.tuning.table;
    let conf = cfg.tuning.configured;
    let mut log = Vec::new();
    let mut used = |class: SizeClass| {
        let b = written(file, t, class)?;
        let refit = as_budget(b, default_budget(cfg, class));
        let c = class.label();
        if configured(conf, class) {
            let own = budget_text(&default_budget(cfg, class));
            let would = budget_text(&refit);
            log.push(format!(
                "tuning: budget {c} {own} configured (refit would be {would})"
            ));
            return None;
        }
        let samples = b.samples;
        log.push(format!(
            "tuning: budget {c} {} from {samples} samples",
            budget_text(&refit)
        ));
        Some(refit)
    };
    let (budget_s, budget_m, budget_hub) =
        (used(SizeClass::S), used(SizeClass::M), used(SizeClass::Hub));
    let weights = file.weights.clone().filter(|_| t.path_weights);
    if let Some(w) = &weights {
        log.push(format!("tuning: path weights {}", weights_text(w)));
    }
    if let Some(th) = file.thresholds {
        log.push(format!(
            "tuning: thresholds S {}, M {} lines",
            th.s_lines, th.m_lines
        ));
    }
    let routes = ClassRoutes {
        s: current_route(file, SizeClass::S),
        m: current_route(file, SizeClass::M),
        ..ClassRoutes::default()
    };
    for class in [SizeClass::S, SizeClass::M] {
        if let Some(r) = file.routes.get(class.key()) {
            log.push(format!(
                "tuning: route {} {} (applied)",
                class.label(),
                route_text(*r)
            ));
        }
    }
    if log.is_empty() {
        log.push(format!(
            "tuning: none (history has fewer than {} samples per class)",
            t.min_samples
        ));
    }
    Tuned {
        budget_s,
        budget_m,
        budget_hub,
        configured: conf,
        weights,
        thresholds: thresholds_of(file),
        routes,
        lists: cfg.tuning.routes.clone(),
        log,
    }
}

/// Decision 11's report over `file` as it is (the caller refits and saves first);
/// `applied`, `dismissed` and `moved_bad_file` are the caller's to fill.
pub fn report(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
    path: &Path,
) -> TuningReport {
    let t = &cfg.tuning.table;
    let tuned = tuned(file, cfg);
    let weights = tuned.weights.as_ref();
    let classes = SizeClass::ALL
        .into_iter()
        .map(|class| {
            let samples = budget_samples(lines, class, t);
            let is_configured = configured(cfg.tuning.configured, class);
            let written = written(file, t, class);
            let refit = match (t.refit_budgets, is_configured, written) {
                (false, _, _) => RefitState::Off,
                (true, true, _) => RefitState::Configured,
                (true, false, Some(b)) => RefitState::Written { at: b.at },
                (true, false, None) if qualifies(&samples, t) => RefitState::Kept,
                (true, false, None) => RefitState::NotYet,
            };
            let weight_secs = weights.map(|w| match class {
                SizeClass::S => w.s_secs,
                SizeClass::M => w.m_secs,
                SizeClass::Hub => w.hub_secs,
            });
            let weight_derived =
                weights.is_some_and(|w| w.derived.iter().any(|d| d == class.label()));
            let list = list_of(&cfg.tuning.routes, class);
            let route = current_route(file, class);
            ClassTuning {
                class: class.label().to_string(),
                samples: samples.len() as u32,
                budget: tuned.effective(cfg, class),
                refit,
                configured: is_configured,
                refit_budget: written.map(|b| as_budget(b, default_budget(cfg, class))),
                weight_secs,
                weight_derived,
                route: if list.candidates.is_empty() {
                    route_text(route)
                } else {
                    list_text(list, route.effort)
                },
            }
        })
        .collect();
    let orchestrator_list = (cfg.tuning.routes.orchestrator.candidates.first()).map(|c| {
        let effort = c.effort.map(|e| format!(" {}", effort_label(e)));
        format!(
            "{}/{}{}",
            c.runtime.label(),
            c.model,
            effort.unwrap_or_default()
        )
    });
    TuningReport {
        path: path.to_path_buf(),
        min_samples: t.min_samples,
        refit_budgets: t.refit_budgets,
        classes,
        proposals: proposals(lines, file, cfg),
        moved_bad_file: None,
        applied: Vec::new(),
        dismissed: Vec::new(),
        orchestrator_list,
    }
}
