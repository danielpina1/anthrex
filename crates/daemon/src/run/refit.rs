//! Milestone 9.5 decisions 4–9 and 11 (rulings RH-1 to RH-5, RH-7): what a repository's
//! history teaches. Budgets and critical-path weights are refitted into `tuning.toml`
//! automatically; line thresholds and class routes are only proposed, applied by
//! `anthrex run stats --apply <id>` and silenced by `--dismiss <id>`. [`tuned`] is what a
//! run freezes at start (decision 12).
//!
//! Pure (decision 1): no I/O, no clock (`now` is passed in), and integers only
//! (decision 5). `driver/tuning.rs` reads and writes the file.

use std::collections::{HashMap, HashSet};

use config::{ConfiguredBudgets, Tuning};
use proto::{
    AgentRole, Budget, ClassBudget, ClassRoute, Effort, HistoryLine, PathWeights, PhaseSecs, Size,
    Strength, TaskKind, TaskOrigin, TaskOutcome, TaskPattern, TaskRecord, TokenUsage, TuningFile,
};

use super::history_io::effective_reverts;
use super::routing::CLASS_DEFAULT;

/// A task class the refit learns per (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SizeClass {
    S,
    M,
    Hub,
}

impl SizeClass {
    pub const ALL: [SizeClass; 3] = [SizeClass::S, SizeClass::M, SizeClass::Hub];

    /// As `run stats` prints it: `S`, `M`, `hub`.
    pub fn label(self) -> &'static str {
        match self {
            SizeClass::S => "S",
            SizeClass::M => "M",
            SizeClass::Hub => "hub",
        }
    }

    /// As `tuning.toml` keys it: `s`, `m`, `hub`.
    pub fn key(self) -> &'static str {
        match self {
            SizeClass::S => "s",
            SizeClass::M => "m",
            SizeClass::Hub => "hub",
        }
    }
}

/// The class a task record belongs to: `hub` whatever its size, else its final size. A
/// non-hub `L` task (blocked for a split, never merged) belongs to none. M8b's
/// `stats::class`, moved here unchanged.
pub fn class_of(record: &TaskRecord) -> Option<SizeClass> {
    match (record.hub, record.final_size) {
        (true, _) => Some(SizeClass::Hub),
        (false, Size::S) => Some(SizeClass::S),
        (false, Size::M) => Some(SizeClass::M),
        (false, Size::L) => None,
    }
}

const fn class_route(strength: Strength, effort: Effort) -> ClassRoute {
    ClassRoute { strength, effort }
}

/// Decision 9's ladder for S, bottom first; M8a's default is `standard/low`.
pub const S_ROUTE_LADDER: [ClassRoute; 4] = [
    class_route(Strength::Fast, Effort::Low),
    class_route(Strength::Fast, Effort::Medium),
    class_route(Strength::Standard, Effort::Low),
    class_route(Strength::Standard, Effort::Medium),
];

/// Decision 9's ladder for M, bottom first; M8a's default is `standard/medium`.
pub const M_ROUTE_LADDER: [ClassRoute; 4] = [
    class_route(Strength::Standard, Effort::Medium),
    class_route(Strength::Standard, Effort::High),
    class_route(Strength::Frontier, Effort::Medium),
    class_route(Strength::Frontier, Effort::High),
];

// ---- samples (decision 4) ----

/// Ruling RH-1: plan code tasks only, and never a race (two lanes' spend).
fn plan_code(record: &TaskRecord, class: SizeClass) -> bool {
    record.origin == TaskOrigin::Plan
        && record.kind == TaskKind::Code
        && record.pattern != Some(TaskPattern::Race)
        && class_of(record) == Some(class)
}

fn task_records(lines: &[HistoryLine]) -> impl Iterator<Item = &TaskRecord> {
    lines.iter().filter_map(|line| match line {
        HistoryLine::Task(t) => Some(t),
        _ => None,
    })
}

