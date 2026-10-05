//! Milestone 9.6 decision 33 (DF §2.2, §8.4): the design agents' budgets, refitted from
//! history as a task class's are. A design class's samples are the agents of its role
//! whose outcome is `ok`, in the phase records (`HistoryLine::Phase`) of each run,
//! round and phase's latest approval (ruling T13-4: the highest `…/v<n>`). An agent's
//! spend is summed over its sessions in the phase (ruling T13-1), and its budget is a
//! session's, so a sample is its per-session mean, rounded (ruling T13-3). Samples are
//! capped per run and round and windowed as task samples are (ruling RH-3), and fitted
//! with decision 6's rule: tool calls from their calls, minutes from their active
//! seconds, tokens (with `refit_tokens`) from those that recorded any. Part of
//! `run::refit` (split for the 600-line rule), which uses its items. Pure.

use std::collections::HashMap;

use config::Tuning;
use proto::{AgentRole, ClassBudget, HistoryLine, PhaseAgent, PhaseRecord};

use super::{Sample, budget_of, cap_and_window, lower_median, qualifies};
use crate::run::design::spend::OK;

/// One design agent's spend in one phase record.
#[derive(Debug, Clone, Copy)]
pub(super) struct DesignSample<'a> {
    record: &'a PhaseRecord,
    agent: &'a PhaseAgent,
}

impl Sample for DesignSample<'_> {
    fn order(&self) -> (u64, &str) {
        (self.record.at, &self.record.record_id)
    }

    fn run_round(&self) -> (&str, u32) {
        (&self.record.run_id, self.record.round)
    }
}

/// The gate version a phase record's id names (`…/v<n>`, ruling T13-4); 0 for none.
fn approved_version(record: &PhaseRecord) -> u32 {
    let n = record
        .record_id
        .rsplit_once("/v")
        .map(|(_, n)| n.parse().ok());
    n.flatten().unwrap_or(0)
}

/// Each run, round and phase's latest approval: its highest version, the last line
/// among equals.
fn latest(lines: &[HistoryLine]) -> Vec<&PhaseRecord> {
    let mut kept: HashMap<(&str, u32, &str), &PhaseRecord> = HashMap::new();
    for line in lines {
        let HistoryLine::Phase(record) = line else {
            continue;
        };
        let key = (record.run_id.as_str(), record.round, record.phase.as_str());
        let newer = kept
            .get(&key)
            .is_none_or(|k| approved_version(k) <= approved_version(record));
        if newer {
            kept.insert(key, record);
        }
    }
    kept.into_values().collect()
}

/// `value` of one session of `agent`: its sum over its sessions, divided by them,
/// rounded half up.
fn per_session(agent: &PhaseAgent, value: u64) -> u64 {
    let n = u64::from(agent.sessions.max(1));
    value.saturating_add(n / 2) / n
}

/// `role`'s budget samples: its `ok` agents, capped per run and round, windowed.
pub(super) fn samples<'a>(
    lines: &'a [HistoryLine],
    role: AgentRole,
    t: &Tuning,
) -> Vec<DesignSample<'a>> {
    let all = (latest(lines).into_iter())
        .flat_map(|record| (record.agents.iter()).map(move |agent| DesignSample { record, agent }))
        .filter(|s| s.agent.role == role && s.agent.outcome == OK)
        .collect();
    cap_and_window(all, t)
}

/// The refit of `role`'s budget, when its samples qualify.
pub(super) fn fit(
    lines: &[HistoryLine],
    role: AgentRole,
    t: &Tuning,
    now: u64,
) -> Option<ClassBudget> {
    let samples = samples(lines, role, t);
    if !qualifies(&samples, t) {
        return None;
    }
    let median = |of: &[DesignSample], value: fn(&PhaseAgent) -> u64| {
        lower_median(&of.iter().map(|s| value(s.agent)).collect::<Vec<_>>())
    };
    let calls = median(&samples, |a| per_session(a, u64::from(a.calls)))?;
    let secs = median(&samples, |a| per_session(a, a.secs))?;
    // As ruling T8-5 for a task class: only the samples that recorded tokens, when they
    // qualify on their own and their median is above 0.
    let recorded: Vec<DesignSample> = (samples.iter().copied())
        .filter(|s| s.agent.tokens > 0)
        .collect();
    let tokens = (t.refit_tokens && qualifies(&recorded, t))
        .then(|| median(&recorded, |a| per_session(a, a.tokens)))
        .flatten()
        .filter(|&m| m > 0);
    Some(budget_of((calls, secs, tokens), samples.len(), t, now))
}
