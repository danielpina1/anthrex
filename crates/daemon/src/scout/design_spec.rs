//! Milestone 9.6 design decision 9: the design flow's headless agents on M8b's scout
//! machine, as sub-planners are (`planner.rs`): a brainstormer (task M9.6.8) and a
//! document reviewer (task M9.6.10). Each is read-only with a scout's sandbox and read
//! tools, its one write going to the engine (`submit_doc`, `submit_findings`), which
//! answers it and then retires the session (`ScoutService::accept_planner`) or stops it
//! (`stop_planner`), as for a sub-planner.
//!
//! Its spec is built by the engine ([`brainstormer_spec`], pure) and carried by
//! `OpKind::StartDesignAgent`; the driver appends the input pack to its first turn
//! (`run::design::pack`), and [`ScoutService::start_design_agent`] launches it.
//!
//! **Independence on disk** (task 6's review a). No draft's file exists while either
//! brainstormer runs: the engine holds each draft until both have ended (ruling T8-1),
//! which is what keeps a Codex brainstormer independent, since Codex's `read-only`
//! sandbox restricts writes only. As defence in depth, on Claude the run's design
//! folder is denied to the read tools (`permissions.deny`) and to commands
//! (`sandbox.filesystem.denyRead`), by its path as given and, added by the driver, its
//! canonical path (ruling T8-3). Ruling T8-4: no design agent's session is saved
//! (`headless::argv`'s `--no-session-persistence` and `--ephemeral`, keyed on the
//! role, so the document reviewer has it too), and a Claude brainstormer is also denied
//! the Codex sessions folder.

use std::path::PathBuf;
use std::sync::Arc;

use proto::{AgentRole, Route, RunRef, ScoutKind};
use serde::{Deserialize, Serialize};

use super::machine::{MachineTexts, ScoutLimits};
use super::planner::PlannerTag;
use super::service::{ScoutHandle, ScoutService};
use super::spec::{ScoutSpec, valid_id};
use crate::headless::{HeadlessSpec, McpTarget};
use crate::run::design::state::{DesignAgent, design_dir};
use crate::run::model::Run;

/// DF §3.1's lens A, exact (decision 11).
pub const LENS_A: &str = "Your angle: the smallest change that fully meets the goal.";
/// DF §3.1's lens B, exact (decision 11).
pub const LENS_B: &str = "Your angle: the most robust, long-lived design.";

/// A brainstormer's instructions (Contracts: under 1 KiB).
pub const BRAINSTORMER_CONTRACT: &str = "You are a brainstormer in an anthrex run. You read and think; you never change anything.
1. Your first message has the goal and an input pack. Check your claims against the repository, and cite file:line for every constraint you find.
2. Do not edit, create or delete files. Your session cannot write anything.
3. Write one draft, under 12 KiB, in exactly these sections: ## Understanding (the goal restated; what success looks like), ## Assumptions (each marked), ## Constraints found (with file:line), ## Approaches (2 or 3; each: how it works, files touched, trade-offs, risks, size S/M/L), ## Recommendation (one approach, and why), ## Questions for you.
4. Submit it with the anthrex tool submit_doc (in Claude: mcp__anthrex__submit_doc), kind \"brainstorm_draft\". If it is refused, fix what the refusal names and submit again. Then stop.
5. You work alone. Another brainstormer drafts the same goal; you never see its draft, and you cannot read it.";

/// Sent once when a brainstormer's turn ends without an accepted draft.
pub const BRAINSTORMER_NUDGE: &str = "[anthrex] Your turn ended without an accepted draft. Call submit_doc with kind \"brainstorm_draft\" now, then stop.";

/// A brainstormer reached its budget's tool calls (the machine's wrap-up).
pub fn brainstormer_wrap_up(n: u32) -> String {
    format!(
        "[anthrex] You have used {n} tool calls. Stop reading and call submit_doc with your draft now."
    )
}

/// A brainstormer's machine texts.
pub const BRAINSTORMER_TEXTS: MachineTexts = MachineTexts {
    nudge: BRAINSTORMER_NUDGE,
    wrap_up: brainstormer_wrap_up,
    submit_tool: "mcp__anthrex__submit_doc",
    noun: "the brainstormer",
    missing: "an accepted draft",
};