/// Decision 33's reverts in effect, by task and by run (`task_id: None`).
struct Reverted<'a> {
    tasks: HashSet<(&'a str, &'a str)>,
    runs: HashSet<&'a str>,
}

impl<'a> Reverted<'a> {
    fn of(lines: &'a [HistoryLine]) -> Self {
        let mut reverted = Reverted {
            tasks: HashSet::new(),
            runs: HashSet::new(),
        };
        for r in effective_reverts(lines) {
            match &r.task_id {
                Some(task) => reverted.tasks.insert((r.run_id.as_str(), task.as_str())),
                None => reverted.runs.insert(r.run_id.as_str()),
            };
        }
        reverted
    }

    fn names(&self, record: &TaskRecord) -> bool {
        self.runs.contains(record.run_id.as_str())
            || (self.tasks).contains(&(record.run_id.as_str(), record.task_id.as_str()))
    }
}

/// Steps 2 and 3: sorted by `(at, record_id)`, at most `per_run_cap` per `(run, round)`
/// (round 0, a line written before 9.5, reads as 1), the newest; then the newest
/// `window`. Oldest first.
fn cap_and_window<'a>(mut records: Vec<&'a TaskRecord>, t: &Tuning) -> Vec<&'a TaskRecord> {
    records.sort_by(|a, b| (a.at, &a.record_id).cmp(&(b.at, &b.record_id)));
    let mut per_round: HashMap<(&str, u32), u32> = HashMap::new();
    let mut kept: Vec<&TaskRecord> = Vec::new();
    for record in records.into_iter().rev() {
        let n = per_round
            .entry((record.run_id.as_str(), record.round.max(1)))
            .or_default();
        if *n < t.per_run_cap {
            *n += 1;
            kept.push(record);
        }
    }
    kept.truncate(t.window as usize);
    kept.reverse();
    kept
}

/// Budget samples: merged with approval and not reverted (decision 33's reverts in
/// effect).
pub fn budget_samples<'a>(
    lines: &'a [HistoryLine],
    class: SizeClass,
    t: &Tuning,
) -> Vec<&'a TaskRecord> {
    let reverted = Reverted::of(lines);
    let records = task_records(lines)
        .filter(|r| plan_code(r, class) && r.outcome == TaskOutcome::Merged)
        .filter(|r| !reverted.names(r))
        .collect();
    cap_and_window(records, t)
}

/// Threshold samples: budget samples with a measured diff.
pub fn threshold_samples<'a>(
    lines: &'a [HistoryLine],
    class: SizeClass,
    t: &Tuning,
) -> Vec<&'a TaskRecord> {
    let reverted = Reverted::of(lines);
    let records = task_records(lines)
        .filter(|r| plan_code(r, class) && r.outcome == TaskOutcome::Merged)
        .filter(|r| !reverted.names(r) && r.diff.is_some())
        .collect();
    cap_and_window(records, t)
}

/// Route samples at the class's current route `cur`: every task that ran a session,
/// whatever its outcome (the tasks that escalated and never merged are what a routing
/// rule needs), whose first worker took the class default at `cur`'s strength and
/// effort. Whole-branch review B, I2: a sample on another route, or on the planner's
/// explicit one, is no evidence about `cur`, so an applied step is not climbed again on
/// the history that proposed it.
pub fn route_samples<'a>(
    lines: &'a [HistoryLine],
    class: SizeClass,
    t: &Tuning,
    cur: ClassRoute,
) -> Vec<&'a TaskRecord> {
    let records = task_records(lines)
        .filter(|r| plan_code(r, class) && r.sessions >= 1 && ran_on(r, cur))
        .collect();
    cap_and_window(records, t)
}

/// Whether `record`'s first worker routing decision took the class default at `cur`.
fn ran_on(record: &TaskRecord, cur: ClassRoute) -> bool {
    (record.routing_decisions.iter())
        .filter(|d| d.role == AgentRole::Worker)
        .min_by_key(|d| d.seq)
        .is_some_and(|d| {
            d.source == CLASS_DEFAULT
                && d.chosen.strength == cur.strength
                && d.chosen.effort == cur.effort
        })
}

