//! Milestone 9's driver side. Task M9.6: decision 20's overlay of each run's scouts
//! onto the pure snapshot. Task M9.7: the user's hold verdicts and `run promote`,
//! handed to the engine. Task M9.11: decision 15's routing of the orchestrator's, the
//! sub-planners' and the worker's `task_note` calls, and the read path of decisions 16
//! to 18 (`run_status`'s long-poll, `get_context`, `task_result`).
//!
//! **The read path holds no engine lock but to look a run up and clone it.** Each
//! answer is built from that clone on `spawn_blocking`, and its file and git reads are
//! bounded by a timeout. `run_status` waits on the snapshot pushes (a `broadcast`
//! channel, resubscribed after a lag), never under the lock.

use std::time::Duration;

use proto::run_wire::request;
use proto::{AgentRole, RunReply, RunsSnapshot};
use proto::{Runtime, ToolCall};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use super::{RunService, unix_now};
use crate::run::chain::{idle_refusal, resolve};
use crate::run::engine::early::{awaits_launch, holds_planner_call};
use crate::run::engine::{EventKind, HOLD_LIMIT_SECS, OrchEvent, notes_seq};
use crate::run::model::{Run, task_branch};
use crate::run::orch::context::Asker;
use crate::run::orch::result::{TaskGit, task_result};
use crate::run::orch::tools::{OrchCall, parse_call};

#[path = "chain_ops.rs"]
mod chain_ops;
#[path = "orch_promote.rs"]
mod promote;
#[cfg(test)]
use promote::filled;
use promote::get_context;

/// How long `get_context`'s file reads and build may take (the stored profile and the
/// scout reports, each at most 1 MiB).
pub const CONTEXT_READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How often a call waiting for its launch looks again (review finding 1).
const LAUNCH_POLL: Duration = Duration::from_millis(50);

/// The most an early call waits for its launch: the engine's own hold limit.
const LAUNCH_WAIT: Duration = Duration::from_secs(HOLD_LIMIT_SECS);

/// The refusal of an orchestrator write whose launch was still in flight when its wait
/// ended (M9.11 re-review finding 5).
pub(super) const ORCHESTRATOR_LAUNCH_PENDING: &str =
    "the orchestrator's launch has not finished; call again once it has";

/// Decision 15: whether `call` is one of milestone 9's, which `orch_tool` routes: every
/// orchestrator and sub-planner call, and a worker's `task_note`.
pub(super) fn is_orch_call(call: &ToolCall) -> bool {
    matches!(call.role, AgentRole::Orchestrator | AgentRole::Planner)
        || (call.role == AgentRole::Worker && call.tool == "task_note")
}

/// Decision 15's refusal: `ToolResult { ok: false }` holding `{"error": "<text>"}`.
fn refused(text: impl Into<String>) -> RunReply {
    RunReply::tool_result(false, json!({ "error": text.into() }).to_string())
}

/// Decision 15's caller check, the engine's for writes: an orchestrator call comes
/// from the run's orchestrator window, a planner call from the live session of its
/// epic's sub-planner. Any other role is left to `parse_call`, which refuses it.
fn caller(run: &Run, call: &ToolCall) -> Result<(), String> {
    match call.role {
        AgentRole::Orchestrator => {
            let window = run.orch.orchestrator.as_ref().and_then(|o| o.window_id);
            if window == Some(call.window_id) {
                return Ok(());
            }
            Err(format!(
                "this window is not the orchestrator of run {}",
                run.id
            ))
        }
        AgentRole::Planner => {
            let epic = call.epic.as_deref().unwrap_or_default();
            let Some(record) = run.orch.epics.iter().find(|e| e.epic == epic) else {
                return Err(format!("unknown epic {epic}"));
            };
            if record.is_live_caller(call.window_id) {
                return Ok(());
            }
            Err(format!(
                "this window is not the sub-planner of epic {epic} of run {}",
                run.id
            ))
        }
        _ => Ok(()),
    }
}

fn answer(label: &str, result: Result<String, String>) -> RunReply {
    match result {
        Ok(message) => RunReply::done(label, message),
        Err(message) => RunReply::refused(label, message),
    }
}

impl RunService {
    /// Decision 20: each run's `RunInfo.scouts` is M8b's `ScoutService::run_scouts`,
    /// with its live counters, which the pure snapshot cannot call; and M9.0.5's
    /// decision 10: `RunsSnapshot.proposals` is the profile service's ready list. Called
    /// with the engine's lock released; each call takes only its own service's table.
    pub(super) fn with_scouts(&self, snap: RunsSnapshot) -> RunsSnapshot {
        self.overlaid(snap).0
    }

