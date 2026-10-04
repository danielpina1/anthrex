//! Milestone 9.6 decision 9, the driver's half (task M9.6.8): `OpKind::StartDesignAgent`
//! executed as a sub-planner's start is (`orch_ops.rs::start_planner`). A brainstormer's
//! first turn gets decision 11's input pack, read off the engine; the session starts on
//! the scout service; its end is sent as `OrchEvent::DesignAgentEnded`, with its usage
//! and tool calls, after the op's result reached the engine.
//!
//! No lock is held across an await: the engine's state is looked up and cloned under
//! its lock, and the files (the stored profile, the scout reports, a continued goal's
//! previous spec) are read on `spawn_blocking`, within `CONTEXT_READ_TIMEOUT` (AGENTS.md
//! rule 2).

use std::sync::Arc;

use proto::{DocKind, TokenUsage};

use super::design_io::DocQuery;
use super::ops::failed;
use super::orch::{CONTEXT_READ_TIMEOUT, context_reads};
use super::{OpCtx, RunService};
use crate::run::design::pack::{Earlier, PackInputs, pack, previous_spec};
use crate::run::engine::{EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::scout::design_spec::{DesignAgentKind, DesignAgentSpec};
use crate::scout::service::{ScoutHandle, ScoutOutcome, ScoutService};

impl RunService {
    /// Decision 9: design agent `spec` on the scout machine, a brainstormer's first turn
    /// with the input pack appended; its end is sent once this op's result is in.
    pub(super) async fn start_design_agent(
        self: &Arc<Self>,
        ctx: &OpCtx,
        mut spec: DesignAgentSpec,
    ) -> OpResult {
        let Some(scouts) = self.adaptation.get().map(|a| a.scouts.clone()) else {
            return failed("the scout service is not running");
        };
        if matches!(spec.kind, DesignAgentKind::Brainstormer { .. }) {
            let pack = self.brainstorm_pack(&ctx.run_id).await;
            spec.first_turn = format!("{}\n\n{pack}", spec.first_turn);
        }
        let (kind, session) = (spec.kind.clone(), spec.session);
        match scouts.start_design_agent(spec).await {
            Ok(handle) => {
                let window_id = handle.window_id;
                let (service, run_id) = (self.clone(), ctx.run_id.clone());
                tokio::spawn(async move {
                    let (outcome, usage, calls) = ended(&scouts, handle).await;
                    let k = kind.clone();
                    service
                        .settled(&run_id, move |op| {
                            matches!(op, OpKind::StartDesignAgent { spec }
                                if spec.kind == k && spec.session == session)
                        })
                        .await;
                    service.send(EventKind::Orch(OrchEvent::DesignAgentEnded {
                        run_id,
                        role: kind.role(),
                        label: kind.label(),
                        session,
                        outcome,
                        usage,
                        calls,
                    }));
                });
                OpResult::DesignAgentStarted { window_id }
            }
            Err(error) => failed(error.to_string()),
        }
    }

    /// Decision 11: run `run_id`'s input pack. What cannot be read in time is left out
    /// (the goal and the answers are always in).
    pub(super) async fn brainstorm_pack(&self, run_id: &str) -> String {
        let (run, previous) = {
            let state = crate::lock(&self.state);
            let Some(run) = state.runs.get(run_id) else {
                return String::new();
            };
            (run.clone(), previous_spec(state.runs.values(), run_id))
        };
        let mut inputs = PackInputs {
            goal: run.goal.clone(),
            answers: run.orch.design.as_ref().and_then(|d| d.answers.clone()),
            ..PackInputs::default()
        };
        if let Some((previous, n, path)) = previous {
            let query = DocQuery {
                kind: DocKind::Spec,
                version: Some(n),
                from: None,
                diff: false,
                findings: false,
            };
            match self.doc_view(&previous, query).await {
                Ok(view) => {
                    inputs.earlier = Some(Earlier {
                        path,
                        text: view.text,
                    })
                }
                Err(error) => tracing::warn!(run = %run_id, %error, "the earlier spec is left out"),
            }
        }
        let read = tokio::task::spawn_blocking(move || context_reads(&run));
        match tokio::time::timeout(CONTEXT_READ_TIMEOUT, read).await {
            Ok(Ok((profile, reports))) => {
                inputs.profile = profile.as_ref().map(crate::profile::summary);
                inputs.reports = reports;
            }
            _ => tracing::warn!(run = %run_id, "the pack's profile and reports are left out"),
        }
        pack(&inputs)
    }
}

/// How a design agent's session ended, what it spent and how many tools it called.
async fn ended(scouts: &ScoutService, handle: ScoutHandle) -> (ScoutEnd, TokenUsage, u32) {
    let info = |id: &str| scouts.info(id).map(|i| (i.usage, i.tool_calls));
    let outcome = match handle.outcome.await {
        Ok(ScoutOutcome::Accepted | ScoutOutcome::Report(_)) => ScoutEnd::Reported,
        Ok(ScoutOutcome::Failed { reason }) => ScoutEnd::Failed { reason },
        Err(_) => ScoutEnd::Failed {
            reason: "the session ended without an outcome".to_string(),
        },
    };
    let (usage, calls) = info(&handle.id).unwrap_or_default();
    (outcome, usage, calls)
}