/// Ruling RH-2's quality evidence for `record`: a revert in effect names it (or its
/// run's accept); a bisect of its run names it as the culprit; or a CI or review fix
/// task of its run and stage exists (such a fix task names a stage, never a task, so it
/// counts against every plan task of that stage, the conservative reading).
pub fn failed_on_quality(record: &TaskRecord, lines: &[HistoryLine]) -> bool {
    Quality::of(lines).fails(record)
}

/// [`failed_on_quality`]'s evidence, read once per pass over the history.
pub(super) struct Quality<'a> {
    reverted: Reverted<'a>,
    /// `(run, task)` a bisect named as its culprit.
    culprits: HashSet<(&'a str, &'a str)>,
    /// `(run, stage)` with a CI or review fix task.
    fixed_stages: HashSet<(&'a str, u16)>,
}

impl<'a> Quality<'a> {
    pub(super) fn of(lines: &'a [HistoryLine]) -> Self {
        let mut q = Quality {
            reverted: Reverted::of(lines),
            culprits: HashSet::new(),
            fixed_stages: HashSet::new(),
        };
        for line in lines {
            match line {
                HistoryLine::Bisect(b) => {
                    if let Some(culprit) = &b.culprit {
                        q.culprits.insert((b.run_id.as_str(), culprit.as_str()));
                    }
                }
                HistoryLine::Task(t) if matches!(t.origin, TaskOrigin::Ci | TaskOrigin::Review) => {
                    q.fixed_stages.insert((t.run_id.as_str(), t.stage));
                }
                _ => {}
            }
        }
        q
    }

    pub(super) fn fails(&self, record: &TaskRecord) -> bool {
        let run = record.run_id.as_str();
        self.reverted.names(record)
            || self.culprits.contains(&(run, record.task_id.as_str()))
            || self.fixed_stages.contains(&(run, record.stage))
    }
}

// ---- statistics (decision 5) ----

/// The value at index `(n - 1) / 2` of the sorted values; `None` for none.
/// M8b's `stats::median`, reused (decision 4).
pub fn lower_median(values: &[u64]) -> Option<u64> {
    super::stats::median(values.to_vec())
}

/// Nearest rank: the value at index `ceil(p × n / 100) - 1` of the sorted values;
/// `None` for none.
pub fn percentile(values: &mut [u64], p: u32) -> Option<u64> {
    values.sort_unstable();
    let n = values.len() as u64;
    let rank = (u64::from(p) * n).div_ceil(100).clamp(1, n.max(1));
    values.get(usize::try_from(rank - 1).ok()?).copied()
}

/// The phases a task worked in (decision 5): not queued, not blocked.
pub fn active_secs(phases: &PhaseSecs) -> u64 {
    [
        phases.preparing,
        phases.working,
        phases.proof,
        phases.check,
        phases.review,
        phases.merge,
    ]
    .iter()
    .fold(0, |sum, &s| sum.saturating_add(s))
}

fn median_of(samples: &[&TaskRecord], value: impl Fn(&TaskRecord) -> u64) -> Option<u64> {
    lower_median(&samples.iter().map(|r| value(r)).collect::<Vec<_>>())
}

fn qualifies(samples: &[&TaskRecord], t: &Tuning) -> bool {
    samples.len() as u64 >= u64::from(t.min_samples)
}

/// `new != cur` and `|new − cur| × 100 >= pct × cur` (0 → 0 is no change).
fn moved(new: u64, cur: u64, pct: u32) -> bool {
    new != cur && new.abs_diff(cur).saturating_mul(100) >= u64::from(pct).saturating_mul(cur)
}

// ---- budgets (decision 6) ----