/// A document reviewer's instructions (Contracts: under 1 KiB; task M9.6.10).
pub const DOC_REVIEWER_CONTRACT: &str = "You are a document reviewer in an anthrex run. You read and judge; you never change anything.
1. Your first message names the document to review. Read it with the anthrex tool get_doc (in Claude: mcp__anthrex__get_doc), and the approved brainstorm report with get_doc kind \"brainstorm\". Check its claims against the repository.
2. Do not edit, create or delete files. Your session cannot write anything.
3. Submit your findings once with submit_findings (in Claude: mcp__anthrex__submit_findings): each with an id (F1, F2, ...), a severity, its place and what is wrong. Then stop.
4. Use severity blocking only for placeholders, contradictions, untestable requirements, scope beyond the approved approach, dropped brainstorm decisions, a requirement without an acceptance check, or a plan task that does not deliver its covers. Everything else is minor. No findings is a valid review.";

/// Sent once when a document reviewer's turn ends without findings (task M9.6.10).
pub const DOC_REVIEWER_NUDGE: &str =
    "[anthrex] Your turn ended without findings. Call submit_findings now, then stop.";

/// A document reviewer reached its budget's tool calls.
pub fn doc_reviewer_wrap_up(n: u32) -> String {
    format!("[anthrex] You have used {n} tool calls. Stop reading and call submit_findings now.")
}

/// A document reviewer's machine texts.
pub const DOC_REVIEWER_TEXTS: MachineTexts = MachineTexts {
    nudge: DOC_REVIEWER_NUDGE,
    wrap_up: doc_reviewer_wrap_up,
    submit_tool: "mcp__anthrex__submit_findings",
    noun: "the document reviewer",
    missing: "accepted findings",
};

/// Task 6's review (b): a brainstormer's Claude tools, exactly: its one anthrex tool and
/// an area scout's read tools. A drift test keeps the anthrex half equal to
/// `mcp::tools::tools_for(Brainstormer)`.
pub const BRAINSTORMER_ALLOWED_TOOLS: &[&str] =
    &["mcp__anthrex__submit_doc", "Read", "Glob", "Grep"];

/// Task 6's review (b): a document reviewer's Claude tools, exactly (task M9.6.10
/// launches it).
pub const DOC_REVIEWER_ALLOWED_TOOLS: &[&str] = &[
    "mcp__anthrex__get_doc",
    "mcp__anthrex__submit_findings",
    "Read",
    "Glob",
    "Grep",
];

/// Which design agent a spec launches (decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignAgentKind {
    /// `claude`, `codex`, `A` or `B`.
    Brainstormer { label: String },
    /// The reviewed document's `<doc>-r<n>` (ruling T1-O3), task M9.6.10.
    DocReviewer { doc: String },
}

impl DesignAgentKind {
    /// The agent's label: `DesignAgent.label`, and its `--agent-label`.
    pub fn label(&self) -> String {
        match self {
            DesignAgentKind::Brainstormer { label } => label.clone(),
            DesignAgentKind::DocReviewer { doc } => doc.clone(),
        }
    }

    pub fn role(&self) -> AgentRole {
        match self {
            DesignAgentKind::Brainstormer { .. } => AgentRole::Brainstormer,
            DesignAgentKind::DocReviewer { .. } => AgentRole::DocReviewer,
        }
    }

    /// The role's submission tool, as its contract names it (ruling T8-7's line).
    pub fn submit_tool(&self) -> &'static str {
        match self {
            DesignAgentKind::Brainstormer { .. } => "submit_doc",
            DesignAgentKind::DocReviewer { .. } => "submit_findings",
        }
    }

    /// Its machine's texts: its nudge, its wrap-up and its failures' words.
    pub fn texts(&self) -> MachineTexts {
        match self {
            DesignAgentKind::Brainstormer { .. } => BRAINSTORMER_TEXTS,
            DesignAgentKind::DocReviewer { .. } => DOC_REVIEWER_TEXTS,
        }
    }
}

/// One design agent's session to launch, built by the engine and carried by
/// `OpKind::StartDesignAgent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignAgentSpec {
    pub run_id: String,
    pub kind: DesignAgentKind,
    pub session: u32,
    pub headless: HeadlessSpec,
    /// The engine's part of the first turn; the driver appends the input pack to a
    /// brainstormer's (decision 11).
    pub first_turn: String,
    pub project: PathBuf,
    pub cwd: PathBuf,
    pub route: Route,
    /// `[orchestrator.design.budget]`'s, as the run froze it (DF §2.2).
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
}

