//! Milestone 9.6 (DF §5.2, §9): the design flow's contract texts. Task M9.6.13 adds the
//! requirements a design run's worker and reviewer prompts carry (decisions 25 and 26);
//! task M9.6.14 adds rules 47–53 to a design run's first prompt, and where the run is to
//! each of its orchestrator's sessions (ruling T14-1). Pure.

use proto::{DesignMode, DocGateKind, DocKind, RunState};

use crate::run::design::spend::phase_name;
use crate::run::design::state::design_dir;
use crate::run::model::{Run, Task};

/// Decision 25: the requirements block's cap, in bytes.
pub const REQUIREMENTS_MAX: usize = 6 * 1024;

/// Decision 26: the reviewer's line after the block.
pub const JUDGE_LINE: &str = "Judge the change against each requirement above; cite its id (R2) in any finding that concerns it.";

/// Room kept for the cut marker, `\n[cut: <n> bytes]`.
const MARKER_ROOM: usize = 32;

/// Decision 25: the approved spec's requirements task `task` covers, in the spec's
/// order, and its Goal section, at most [`REQUIREMENTS_MAX`] bytes (a longer block is
/// cut and ends `[cut: <n> bytes]`, as the brainstorm pack does). `None` for a run
/// without the design flow, or a task that covers none of the approved requirements.
/// The requirements are the approved spec's, stored at its approval from the driver's
/// read-back (`DesignState.requirements`), never parsed here.
pub fn requirements_block(run: &Run, task: &Task) -> Option<String> {
    let design = run.orch.design.as_ref()?;
    let covers = &task.spec.covers;
    let lines: Vec<String> = (design.requirements.iter())
        .filter(|r| covers.contains(&r.id))
        .map(|r| format!("{}  {}", r.id, r.text))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let full = format!(
        "Spec requirements this task delivers:\n{}\nGoal (from the spec): {}",
        lines.join("\n"),
        design.goal_section
    );
    if full.len() <= REQUIREMENTS_MAX {
        return Some(full);
    }
    let mut kept = REQUIREMENTS_MAX - MARKER_ROOM;
    while !full.is_char_boundary(kept) {
        kept -= 1;
    }
    Some(format!(
        "{}\n[cut: {} bytes]",
        &full[..kept],
        full.len() - kept
    ))
}

/// Rules 47 to 52 (the brief's "Contracts (exact)"), numbered on from the contract's
/// rule 46, and rule 53 (ruling T11-1: a design run's sub-planner needs `covers`). One
/// literal, as `round_rules!`, with the run's questions limit's place held by
/// `{max_questions}`. They ride in a design run's first prompt, so in its handoff too,
/// never in `ORCHESTRATOR_CONTRACT`, which every run shares (the brief's M9.6.1 note).
macro_rules! design_rules {
    () => {
        "47. This run uses the design flow: brainstorming, then specifying, then planning. Each ends at a gate only the user opens; read verdicts with run_status, never assume one.
48. In brainstorming, ask the user at most {max_questions} short questions in your window, one at a time; when they answer or say skip, call start_brainstorm with their answers.
49. When both brainstorm drafts are in, read them with get_doc and submit one merged report with submit_doc kind \"brainstorm\": where they agree, where they disagree (each side, then your judgment), the approaches tagged [<label>] or [both], one recommendation naming a listed approach, and questions for the user. Never paste a draft wholesale.
50. In specifying, write the spec in the template you were given; every requirement is a line \"R<n> …\" with its acceptance check. Submit with ready false for review; answer every finding (\"fixed\" or \"kept: <reason>\") in the submit with ready true.
51. In planning, every requirement must be covered by a task's covers, and every brief has the headings Files:, Tests first:, Steps:, Acceptance:, Verify:. Answer the plan review's findings in edit_plan's responses when you submit again.
52. When the user asks for changes, revise and submit the next version; the user's note is in run_status. Never approve, and never call a gate approved.
53. In planning, spawn_subplanner needs covers: the requirement ids its epic owns. Its sub-planner is given those requirements, and the whole plan's coverage is still checked when you submit."
    };
}

/// `design_rules!`, as written.
pub const DESIGN_RULES: &str = design_rules!();

/// The line above the rules in a design run's first prompt.
pub const RULES_HEAD: &str = "Your contract's rules continue, for this run:";

/// The first prompt's line naming the phases and the design tools, each by its Claude id
/// at its first mention (the tool-search fix's form).
pub const DESIGN_LINE: &str = "Design flow: brainstorming, then specifying, then planning, under rules 47 to 53 below. Its tools: start_brainstorm (in Claude: mcp__anthrex__start_brainstorm), submit_doc (in Claude: mcp__anthrex__submit_doc) and get_doc (in Claude: mcp__anthrex__get_doc).";

