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
use std::time::Duration;

use proto::{AgentRole, DeliveryMode, IdleOrchestrator, RunState, Runtime, ToolCall};

use serde_json::Value;

use super::model::Run;
use super::orch::contract_rounds::{chain_left, no_chain_to_continue, not_last, still_going};
use super::orch::tools::{OrchCall, parse_call};

/// The final fix wave (review B, I1): the deadline on everything a continued start does
/// before the engine's `Start` (the lookup, the root detection, `goal_ready` with the
/// delivery's host preflight, the build and the handoff's history read), for a goal
/// from a request: the CLI's `--continue` and the TUI's goal dialog. `run start`'s
/// terms, as the CLI's `RUN_START_TIMEOUT` counts them: M8a's git preflight (180 s),
/// the host preflight (`PREFLIGHT_BOUND`), 30 s for the build and its tuning
/// (`TUNING_START_BOUND`, milestone 9.5 ruling T9-3). Past it the start is refused and
/// nothing was started (`contract_rounds::CONTINUE_TOO_SLOW`).
pub const CONTINUE_START_BOUND: Duration = Duration::from_secs(180)
    .saturating_add(crate::host::PREFLIGHT_BOUND)
    .saturating_add(Duration::from_secs(30))
    .saturating_add(crate::run::driver::tuning::TUNING_START_BOUND);

/// The same deadline for the orchestrator's `start_goal`: `anthrex mcp`'s
/// `TOOL_REPLY_TIMEOUT` (100 s) less 10 s for the engine step and the round trip, so
/// the tool always hears the daemon's answer, which is then true: the run started, or
/// nothing was.
pub const START_GOAL_TOOL_BOUND: Duration = Duration::from_secs(90);

/// The tools an idle orchestrator may call (decision 21): its last run's two reads and
/// `start_goal`.
pub const IDLE_TOOLS: [&str; 3] = ["get_context", "run_status", "start_goal"];

/// D17: whether `run` is finished for its chain: accepted, discarded, or delivered.
pub fn finished(run: &Run) -> bool {
    matches!(run.state, RunState::Accepted | RunState::Discarded) || delivered(run)
}

/// D17: a `pr` run complete and not cancelled, so every PR has landed; it has not
/// ended (it may still iterate, decision 9).
pub fn delivered(run: &Run) -> bool {
    run.state == RunState::Complete && run.delivery.mode == DeliveryMode::Pr && !run.cancelled
}

/// KG §3.3's `<accepted|discarded|delivered>`: how a finished run's outcome is named.
pub fn outcome(run: &Run) -> &'static str {
    if delivered(run) {
        "delivered"
    } else {
        run.state.label()
    }
}

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

/// `Active` while its current run is not finished; `Idle` once it is finished:
/// accepted, discarded, or delivered (D17, [`delivered`]).
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
/// keeps only the id's last four characters, so the run-id draw avoids these. Task
/// 6b (6a re-review N1): also any run's own suffix, since a plain run can be promoted
/// later and then starts `o-<its suffix>` (`engine/chains.rs::assign`).
pub fn suffix_taken(
    chains: &BTreeMap<String, Chain>,
    runs: &BTreeMap<String, Run>,
    id: &str,
) -> bool {
    let cut = id.len().saturating_sub(4);
    let suffix = id.get(cut..).unwrap_or(id);
    let chain = format!("o-{suffix}");
    chains.contains_key(&chain)
        || runs
            .values()
            .any(|r| r.chain.as_deref() == Some(&chain) || r.short() == suffix)
}

/// The newest run carrying `chain` (`Run.chain`), but `except`: the last in
/// [`in_order`]'s order, as [`rebuild`] orders a chain's runs.
pub fn newest<'a>(
    runs: &'a BTreeMap<String, Run>,
    chain: &str,
    except: Option<&str>,
) -> Option<&'a Run> {
    let of_chain = runs
        .values()
        .filter(|r| r.chain.as_deref() == Some(chain) && Some(r.id.as_str()) != except);
    in_order(of_chain.collect()).pop()
}

/// The final fix wave (review A, M2): `list`, one chain's runs, in their continue
/// order. Each run a continue linked (`Run.continued_by`, D17) follows the run it
/// continues, whatever their clocks say; runs no link orders (none in a chain made
/// since 9.3) fall back to creation time, then id. Unlinked runs start a sequence each,
/// the earliest first.
pub fn in_order(mut list: Vec<&Run>) -> Vec<&Run> {
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    let linked = |id: &str| list.iter().any(|r| r.continued_by.as_deref() == Some(id));
    let mut out: Vec<&Run> = Vec::with_capacity(list.len());
    for start in list.iter().filter(|r| !linked(&r.id)) {
        let mut next = Some(*start);
        while let Some(run) = next.filter(|r| !out.iter().any(|o| o.id == r.id)) {
            out.push(run);
            let to = run.continued_by.as_deref();
            next = to.and_then(|id| list.iter().copied().find(|r| r.id == id));
        }
    }
    // A cycle (never written) leaves runs unvisited: they follow, in time order.
    for run in &list {
        if !out.iter().any(|o| o.id == run.id) {
            out.push(run);
        }
    }
    out
}