/// The refit of a qualifying class's budget samples.
fn fit_budget(samples: &[&TaskRecord], t: &Tuning, now: u64) -> Option<ClassBudget> {
    if !qualifies(samples, t) {
        return None;
    }
    let f = u64::from(t.budget_factor_percent);
    let calls = median_of(samples, |r| u64::from(r.tool_calls))?;
    let working = median_of(samples, |r| r.phases.working)?;
    // Ruling T8-5: only samples that recorded worker usage, and only when they qualify
    // on their own and their lower median is above 0; else no token refit (the
    // configured token budget, or none, stays).
    let recorded: Vec<&TaskRecord> = (samples.iter().copied())
        .filter(|r| r.worker_usage != TokenUsage::default())
        .collect();
    let tokens = (t.refit_tokens && qualifies(&recorded, t))
        .then(|| median_of(&recorded, |r| r.worker_usage.billable()))
        .flatten()
        .filter(|&m| m > 0)
        .map(|m| m.saturating_mul(f).div_ceil(100));
    let clamp = |v: u64, lo: u64, hi: u64| v.clamp(lo, hi) as u32;
    Some(ClassBudget {
        tool_calls: clamp(calls.saturating_mul(f).div_ceil(100), 10, 2000),
        minutes: clamp(working.saturating_mul(f).div_ceil(6000), 5, 1440),
        tokens,
        samples: samples.len() as u32,
        at: now,
    })
}

/// The class's default with no refit: S's or M's configured budget; hub takes M's.
fn default_budget(cfg: &config::Orchestrator, class: SizeClass) -> Budget {
    match class {
        SizeClass::S => cfg.budget_s,
        SizeClass::M | SizeClass::Hub => cfg.budget_m,
    }
}

fn as_budget(b: &ClassBudget, default: Budget) -> Budget {
    Budget {
        tool_calls: b.tool_calls,
        minutes: b.minutes,
        tokens: b.tokens.or(default.tokens),
    }
}

/// Whether `new` moved from `cur` by `min_change_percent` on either axis.
fn budget_moved(new: &Budget, cur: &Budget, pct: u32) -> bool {
    let tokens = match (new.tokens, cur.tokens) {
        (Some(n), Some(c)) => moved(n, c, pct),
        (n, c) => n != c,
    };
    moved(new.tool_calls.into(), cur.tool_calls.into(), pct)
        || moved(new.minutes.into(), cur.minutes.into(), pct)
        || tokens
}

/// Whether config sets the class's budget explicitly (ruling RH-5); explicit M also
/// holds hub.
fn configured(c: ConfiguredBudgets, class: SizeClass) -> bool {
    match class {
        SizeClass::S => c.s,
        SizeClass::M | SizeClass::Hub => c.m,
    }
}

/// `<calls> calls <minutes>m`, then ` <tokens> tok` (M8b's `k`/`M` notation) when set.
pub fn budget_text(b: &Budget) -> String {
    let mut text = format!("{} calls {}m", b.tool_calls, b.minutes);
    if let Some(tokens) = b.tokens {
        text.push_str(&format!(" {} tok", super::stats::tokens(tokens)));
    }
    text
}

/// Rung 4's ceiling (ruling RH-4), per axis: for S, the effective M budget; for M or
/// hub, the larger of L's budget and twice the effective M budget. On the token axis
/// (ruling T9-1), M without a token budget leaves L's, and L without one leaves none.
///
/// Ruling T8-6: never below `own`, the class's own effective budget, on any axis (a
/// refit S budget above the effective M, or a hub refit above the M or hub rule). A
/// token axis with no ceiling stays without one; `own` without tokens adds none.
pub fn ceiling(class: SizeClass, own: Budget, effective_m: Budget, budget_l: Budget) -> Budget {
    let rule = if class == SizeClass::S {
        effective_m
    } else {
        Budget {
            tool_calls: (budget_l.tool_calls).max(effective_m.tool_calls.saturating_mul(2)),
            minutes: budget_l.minutes.max(effective_m.minutes.saturating_mul(2)),
            tokens: (budget_l.tokens)
                .map(|l| effective_m.tokens.map_or(l, |m| l.max(m.saturating_mul(2)))),
        }
    };
    Budget {
        tool_calls: rule.tool_calls.max(own.tool_calls),
        minutes: rule.minutes.max(own.minutes),
        tokens: rule.tokens.map(|r| own.tokens.map_or(r, |o| r.max(o))),
    }
}