    /// [`with_scouts`](Self::with_scouts), and the ready list's generation it laid over
    /// the snapshot (0 before the daemon's services are set).
    pub(super) fn overlaid(&self, mut snap: RunsSnapshot) -> (RunsSnapshot, u64) {
        let Some(adaptation) = self.adaptation.get() else {
            return (snap, 0);
        };
        for run in snap.runs.iter_mut() {
            run.scouts = adaptation.scouts.run_scouts(&run.run_id);
        }
        let (generation, proposals) = adaptation.profiles.ready_proposals();
        snap.proposals = proposals;
        (snap, generation)
    }

    /// Whether the ready proposals changed since the last push (decision 10).
    pub(super) fn proposals_moved(&self) -> bool {
        let Some(adaptation) = self.adaptation.get() else {
            return false;
        };
        let generation = adaptation.profiles.ready_proposals().0;
        generation != crate::lock(&self.book).proposals_published
    }

    /// Decision 28: `run approve|reject --hold`, the only way a hold is decided.
    pub(super) async fn hold_verdict(
        &self,
        run_id: String,
        hold: String,
        approve: bool,
    ) -> RunReply {
        let (label, result) = if approve {
            let event = |reply| OrchEvent::ApproveHold {
                reply,
                run_id,
                hold,
            };
            (
                request::APPROVE,
                self.ask(|r| EventKind::Orch(event(r))).await,
            )
        } else {
            let event = |reply| OrchEvent::RejectHold {
                reply,
                run_id,
                hold,
            };
            (
                request::REJECT,
                self.ask(|r| EventKind::Orch(event(r))).await,
            )
        };
        answer(label, result)
    }

    /// Decision 15's routing: `get_context`, `run_status` and `task_result` go to the
    /// driver's read path; every other call, with the runtime refusals an `edit_plan`
    /// or `submit_epic` batch needs (ruling T22-I1b), goes to the engine as
    /// `OrchEvent::Tool`.
    pub(super) async fn orch_tool(&self, call: ToolCall) -> RunReply {
        self.orch_tool_within(call, LAUNCH_WAIT).await
    }

    /// [`Self::orch_tool`], with an early call's launch wait bounded by `limit` (a test
    /// shortens it; production passes [`LAUNCH_WAIT`]).
    pub(super) async fn orch_tool_within(&self, call: ToolCall, limit: Duration) -> RunReply {
        // Milestone 9.3 decisions 20 and 21, before any other check: a chained call
        // reaches its chain's current run, and an idle chain's tools are limited. `read`
        // is reached only from here, so its calls are resolved too.
        let call = match self.resolved(call) {
            Ok(call) => call,
            Err(text) => return refused(text),
        };
        let read = matches!(
            call.tool.as_str(),
            "get_context" | "run_status" | "task_result"
        );
        if read && call.role != AgentRole::Worker {
            return self.read(call, limit).await;
        }
        // The engine holds a sub-planner's early `submit_epic` itself; it has no hold
        // for the orchestrator, whose early calls wait here instead (review finding 1).
        // Still launching when the wait ends: refused here, since its runtime refusals
        // were not computed and a `Window` result could bind it before the engine
        // reads it (M9.11 re-review finding 5).
        if call.role == AgentRole::Orchestrator && !self.await_launch(&call, limit).await {
            return refused(ORCHESTRATOR_LAUNCH_PENDING);
        }
        // Decision 42e: a refresh's clean-tree check, for a caller that passes decision
        // 15's check, before the engine sees the call.
        if let Ok(OrchCall::EditPlan {
            edits,
            submit,
            summary,
            iterate,
        }) = parse_call(call.role, &call.tool, &call.args)
            && self.looked_up(&call, |_| ()).is_ok()
        {
            let alone = !submit && summary.is_none() && iterate.is_none();
            if let Err(text) = self.refresh_precheck(&call.run_id, &edits, alone).await {
                return refused(text);
            }
        }
        let refusals = match self.tool_refusals(&call).await {
            Ok(refusals) => refusals,
            Err(text) => return refused(text),
        };
        let event = |reply| {
            EventKind::Orch(OrchEvent::Tool {
                reply,
                call,
                refusals,
            })
        };
        match self.ask(event).await {
            Ok(text) => RunReply::tool_result(true, text),
            Err(text) => RunReply::tool_result(false, text),
        }
    }

