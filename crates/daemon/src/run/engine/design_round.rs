//! Milestone 9.6 task M9.6.15 (decisions 28 and 29, DF §8.1), engine side: a design
//! run's later rounds.
//!
//! - **The round's design** ([`mode`]): `amend` by default in a design run, `off`
//!   otherwise; `amend` or `full` on a run without the flow is refused.
//! - **Its start** ([`start`], from `goal_rounds::iterate`): `amend` enters specifying
//!   (its clock runs from there), `full` brainstorming with the brainstorm's state
//!   cleared for a new brainstorm round, `off` planning as 9.3. The round records its
//!   base (`design::round::DesignRound`), and the orchestrator is told how the round's
//!   spec is written.
//! - **A rejected round** ([`rejected`], from `goal_rounds_end::reject_round`): its
//!   design agents stop and the design state goes back to the approval before it.
//! - **A halt before the plan is approved** ([`halting`], from `merge::halt`; task 7's
//!   carry): the phase it left is kept, so `run resume` returns there and never starts
//!   a plan the user has not approved.
//!
//! Pure (design decision 2).

use proto::{RoundDesign, RunState};

use super::requests::log;
use super::{Effect, design, design_agents, design_spend, wake};
use crate::run::design::round::DesignRound;
use crate::run::model::Run;

/// Decision 28: `amend` or `full` on a run without the flow.
pub const NO_SPEC: &str = "this run has no spec to amend; iterate with --design off";

/// Decision 28: the round's design: `requested`, else `amend` in a design run and `off`
/// in any other; `amend` and `full` need a design run.
pub(super) fn mode(run: &Run, requested: Option<RoundDesign>) -> Result<RoundDesign, String> {
    let flow = run.orch.design.is_some();
    match (requested, flow) {
        (Some(RoundDesign::Off), _) | (None, false) => Ok(RoundDesign::Off),
        (None, true) => Ok(RoundDesign::Amend),
        (Some(_), false) => Err(NO_SPEC.into()),
        (Some(mode), true) => Ok(mode),
    }
}

/// Decision 28: round `run.round()` of a design run starts in `mode` (a run without the
/// flow plans as 9.3, untouched). The round's base is the approval before it; nothing
/// of the earlier round's gates, plan review or brainstorm carries over.
pub(super) fn start(run: &mut Run, mode: RoundDesign, now: u64) {
    let n = run.round();
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    // Ruling T15-1: the specs approved so far, which a later pack carries.
    design.approved_before = design.approved_specs();
    design.round = Some(DesignRound::starting(design, n, mode));
    design.gate = None;
    design.phase_started = None;
    design.plan_review_done = false;
    design.plan_revision = None;
    design.plan_fingerprint = None;
    if mode == RoundDesign::Full {
        // A new brainstorm round (ruling T8-5, T8-6): new answers, new brainstormers.
        design.answers = None;
        design.brainstormers.clear();
        design.drafts_settled = false;
        design.held.clear();
        design.rethink_starts.clear();
        design.read_back_owed = false;
    }
    let last = design.amend_base().len();
    run.state = match mode {
        RoundDesign::Amend => RunState::Specifying,
        RoundDesign::Full => RunState::Brainstorming,
        RoundDesign::Off => RunState::Planning,
    };
    if mode == RoundDesign::Amend {
        design::start_clock(run, now);
    }
    let how = match mode {
        RoundDesign::Amend => "amends the spec",
        RoundDesign::Full => "brainstorms, then amends the spec",
        RoundDesign::Off => "plans without the design flow",
    };
    log(run, now, format!("round {n} {how}"));
    if let Some(note) = note(mode, n, last) {
        wake::note(run, note);
    }
}

/// The orchestrator's note of how round `n` writes its spec (`last`: the base's last
/// requirement number).
fn note(mode: RoundDesign, n: u32, last: usize) -> Option<String> {
    let amendment = format!(
        "submit_doc kind \"spec\" with amend true: new requirements continue from R{}, a changed one keeps its number",
        last + 1
    );
    match mode {
        RoundDesign::Amend => Some(format!(
            "round {n} amends the spec: write the amendment with {amendment}; then plan only the new and changed requirements"
        )),
        RoundDesign::Full => Some(format!(
            "round {n} starts at the brainstorm: ask your questions (rule 48) and call start_brainstorm; its spec is an amendment, {amendment}"
        )),
        RoundDesign::Off => None,
    }
}

/// Decision 29 (9.3's rule): round `n`'s reject, or (ruling T15-2) its cancel, drops
/// only the round. Its brainstormers and reviewer stop, and the design state is the
/// approval before it again: its requirements, approved spec and sections, with no gate
/// and no clock, and none of the round's document reviews (m7), so a later round's
/// digest and checks never read them.
pub(super) fn rejected(run: &mut Run, reason: &str, fx: &mut Vec<Effect>) {
    design_agents::halt_all(run, reason, fx);
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let Some(round) = design.round.take() else {
        return;
    };
    design.requirements = round.base;
    design.approved_spec = round.spec_before;
    design.goal_section = round.goal_before;
    design.interfaces_section = round.interfaces_before;
    design.gate = None;
    design.phase_started = None;
    design.commit_due = false;
    design.spec_unread = false;
    design.reviews.truncate(round.reviews_before);
    // A halt in the dropped round no longer returns to its phase or gate.
    design.halted_from = None;
}

/// Task 7's carry: a halt of a design run before its plan is approved (in a design
/// phase, at a gate, or paused there) keeps the phase it left, unless the halt already
/// did (a phase budget's, a failed brainstorm's), and stops its clock: `run resume`
/// returns there (`design::resume_phase`), never to `running`.
pub(super) fn halting(run: &mut Run, now: u64) {
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    let state = match run.state {
        RunState::Paused => run.paused_from,
        state => Some(state),
    };
    let before = matches!(
        state,
        Some(
            RunState::Brainstorming
                | RunState::Specifying
                | RunState::Planning
                | RunState::AwaitingApproval
        )
    );
    if design.halted_from.is_some() || !before {
        return;
    }
    design_spend::stop_clock(run, now);
    if run.state == RunState::Paused {
        run.paused_from = None;
    }
    if let Some(design) = run.orch.design.as_mut() {
        design.halted_from = state;
    }
}
