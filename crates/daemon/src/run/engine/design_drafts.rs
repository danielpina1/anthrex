//! Milestone 9.6 task M9.6.8: a design agent's write (task 6's review c), part of
//! `design_agents.rs`: a brainstormer's draft, accepted only from its live window,
//! checked against its template and stored. Pure (design decision 2).

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
    let live = (design.brainstormers.iter()).position(|a| {
        a.role == call.role
            && a.window_id == Some(call.window_id)
            && a.state == DesignAgentState::Running
    });
    let (Some(k), AgentRole::Brainstormer) = (live, call.role) else {
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
            match submit_draft(run, k, &doc.text, now, fx) {
                Ok(n) => {
                    let text = json!({"accepted": true, "kind": "brainstorm_draft", "version": n});
                    fx.push(Effect::Reply {
                        reply,
                        result: Ok(text.to_string()),
                    });
                    fx.push(Effect::PlannerAccepted {
                        window_id: call.window_id,
                    });
                    settle(run, now);
                }
                Err(text) => refuse(fx, reply, text),
            }
        }
        _ => refuse(fx, reply, "a brainstormer submits its brainstorm draft"),
    }
}

/// DF §3.3: brainstormer `k`'s draft, checked against its template (capped, then
/// cleaned), stored as its draft and written by the driver. Its version number.
fn submit_draft(
    run: &mut Run,
    k: usize,
    raw: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<u32, String> {
    if run.state != RunState::Brainstorming {
        return Err(format!("run {} is {}", run.id, run.state.label()));
    }
    let text = template::admit(DocKind::BrainstormDraft, raw, &TemplateCtx::default())?;
    let label =
        (run.orch.design.as_ref()).map_or_else(String::new, |d| d.brainstormers[k].label.clone());
    let author = DocAuthor::Brainstormer {
        label: label.clone(),
    };
    let doc = NewDoc::new(DocKind::BrainstormDraft, author, "submitted", &text);
    let (version, write) = store(run, doc, now)?;
    fx.push(write);
    if let Some(design) = run.orch.design.as_mut() {
        design.brainstormers[k].state = DesignAgentState::Done;
    }
    let session = (run.orch.design.as_ref()).map_or(0, |d| d.brainstormers[k].session);
    let record = format!("{label}/{session}");
    history::note_result(run, (AgentRole::Brainstormer, &record), DRAFT_ACCEPTED);
    log(
        run,
        now,
        format!("brainstormer {label} submitted its draft"),
    );
    Ok(version.n)
}
