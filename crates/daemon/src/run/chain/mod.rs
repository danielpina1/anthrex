//! Milestone 9.3 decisions 19–21 (KG §3): chains, the runs that share one orchestrator
//! session. Pure (design decision 2): no file, process, network or thread I/O, no
//! async runtime and no clock (the Verification grep).
//!
//! The chain table is derived from the runs: `Run.chain` is the only persisted chain
//! state, and [`rebuild`] makes the table from it at a restart. The engine keeps it as
//! runs start and end (`engine/chains.rs`); the driver resolves a chained call with
//! [`resolve`] and limits an idle orchestrator's tools with [`idle_refusal`].

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{AgentRole, IdleOrchestrator, RunState, Runtime, ToolCall};

use super::model::Run;

/// The tools an idle orchestrator may call (decision 21): its last run's two reads and
/// `start_goal`.
pub const IDLE_TOOLS: [&str; 3] = ["get_context", "run_status", "start_goal"];

/// KG §3.1's chain, plus D8's `ended`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chain {
    /// `o-<h4 of its first run>`.
    pub id: String,
    /// The user's checkout.
    pub project: PathBuf,
    /// Run ids in order; the last is the current one.
    pub runs: Vec<String>,
    /// The orchestrator's PTY window: its current run's, 0 until its launch answers.
    pub window_id: u32,
    pub runtime: Runtime,
    pub model: String,
    pub state: ChainState,
    /// Its session is gone: the window closed or exited, or the daemon restarted while
    /// it was idle (KG §3.2, §3.6).
    pub ended: bool,
}

/// `Active` while its current run is not terminal; `Idle` once it is accepted or
/// discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainState {
    Active,
    Idle,
}

impl Chain {
    /// An active chain whose one run is `run`; `None` for a run with no orchestrator.
    pub fn new(id: String, run: &Run) -> Option<Chain> {
        let o = run.orch.orchestrator.as_ref()?;
        Some(Chain {
            id,
            project: run.project.clone(),
            runs: vec![run.id.clone()],
            window_id: o.window_id.unwrap_or_default(),
            runtime: o.route.runtime,
            model: o.route.model.clone(),
            state: ChainState::Active,
            ended: false,
        })
    }

    /// The chain's current run: its last.
    pub fn current(&self) -> &str {
        self.runs.last().map_or("", String::as_str)
    }
}

/// `o-<h4>`, after the run that starts the chain.
pub fn chain_id(run: &Run) -> String {
    format!("o-{}", run.short())
}

/// Task M9.3.6a fix round 1, m1: whether a run `id` would start a chain whose id is
/// already a chain's in the table, or a run's own (a chain dropped from the table
/// keeps its id on its runs, which a restart's [`rebuild`] would merge). `o-<h4>`
/// keeps only the id's last four characters, so the run-id draw avoids these.
pub fn suffix_taken(
    chains: &BTreeMap<String, Chain>,
    runs: &BTreeMap<String, Run>,
    id: &str,
) -> bool {
    let cut = id.len().saturating_sub(4);
    let chain = format!("o-{}", id.get(cut..).unwrap_or(id));
    chains.contains_key(&chain) || runs.values().any(|r| r.chain.as_deref() == Some(&chain))
}

/// Decision 19, at `EventKind::Restore`: one chain per `Run.chain`, its runs oldest
/// first. A chain whose last run failed is dropped; one whose last run was accepted or
/// discarded is idle and ended (its window died with the daemon, KG §3.6), the
/// project's newest only; any other is active.
pub fn rebuild(runs: &BTreeMap<String, Run>) -> BTreeMap<String, Chain> {
    let mut members: BTreeMap<&str, Vec<&Run>> = BTreeMap::new();
    for run in runs.values() {
        if let Some(chain) = run.chain.as_deref() {
            members.entry(chain).or_default().push(run);
        }
    }
    let mut chains = BTreeMap::new();
    let mut idle = Vec::new();
    for (id, mut list) in members {
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let Some(&last) = list.last() else {
            continue;
        };
        let Some(mut chain) = Chain::new(id.to_string(), last) else {
            continue;
        };
        chain.runs = list.iter().map(|r| r.id.clone()).collect();
        match last.state {
            RunState::Failed => continue,
            RunState::Accepted | RunState::Discarded => {
                idle.push((ended_at(last), chain.id.clone()))
            }
            _ => {}
        }
        chains.insert(chain.id.clone(), chain);
    }
    // The oldest first, so the project's newest idle chain is the one kept.
    idle.sort();
    for (_, id) in idle {
        make_idle(&mut chains, &id);
    }
    for chain in chains.values_mut() {
        chain.ended |= chain.state == ChainState::Idle;
    }
    chains
}