/// The engine's part of a brainstormer's first turn: what to do, and lens `A`'s or
/// `B`'s line with one runtime (decision 11). With two runtimes both get this same
/// neutral text, so their drafts compare fairly (DF §3.1).
pub fn brainstormer_first_turn(label: &str) -> String {
    let lens = match label {
        "A" => format!("\n{LENS_A}"),
        "B" => format!("\n{LENS_B}"),
        _ => String::new(),
    };
    format!(
        "[anthrex] Brainstorm the goal below on your own and submit one draft with submit_doc, kind \"brainstorm_draft\".{lens}"
    )
}

/// Decision 9: brainstormer `agent`'s session, launched read-only as a sub-planner is
/// (`orch::launch::read_only`: a scout's sandbox in the user's checkout), with its own
/// contract and tools, named by its label (`--agent-label`, ruling T1-O3), and the
/// run's design folder denied to it on Claude (task 6's review a).
pub fn brainstormer_spec(run: &Run, agent: &DesignAgent) -> DesignAgentSpec {
    let kind = DesignAgentKind::Brainstormer {
        label: agent.label.clone(),
    };
    let mut headless = crate::run::orch::launch::read_only(
        run,
        &agent.route,
        (BRAINSTORMER_CONTRACT, BRAINSTORMER_ALLOWED_TOOLS),
        mcp_target(run, &kind),
        run_ref(run, &kind, agent.session),
    );
    if let Some(sandbox) = headless.claude_sandbox.as_mut() {
        sandbox.deny_read.push(design_dir(run));
    }
    let budget = run.limits.orch.design.brainstormer;
    let mut first_turn = brainstormer_first_turn(&agent.label);
    // Ruling T8-7: the one relaunch after an attempt that ended without submitting.
    if agent.unsubmitted {
        first_turn = format!("{first_turn}\n{}", resubmit_line(kind.submit_tool()));
    }
    DesignAgentSpec {
        run_id: run.id.clone(),
        first_turn,
        kind,
        session: agent.session,
        headless,
        project: run.project.clone(),
        cwd: run.root.clone(),
        route: agent.route.clone(),
        max_tool_calls: budget.tool_calls,
        timeout_secs: u64::from(budget.minutes) * 60,
    }
}

/// The first turn of the reviewer of review `k` of the spec: its draft, named
/// explicitly (ruling T5-1), and its one submission.
/// Task M9.6.11: the plan's review reads the plan sent to it, named explicitly too
/// (ruling WB-A-I2), and the approved spec it must cover.
pub fn reviewer_first_turn(doc: proto::DocKind, k: u32) -> String {
    match doc {
        proto::DocKind::Plan => format!(
            "[anthrex] Review the plan sent to review {k}: read it with get_doc, kind \"plan\", draft {k}, and the approved spec with get_doc, kind \"spec\". Then submit your findings once with submit_findings."
        ),
        _ => format!(
            "[anthrex] Review the spec draft sent to review {k}: read it with get_doc, kind \"spec\", draft {k}. Then submit your findings once with submit_findings."
        ),
    }
}

/// Decision 9 (task M9.6.10): the document reviewer `agent` of review `k`, launched
/// read-only as a brainstormer is, with its own contract, tools and budget
/// (`[orchestrator.design.budget] doc_reviewer`), named `<doc>-r<k>` (ruling T1-O3).
pub fn reviewer_spec(
    run: &Run,
    agent: &DesignAgent,
    (doc, k): (proto::DocKind, u32),
) -> DesignAgentSpec {
    let kind = DesignAgentKind::DocReviewer {
        doc: agent.label.clone(),
    };
    let headless = crate::run::orch::launch::read_only(
        run,
        &agent.route,
        (DOC_REVIEWER_CONTRACT, DOC_REVIEWER_ALLOWED_TOOLS),
        mcp_target(run, &kind),
        run_ref(run, &kind, agent.session),
    );
    let budget = run.limits.orch.design.doc_reviewer;
    let mut first_turn = reviewer_first_turn(doc, k);
    // Ruling T8-7: the one relaunch after an attempt that ended without submitting.
    if agent.unsubmitted {
        first_turn = format!("{first_turn}\n{}", resubmit_line(kind.submit_tool()));
    }
    DesignAgentSpec {
        run_id: run.id.clone(),
        first_turn,
        kind,
        session: agent.session,
        headless,
        project: run.project.clone(),
        cwd: run.root.clone(),
        route: agent.route.clone(),
        max_tool_calls: budget.tool_calls,
        timeout_secs: u64::from(budget.minutes) * 60,
    }
}