    /// The runtime refusals of an `edit_plan` or `submit_epic` batch from a caller that
    /// passes decision 15's check, or whose call the engine will hold until its window
    /// is bound (`early::holds_planner_call`), so a replayed batch carries them too
    /// (review finding 2); none for any other call (the engine refuses those itself).
    async fn tool_refusals(&self, call: &ToolCall) -> Result<Vec<(Runtime, String)>, String> {
        let edits = match parse_call(call.role, &call.tool, &call.args) {
            Ok(OrchCall::EditPlan { edits, .. } | OrchCall::SubmitEpic { edits, .. }) => edits,
            _ => return Ok(Vec::new()),
        };
        let held = holds_planner_call(&crate::lock(&self.state), call);
        if !held && self.looked_up(call, |_| ()).is_err() {
            return Ok(Vec::new());
        }
        self.runtime_refusals(&call.run_id, &edits).await
    }

    /// Review finding 1: a call from a window nothing has yet, while the launch it may
    /// belong to is in flight (`early::awaits_launch`: its sub-planner's `StartPlanner`
    /// or the orchestrator's `CreateOrchestrator`), waits for that launch to resolve,
    /// at most the engine's own `HOLD_LIMIT_SECS`, polling the engine state with the
    /// lock taken only for each look. The caller is then checked as usual.
    ///
    /// `true` once the call no longer waits; `false` when `limit` passed with the
    /// launch still in flight.
    async fn await_launch(&self, call: &ToolCall, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let waiting = awaits_launch(&crate::lock(&self.state), call);
            if !waiting {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(LAUNCH_POLL).await;
        }
    }

    /// Decisions 20 and 21: `call` with its run resolved to its chain's current run
    /// (`chain::resolve`), or the refusal of a run outside the chain or of a tool an
    /// idle chain does not allow (`chain::idle_refusal`). The engine lock is taken only
    /// for the lookup: this is not async, so it is never held across an await.
    fn resolved(&self, mut call: ToolCall) -> Result<ToolCall, String> {
        let state = crate::lock(&self.state); // lookup
        let Some(current) = resolve(&state.chains, &state.runs, &call)? else {
            return Ok(call);
        };
        let chain = call.chain.as_deref().and_then(|id| state.chains.get(id));
        if let (Some(chain), Some(last)) = (chain, state.runs.get(&current))
            && let Some(text) = idle_refusal(chain, last, &call.tool)
        {
            return Err(text);
        }
        drop(state);
        call.run_id = current;
        Ok(call)
    }

    /// `read(run)` of the caller's run under the engine lock, after decision 15's run
    /// and caller checks; the lock is released on return.
    fn looked_up<T>(&self, call: &ToolCall, read: impl FnOnce(&Run) -> T) -> Result<T, String> {
        let state = crate::lock(&self.state);
        let Some(run) = state.runs.get(&call.run_id) else {
            return Err(format!("unknown run {}", call.run_id));
        };
        caller(run, call)?;
        Ok(read(run))
    }

    /// Decisions 16 to 18. Reads answer in every run state, terminal ones included.
    async fn read(&self, call: ToolCall, limit: Duration) -> RunReply {
        self.await_launch(&call, limit).await;
        if let Err(text) = self.looked_up(&call, |_| ()) {
            return refused(text);
        }
        let parsed = match parse_call(call.role, &call.tool, &call.args) {
            Ok(parsed) => parsed,
            Err(text) => return refused(text),
        };
        match parsed {
            OrchCall::RunStatus { since, wait_secs } => {
                if let Some(since) = since.filter(|_| wait_secs > 0) {
                    let wait = Duration::from_secs(wait_secs);
                    self.wait_digest(&call.run_id, since, wait).await;
                }
                self.run_status(&call).await
            }
            OrchCall::GetContext { scouts } => {
                let asker = match call.role {
                    AgentRole::Planner => Asker::Planner {
                        epic: call.epic.clone().unwrap_or_default(),
                    },
                    _ => Asker::Orchestrator,
                };
                match self.looked_up(&call, Run::clone) {
                    Ok(run) => get_context(run, asker, scouts).await,
                    Err(text) => refused(text),
                }
            }
            OrchCall::TaskResult { task_id } => match self.looked_up(&call, Run::clone) {
                Ok(run) => self.task_result(run, task_id).await,
                Err(text) => refused(text),
            },
            // `parse_call` lets no other call of these three tools through.
            _ => refused(format!("tool {} is not available here", call.tool)),
        }
    }