/// When `run` ended, as near as the run says: its last log entry.
fn ended_at(run: &Run) -> u64 {
    run.log.last().map_or(run.created_at, |entry| entry.at)
}

/// Decision 19: chain `id` becomes idle, and the project's other idle chain, if any,
/// leaves the table (its window stays as a plain window); an active chain of the
/// project is untouched (KG §3.1).
pub fn make_idle(chains: &mut BTreeMap<String, Chain>, id: &str) {
    let Some(chain) = chains.get_mut(id) else {
        return;
    };
    chain.state = ChainState::Idle;
    let project = chain.project.clone();
    chains.retain(|other, c| other == id || c.state != ChainState::Idle || c.project != project);
}

/// Decision 20 (KG §3.4), as D16 amends it: the run a chained orchestrator call
/// reaches. `Ok(None)`: the call carries no chain, or is not the orchestrator's, or its
/// chain has left the table (its run failed, or it was the project's older idle chain)
/// and the run it names carries that chain; it stays as it is, as before 9.3 (KG §3.1,
/// decision 19). `Ok(Some)`: the chain's current run, for a call naming any run of the
/// chain. Otherwise refused, never redirected.
pub fn resolve(
    chains: &BTreeMap<String, Chain>,
    runs: &BTreeMap<String, Run>,
    call: &ToolCall,
) -> Result<Option<String>, String> {
    let Some(id) = call.chain.as_deref() else {
        return Ok(None);
    };
    if call.role != AgentRole::Orchestrator {
        return Ok(None);
    }
    let carries = || {
        runs.get(&call.run_id)
            .is_some_and(|r| r.chain.as_deref() == Some(id))
    };
    match chains.get(id) {
        Some(chain) if chain.runs.contains(&call.run_id) => Ok(Some(chain.current().to_string())),
        None if carries() => Ok(None),
        _ => Err(format!(
            "this window is the orchestrator of {id}; run {} is not one of its runs",
            call.run_id
        )),
    }
}

/// Decision 21 (KG §3.2): while `chain` is idle, every tool but [`IDLE_TOOLS`] is
/// refused, naming `last`, its last run.
pub fn idle_refusal(chain: &Chain, last: &Run, tool: &str) -> Option<String> {
    (chain.state == ChainState::Idle && !IDLE_TOOLS.contains(&tool)).then(|| {
        format!(
            "run {} has ended; start a new goal with start_goal when the user gives you one",
            last.short()
        )
    })
}

/// `RunsSnapshot.idle_orchestrators`: every idle chain whose last run is known.
pub fn idle_list(
    chains: &BTreeMap<String, Chain>,
    runs: &BTreeMap<String, Run>,
) -> Vec<IdleOrchestrator> {
    chains
        .values()
        .filter(|chain| chain.state == ChainState::Idle)
        .filter_map(|chain| {
            let last = runs.get(chain.current())?;
            Some(IdleOrchestrator {
                chain: chain.id.clone(),
                project: chain.project.clone(),
                after_run: last.id.clone(),
                outcome: last.state,
                runtime: chain.runtime,
                model: chain.model.clone(),
                window_id: (!chain.ended).then_some(chain.window_id),
                fresh: chain.ended,
                runs: u32::try_from(chain.runs.len()).unwrap_or(u32::MAX),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