/// Ruling T8-7: the line a relaunched design agent's first turn ends with, after its
/// previous attempt ended without submitting.
pub fn resubmit_line(tool: &str) -> String {
    format!("Your previous attempt ended without submitting; submit it now with {tool}.")
}

/// A design agent's machine limits: its budget, and (ruling T8-7) a nudge by resumed
/// turn only on Claude; a Codex design agent's session is never resumed.
pub fn design_limits(spec: &DesignAgentSpec) -> ScoutLimits {
    let claude = spec.route.runtime == proto::Runtime::Claude;
    ScoutLimits {
        timeout_secs: spec.timeout_secs,
        max_tool_calls: spec.max_tool_calls,
        send_mid_turn: claude,
        nudges: claude,
        texts: spec.kind.texts(),
    }
}

fn mcp_target(run: &Run, kind: &DesignAgentKind) -> McpTarget {
    McpTarget {
        role: kind.role(),
        run_id: run.id.clone(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: Some(kind.label()),
    }
}

fn run_ref(run: &Run, kind: &DesignAgentKind, session: u32) -> RunRef {
    RunRef {
        run_id: run.id.clone(),
        task_id: None,
        role: kind.role(),
        session,
        lane: None,
    }
}

/// A run's `<h4>` (`Run::short`): the last four characters of its id.
fn short(run_id: &str) -> &str {
    let cut = run_id.len().saturating_sub(4);
    run_id.get(cut..).unwrap_or(run_id)
}

/// A design agent's internal id `<h4>-design-<label>-<n>` (lower case, as scout ids
/// are) and window name: `<h4>/brainstorm-<label>.<n>` for a brainstormer,
/// `<h4>/review-<doc>.<n>` for a document reviewer.
pub fn design_names(run_id: &str, kind: &DesignAgentKind, session: u32) -> (String, String) {
    let h4 = short(run_id);
    let label = kind.label();
    let id = format!("{h4}-design-{}-{session}", label.to_ascii_lowercase());
    let name = match kind {
        DesignAgentKind::Brainstormer { .. } => format!("{h4}/brainstorm-{label}.{session}"),
        DesignAgentKind::DocReviewer { .. } => format!("{h4}/review-{label}.{session}"),
    };
    (id, name)
}

impl ScoutService {
    /// Starts design agent `spec` on the scout machine with its own budget and texts,
    /// tagged as a sub-planner's session is: its one write goes to the engine, and its
    /// session ends `Accepted` once the engine retires it.
    pub async fn start_design_agent(
        self: &Arc<Self>,
        spec: DesignAgentSpec,
    ) -> anyhow::Result<ScoutHandle> {
        let (id, name) = design_names(&spec.run_id, &spec.kind, spec.session);
        anyhow::ensure!(valid_id(&id), "invalid design agent id {id:?}");
        let tag = PlannerTag {
            run_id: spec.run_id.clone(),
            role: spec.kind.role(),
            epic: spec.kind.label(),
            session: spec.session,
            limits: design_limits(&spec),
        };
        // The table's bookkeeping spec: a design agent stores no report.
        let entry = ScoutSpec {
            id,
            kind: ScoutKind::Area,
            run_id: Some(spec.run_id.clone()),
            question: format!("design agent {}", spec.kind.label()),
            first_turn: spec.first_turn,
            cwd: spec.cwd,
            project: spec.project,
            web: false,
            codex_config: Vec::new(),
            base_sha: String::new(),
            repo_paths: Vec::new(),
        };
        self.launch(entry, spec.route, spec.headless, name, Some(tag))
            .await
    }
}

#[cfg(test)]
#[path = "design_spec_tests.rs"]
mod tests;