    /// Decision 16's wait: until the run's digest revision differs from `since`, the
    /// run is terminal or gone, or `wait` has passed. It reads the snapshot pushes and
    /// takes the engine lock only for one lookup after each (re)subscription, so a
    /// change published before it subscribed is not missed. After a lag it subscribes
    /// again (AGENTS.md: a lagged receiver resumes from the oldest message).
    async fn wait_digest(&self, run_id: &str, since: u64, wait: Duration) {
        let deadline = tokio::time::Instant::now() + wait;
        let ends = |rev: u64, state: proto::RunState| rev != since || state.is_terminal();
        loop {
            let mut pushes = self.pushes();
            let done = crate::lock(&self.state)
                .runs
                .get(run_id)
                .is_none_or(|run| ends(run.orch.digest_rev, run.state));
            if done {
                return;
            }
            loop {
                match tokio::time::timeout_at(deadline, pushes.recv()).await {
                    Err(_) | Ok(Err(RecvError::Closed)) => return,
                    Ok(Err(RecvError::Lagged(_))) => break,
                    Ok(Ok(snap)) => {
                        let run = snap.runs.iter().find(|r| r.run_id == run_id);
                        if run.is_none_or(|r| ends(r.digest_revision, r.state)) {
                            return;
                        }
                    }
                }
            }
        }
    }

    /// The digest of a clone of the run, built on `spawn_blocking`; then the read
    /// receipt with that clone's revision and wake-note seq (decision 16; M9.9 review
    /// M6), so a note added after the clone stays.
    async fn run_status(&self, call: &ToolCall) -> RunReply {
        let run = match self.looked_up(call, Run::clone) {
            Ok(run) => run,
            Err(text) => return refused(text),
        };
        let (run_id, digest_revision, seq) = (run.id.clone(), run.orch.digest_rev, notes_seq(&run));
        let now = unix_now();
        let built = tokio::task::spawn_blocking(move || {
            crate::run::orch::digest::digest(&run, now).to_string()
        })
        .await;
        match built {
            Ok(text) => {
                // Decision 39: what the answer held is neither kept nor pasted.
                self.wakes.read(&run_id, seq);
                self.send(EventKind::Orch(OrchEvent::DigestRead {
                    run_id,
                    digest_revision,
                    notes_seq: seq,
                }));
                RunReply::tool_result(true, text)
            }
            Err(error) => refused(format!("a blocking step did not finish: {error}")),
        }
    }

    /// Decision 18: the task's result, with its two git reads when it has a start
    /// commit (M9.6: only then), all under one `DONE_CHECK_GIT_TIMEOUT` deadline (M9.6
    /// review, M-4): past it the answer carries the git error, whatever the separate
    /// commands still had left. The build runs on `spawn_blocking`.
    async fn task_result(&self, run: Run, task_id: String) -> RunReply {
        let Some(task) = run.task(&task_id) else {
            return refused(format!("unknown task {task_id}"));
        };
        let git = match task.start_commit.clone() {
            Some(start) => Some(self.task_git(&run, &task_id, start).await),
            None => None,
        };
        let built = tokio::task::spawn_blocking(move || {
            run.task(&task_id)
                .map(|task| task_result(&run, task, git.as_ref()).to_string())
        })
        .await;
        match built {
            Ok(Some(text)) => RunReply::tool_result(true, text),
            Ok(None) => refused("unknown task"),
            Err(error) => refused(format!("a blocking step did not finish: {error}")),
        }
    }

    async fn task_git(&self, run: &Run, task_id: &str, start: String) -> Result<TaskGit, String> {
        let git = self.ctx.git.clone();
        let (root, branch) = (run.root.clone(), task_branch(&run.id, task_id));
        // M9.13a review, item 2: run work a refresh merged in is not the task's.
        let refreshed = run.task(task_id).map_or_else(Default::default, |t| {
            crate::run::git::RefreshedIn::of(&t.orch.refresh_merges, &t.orch.refresh_targets)
        });
        // `read_git` is `GitBudget::DONE_CHECK` in the daemon; tests pass their own.
        let budget = self.ctx.read_git;
        let each = budget.each(Duration::from_secs(run.limits.git_timeout_secs));
        let read = tokio::task::spawn_blocking(move || {
            crate::run::git::task_summary_excluding(&git, &root, &start, &branch, &refreshed, each)
        });
        match tokio::time::timeout(budget.deadline, read).await {
            Ok(Ok(result)) => result,
            Ok(Err(error)) => Err(format!("a blocking step did not finish: {error}")),
            Err(_) => Err(format!(
                "git did not answer within {} s",
                budget.deadline.as_secs()
            )),
        }
    }
}

#[cfg(test)]
#[path = "orch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "chain_tests.rs"]
mod chain_tests;

#[cfg(test)]
#[path = "orch_read_rig.rs"]
mod read_rig;

#[cfg(test)]
#[path = "orch_read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "orch_read_tests_tools.rs"]
mod read_tests_tools;

#[cfg(test)]
#[path = "orch_read_tests_launch.rs"]
mod read_tests_launch;

#[cfg(test)]
#[path = "refresh_tests.rs"]
mod refresh_tests;
