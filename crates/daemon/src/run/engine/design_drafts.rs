//! Milestone 9.6 task M9.6.8: a design agent's write (task 6's review c), part of
//! `design_agents.rs`: a brainstormer's draft, accepted only from its live window,
//! checked against its template and held until the brainstorm settles (ruling T8-1),
//! then stored and written ([`flush`]). Pure (design decision 2).

use proto::{AgentRole, DocAuthor, DocKind, RunState, ToolCall};
use serde_json::json;

use super::super::orch::refuse;
use super::super::requests::log;
use super::super::{Effect, ReplyId, history};
use super::{DRAFT_ACCEPTED, settle};
use crate::run::design::state::{DesignAgentState, NewDoc, not_design, store};
use crate::run::design::template::{self, TemplateCtx};
use crate::run::model::Run;
use crate::run::orch::tools::{OrchCall, parse_call};

/// A design agent's tool (task 6's review c), after `orch::tool`'s run-state gate: the
/// caller must be a live design agent of its role in its own window; a brainstormer's
/// one tool is `submit_doc` of its draft. A run being ended (`ending`, M9.9's C-1)
/// takes none.
pub(in crate::run::engine) fn tool(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    (ending, now): (Option<String>, u64),
    fx: &mut Vec<Effect>,
) {
    if let Some(text) = ending {
        return refuse(fx, reply, text);
    }
    let parsed = match parse_call(call.role, &call.tool, &call.args) {
        Ok(parsed) => parsed,
        Err(text) => return refuse(fx, reply, text),
    };
    let Some(design) = run.orch.design.as_ref() else {
        return refuse(fx, reply, not_design(&run.id));
    };
    let live = design.live_agent(call.role, call.window_id);
    let k = live.and_then(|a| (design.brainstormers.iter()).position(|b| b.label == a.label));
    let (Some(k), AgentRole::Brainstormer) = (k, call.role) else {
        let who = match call.role {
            AgentRole::Brainstormer => "a brainstormer",
            _ => "the document reviewer",
        };
        return refuse(
            fx,
            reply,
            format!("this window is not {who} of run {}", run.id),
        );
    };
    match parsed {
        OrchCall::SubmitDoc(doc) if doc.kind == DocKind::BrainstormDraft => {
            match submit_draft(run, k, &doc.text, now) {
                Ok(()) => {
                    let text = json!({"accepted": true, "kind": "brainstorm_draft"});
                    fx.push(Effect::Reply {
                        reply,
                        result: Ok(text.to_string()),
                    });
                    fx.push(Effect::PlannerAccepted {
                        window_id: call.window_id,
                    });
                    settle(run, now, fx);
                }
                Err(text) => refuse(fx, reply, text),
            }
        }
        _ => refuse(fx, reply, "a brainstormer submits its brainstorm draft"),
    }
}

/// DF §3.3: brainstormer `k`'s draft, checked against its template (capped, then
/// cleaned) and held (ruling T8-1): neither stored nor written until the brainstorm
/// settles, so the other brainstormer cannot read it by any path.
fn submit_draft(run: &mut Run, k: usize, raw: &str, now: u64) -> Result<(), String> {
    if run.state != RunState::Brainstorming {
        return Err(format!("run {} is {}", run.id, run.state.label()));
    }
    let text = template::admit(DocKind::BrainstormDraft, raw, &TemplateCtx::default())?;
    let Some(design) = run.orch.design.as_mut() else {
        return Err(not_design(&run.id));
    };
    let agent = &mut design.brainstormers[k];
    agent.state = DesignAgentState::Submitted;
    let (label, session) = (agent.label.clone(), agent.session);
    design.held.retain(|(l, _)| *l != label);
    design.held.push((label.clone(), text));
    let record = format!("{label}/{session}");
    history::note_result(run, (AgentRole::Brainstormer, &record), DRAFT_ACCEPTED);
    log(
        run,
        now,
        format!("brainstormer {label} submitted its draft"),
    );
    Ok(())
}

/// Ruling T8-1, when the brainstorm settles: each held draft, in the brainstormers'
/// order, is stored as its brainstormer's draft and written by the driver. A draft the
/// store refuses fails its brainstormer.
pub(super) fn flush(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let mut held = std::mem::take(&mut design.held);
    let submitted: Vec<usize> = (design.brainstormers.iter().enumerate())
        .filter(|(_, a)| a.state == DesignAgentState::Submitted)
        .map(|(k, _)| k)
        .collect();
    for k in submitted {
        let label = (run.orch.design.as_ref())
            .map_or_else(String::new, |d| d.brainstormers[k].label.clone());
        let text = (held.iter().position(|(l, _)| *l == label)).map(|i| held.remove(i).1);
        let stored = text
            .ok_or_else(|| "its held draft was lost".to_string())
            .and_then(|text| {
                let author = DocAuthor::Brainstormer {
                    label: label.clone(),
                };
                let doc = NewDoc::new(DocKind::BrainstormDraft, author, "submitted", &text);
                store(run, doc, now)
            });
        let state = match stored {
            Ok((_, write)) => {
                fx.push(write);
                DesignAgentState::Done
            }
            Err(reason) => DesignAgentState::Failed(reason),
        };
        if let Some(design) = run.orch.design.as_mut() {
            design.brainstormers[k].state = state;
        }
    }
}