/// Decision 19, at `EventKind::Restore`: one chain per `Run.chain`, its runs in their
/// continue order ([`in_order`]). A chain whose last run failed, or which the live table had dropped
/// (`Run.chain_left`), is left out; one whose last run is finished
/// (accepted, discarded, or delivered, D17) is idle and ended (its window died with the daemon, KG §3.6), the
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
    for (id, list) in members {
        let list = in_order(list);
        let Some(&last) = list.last() else {
            continue;
        };
        let Some(mut chain) = Chain::new(id.to_string(), last) else {
            continue;
        };
        chain.runs = list.iter().map(|r| r.id.clone()).collect();
        match last.state {
            RunState::Failed => continue,
            // The final fix wave (review A, M1): the live table had dropped it.
            _ if finished(last) && last.chain_left => continue,
            _ if finished(last) => idle.push((ended_at(last), chain.id.clone())),
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

/// When `run` ended (fix round 1, m3): when it was accepted or discarded, which
/// `complete::finished` logs as `accepted: <outcome>` or `discarded: <outcome>` at that
/// step; a run whose entry is gone (the log keeps 500) falls back to its last entry.
fn ended_at(run: &Run) -> u64 {
    terminal_at(run).unwrap_or_else(|| run.log.last().map_or(run.created_at, |entry| entry.at))
}

/// The time of an accepted or discarded run's `accepted: ` or `discarded: ` log entry,
/// or of a delivered run's last `complete: ` (D17; `complete::complete` logs it).
pub fn terminal_at(run: &Run) -> Option<u64> {
    let prefix = match run.state {
        RunState::Accepted => "accepted: ",
        RunState::Discarded => "discarded: ",
        _ if delivered(run) => "complete: ",
        _ => return None,
    };
    let entry = run.log.iter().rev().find(|e| e.text.starts_with(prefix))?;
    Some(entry.at)
}

/// Decision 19: chain `id` becomes idle, and the project's other idle chain, if any,
/// leaves the table (its window stays as a plain window); an active chain of the
/// project is untouched (KG §3.1).
pub fn make_idle(chains: &mut BTreeMap<String, Chain>, id: &str) {
    let Some(chain) = chains.get_mut(id) else {
        return;
    };
    chain.state = ChainState::Idle;
    // Task 6b (6a review m2): an orchestrator that never launched has no session to
    // adopt, so its chain is ended (a continue launches a fresh one).
    chain.ended |= chain.window_id == 0;
    let project = chain.project.clone();
    chains.retain(|other, c| other == id || c.state != ChainState::Idle || c.project != project);
}

/// Decision 20 (KG §3.4), as D16 amends it: the run a chained orchestrator call
/// reaches. `Ok(None)`: the call carries no chain, or is not the orchestrator's; it
/// stays as it is. `Ok(Some)`: the chain's current run, for a call naming any run of
/// the chain; or, when its chain has left the table (its run failed, or it was the
/// project's older idle chain) and the run it names carries that chain, the newest run
/// carrying it (task 6b: an adopted window's MCP target still names the chain's first
/// run), whose window stays a plain one (KG §3.1, decision 19), so its `start_goal` is
/// refused. Otherwise refused, never redirected.
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
        None if carries() && call.tool == "start_goal" => Err(chain_left(id)),
        None if carries() => Ok(newest(runs, id, None).map(|r| r.id.clone())),
        _ => Err(format!(
            "this window is the orchestrator of {id}; run {} is not one of its runs",
            call.run_id
        )),
    }
}

/// Decision 21 (KG §3.2): while `chain` is idle, every tool but [`IDLE_TOOLS`] is
/// refused, naming `last`, its last run. D17, as task 6b fix round 2 narrows it: a
/// delivered `last` has not ended, so an `edit_plan` with `args` holding `summary` alone
/// (decision 38) or `iterate` alone (decision 30, which makes the chain active again)
/// passes too; any other `edit_plan` is refused with the same text.
pub fn idle_refusal(chain: &Chain, last: &Run, tool: &str, args: &Value) -> Option<String> {
    let allowed = IDLE_TOOLS.contains(&tool)
        || (tool == "edit_plan" && delivered(last) && summary_or_iterate(args));
    (chain.state == ChainState::Idle && !allowed).then(|| {
        format!(
            "run {} has ended; start a new goal with start_goal when the user gives you one",
            last.short()
        )
    })
}

/// Task 6b fix round 2: an `edit_plan` call's `args` that parse to `summary` alone or
/// `iterate` alone (no edit, no submit).
fn summary_or_iterate(args: &Value) -> bool {
    match parse_call(AgentRole::Orchestrator, "edit_plan", args) {
        Ok(OrchCall::EditPlan {
            edits,
            submit,
            summary,
            iterate,
        }) => edits.is_empty() && !submit && (summary.is_some() != iterate.is_some()),
        _ => false,
    }
}

/// Decision 22, step 1 (KG §3.3, §3.5): the chain a goal continuing from run `after`
/// joins, and that run, or the refusal: an unknown run, a run whose chain is not in the
/// table, an earlier run of its chain, or a chain whose current run has not ended. The
/// project's check needs the goal's directory resolved, so it is the driver's.
pub fn continuable<'a>(
    chains: &'a BTreeMap<String, Chain>,
    runs: &'a BTreeMap<String, Run>,
    after: &str,
) -> Result<(&'a Chain, &'a Run), String> {
    let run = runs
        .get(after)
        .ok_or_else(|| format!("unknown run {after}"))?;
    let chain = run
        .chain
        .as_deref()
        .and_then(|id| chains.get(id))
        .ok_or_else(|| no_chain_to_continue(run.short()))?;
    let current = chain.current();
    let h4 = |id: &str| {
        runs.get(id)
            .map_or(id.to_string(), |r| r.short().to_string())
    };
    if current != after {
        return Err(not_last(run.short(), &chain.id, &h4(current)));
    }
    if chain.state == ChainState::Active {
        return Err(still_going(&h4(current)));
    }
    Ok((chain, run))
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

#[cfg(test)]
#[path = "continue_tests.rs"]
mod continue_tests;
