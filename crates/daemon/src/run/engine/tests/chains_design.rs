//! Milestone 9.6 task M9.6.15, ruling T14-2: a chain's next goal that uses the design
//! flow and adopts the chain's live window is sent no first prompt, so its first wake,
//! after 9.3's next-goal wake, carries the design line, rules 47 to 53 with the run's
//! questions limit, where the run is, and its next step. A goal without the flow keeps
//! 9.3's wake byte for byte (`chains_continue.rs`).

use proto::{DesignMode, FinishAction, RunState};

use super::chains::ended;
use super::chains_continue::{NEXT, WAKE, continued, start};
use crate::run::orch::contract_design::{ASK_STEP, DESIGN_LINE, RULES_HEAD, design_rules};

#[test]
fn an_adopted_design_goals_first_wake_carries_the_design_rules() {
    let mut fx = ended(FinishAction::Accept);
    let mut run = continued(&fx, NEXT, Some("a handoff prompt"));
    // As `orch::make_planned` does for a run the driver froze `full`.
    run.design_mode = DesignMode::Full;
    crate::run::engine::design::enter(&mut run);
    start(&mut fx, run);
    let run = &fx.state.runs[NEXT];
    assert_eq!(run.state, RunState::Brainstorming);
    let rules = design_rules(run).unwrap();
    assert!(rules.contains("at most 5 short questions"), "{rules}");
    let place = "Where the run is now (run_status has anything newer): phase brainstorming; \
                 open gate: none; approved documents: none.";
    assert_eq!(
        run.orch.request_wake.as_deref(),
        Some(format!("{WAKE}\n{DESIGN_LINE}\n{RULES_HEAD}\n{rules}\n{place}\n{ASK_STEP}").as_str())
    );
}