// ---- critical-path weights (decision 7) ----

fn fit_weights(lines: &[HistoryLine], t: &Tuning, now: u64) -> Option<PathWeights> {
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
fn weights_moved(new: &PathWeights, cur: &PathWeights, pct: u32) -> bool {
    new.derived != cur.derived
        || moved(new.s_secs, cur.s_secs, pct)
        || moved(new.m_secs, cur.m_secs, pct)
        || moved(new.hub_secs, cur.hub_secs, pct)
}

fn weights_text(w: &PathWeights) -> String {
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

/// Ruling T8-7: a configured class's refit, computed from history for display only
/// ("refit would be"), with no change gate; `None` with the refit off or too few
/// samples.
pub fn shown_refit(
    lines: &[HistoryLine],
    cfg: &config::Orchestrator,
    class: SizeClass,
) -> Option<Budget> {
    let t = &cfg.tuning.table;
    if !t.refit_budgets {
        return None;
    }
    let fit = fit_budget(&budget_samples(lines, class, t), t, 0)?;
    Some(as_budget(&fit, default_budget(cfg, class)))
}

/// Decisions 6 and 7: the budgets and weights history supports, written into `file`
/// only where they moved by `min_change_percent` (or were absent), with one refit-write
/// line each (decision 12). A configured class (ruling RH-5) is not refitted here: its
/// `[budgets.<class>]` is left as it is (ruling T8-7), and [`shown_refit`] computes
/// what it would be, for display only.
pub fn refit(
    lines: &[HistoryLine],
    file: &TuningFile,
    cfg: &config::Orchestrator,
    now: u64,
) -> (TuningFile, Vec<String>) {
    let t = &cfg.tuning.table;
    let mut out = file.clone();
    let mut log = Vec::new();
    // Ruling T8-7: a configured class's refit is only shown, never written.
    let refitted = |c: &SizeClass| t.refit_budgets && !configured(cfg.tuning.configured, *c);
    for class in SizeClass::ALL.into_iter().filter(refitted) {
        let Some(new) = fit_budget(&budget_samples(lines, class, t), t, now) else {
            continue;
        };
        let default = default_budget(cfg, class);
        let cur = (file.budgets.get(class.key())).map_or(default, |b| as_budget(b, default));
        let new_budget = as_budget(&new, default);
        if budget_moved(&new_budget, &cur, t.min_change_percent) {
            log.push(format!(
                "tuning: budget {} {} → {} from {} samples",
                class.label(),
                budget_text(&cur),
                budget_text(&new_budget),
                new.samples
            ));
            out.budgets.insert(class.key().to_string(), new);
        }
    }
    if let Some(new) = fit_weights(lines, t, now).filter(|_| t.path_weights) {
        let write = (file.weights.as_ref())
            .is_none_or(|cur| weights_moved(&new, cur, t.min_change_percent));
        if write {
            log.push(format!("tuning: path weights {}", weights_text(&new)));
            out.weights = Some(new);
        }
    }
    (out, log)
}

#[path = "refit_propose.rs"]
mod propose;
#[path = "refit_tuned.rs"]
mod tuned;
pub use propose::{apply, dismiss, proposals};
pub use tuned::{Tuned, report, tuned, tuned_with};

#[cfg(test)]
#[path = "refit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "refit_tests_quality.rs"]
mod quality_tests;

#[cfg(test)]
#[path = "refit_tests_text.rs"]
mod text_tests;

#[cfg(test)]
#[path = "refit_tests_edges.rs"]
mod edge_tests;