/// Where a design run's orchestrator starts: rule 48 holds the questions limit.
pub const DESIGN_START: &str = "Start with get_context, then scout, then follow rule 48.";

/// Whether `run` uses the design flow (its frozen mode; a continued goal's first prompt
/// is built before its design state is).
fn is_design(run: &Run) -> bool {
    run.design_mode == DesignMode::Full
}

/// Rules 47 to 53 with the run's frozen `max_questions`; `None` without the design flow.
pub fn design_rules(run: &Run) -> Option<String> {
    let n = run.limits.orch.design.max_questions;
    is_design(run).then(|| DESIGN_RULES.replace("{max_questions}", &n.to_string()))
}

/// The design run's first-prompt lines between the gate line and the start line, and
/// its start line; `None` without the design flow (9.5's lines stand).
pub fn first_prompt_lines(run: &Run) -> Option<(&'static str, &'static str)> {
    is_design(run).then_some((DESIGN_LINE, DESIGN_START))
}

/// The head of the line [`where_the_run_is`] builds.
const WHERE_HEAD: &str = "Where the run is now (run_status has anything newer):";

/// Ruling T14-1 (DF §8.4): where a design run is, for the first prompt of each of its
/// orchestrator's sessions, built when the session starts: the phase (the gate's own
/// while one is open; paused or halted said so), the open gate's kind and version, and
/// every approved document's path in the design folder. `None` without the design flow.
pub fn where_the_run_is(run: &Run) -> Option<String> {
    if !is_design(run) {
        return None;
    }
    let design = run.orch.design.as_ref()?;
    let (state, held) = match run.state {
        RunState::Paused => (run.paused_from, " (paused)"),
        RunState::Halted => (design.halted_from, " (halted)"),
        state => (Some(state), ""),
    };
    let gate = design.gate.as_ref();
    let phase = match (gate, state) {
        (Some(g), _) => format!("{}{held}", phase_name(g.kind)),
        (None, Some(state)) => format!("{}{held}", state.label()),
        (None, None) => run.state.label().to_string(),
    };
    let open = match gate {
        None => "none".to_string(),
        Some(g) => {
            let how = match g.revising {
                Some(_) => "which you are revising",
                None => "waiting for the user",
            };
            format!("{} v{}, {how}", g.kind.label(), g.version)
        }
    };
    let approved = approved_count(run, gate.map(|g| g.kind), state);
    let kinds = [DocKind::Brainstorm, DocKind::Spec, DocKind::Plan];
    let docs: Vec<String> = (kinds[..approved].iter())
        .filter_map(|&kind| {
            let n = (kind == DocKind::Spec)
                .then_some(design.approved_spec)
                .flatten();
            let v = design.find(kind, n).filter(|v| v.n > 0)?;
            let path = design_dir(run).join(design.file_name(v));
            Some(format!("{} v{} ({})", kind.label(), v.n, path.display()))
        })
        .collect();
    let docs = match docs.is_empty() {
        true => "none".to_string(),
        false => docs.join(", "),
    };
    Some(format!(
        "{WHERE_HEAD} phase {phase}; open gate: {open}; approved documents: {docs}."
    ))
}

/// How many of the brainstorm, the spec and the plan the user has approved: an open
/// gate's earlier ones, a document phase's earlier ones, all three once the plan is
/// approved (round 1's `approved_at`).
fn approved_count(run: &Run, gate: Option<DocGateKind>, state: Option<RunState>) -> usize {
    match (gate, state) {
        (Some(DocGateKind::Brainstorm), _) | (None, Some(RunState::Brainstorming)) => 0,
        (Some(DocGateKind::Spec), _) | (None, Some(RunState::Specifying)) => 1,
        (Some(DocGateKind::Plan), _) | (None, Some(RunState::Planning)) => 2,
        _ if run.approved_at.is_some() => 3,
        _ => match run.orch.design.as_ref().and_then(|d| d.approved_spec) {
            Some(_) => 2,
            None => 0,
        },
    }
}

/// Ruling T14-1: what a session of `run`'s orchestrator is first sent: `first` (the
/// stored first prompt, or a handoff), then, in a design run, [`where_the_run_is`] as of
/// now. The stored prompt never carries the line, so no later session's is stale or
/// doubled; a run without the design flow gets `first` byte for byte.
pub fn session_prompt(run: &Run, first: &str) -> String {
    match where_the_run_is(run) {
        Some(line) => format!("{first}\n{line}"),
        None => first.to_string(),
    }
}

#[cfg(test)]
#[path = "contract_design_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "contract_design_tests_prompts.rs"]
mod tests_prompts;
