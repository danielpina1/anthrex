//! Milestone 9.6 ruling T8-7: a Codex design agent is never resumed, so a turn that
//! ends without its submission is not nudged; the brainstormer is relaunched fresh
//! once, with the same pack and one more line, and a second such end fails it. The
//! helpers are `design_agents.rs`'s.

use proto::RunState;
use std::path::Path;

use super::design_agents::*;
use super::design_fixture::*;
use super::fixture::*;
use super::orch_restore::{restart, resume};
use crate::headless::SessionArg;
use crate::headless::argv::{CLI_CAPS, codex_args};
use crate::run::design::state::DesignAgentState;
use crate::run::engine::{Effect, ScoutEnd};
use crate::scout::design_spec::BRAINSTORMER_TEXTS;
use crate::scout::machine::unsubmitted;

const LINE: &str = "Your previous attempt ended without submitting; submit it now with submit_doc.";

/// `label`'s session `session` ended a turn without its draft, unnudged: the driver's
/// typed cause (task 8's re-review), never a failure's text.
fn unsubmitted_end(fx: &mut Fixture, label: &str, session: u32) -> Vec<Effect> {
    let reason = unsubmitted(&BRAINSTORMER_TEXTS);
    ended(fx, label, session, ScoutEnd::Unsubmitted { reason })
}

/// Its relaunch is a fresh session (never `codex exec resume`, with `--ephemeral`),
/// with the same first turn and the extra line; the other brainstormer runs on, and
/// nothing settles.
#[test]
fn a_codex_brainstormer_that_ends_without_submitting_is_relaunched_fresh_once() {
    let mut fx = brainstorming();
    let first = launches(&fx)[1].1.clone();
    assert_eq!(first.kind.label(), "codex");
    unsubmitted_end(&mut fx, "codex", 1);
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!((spec.kind.label(), spec.session), ("codex".to_string(), 2));
    assert_eq!(spec.first_turn, format!("{}\n{LINE}", first.first_turn));
    let new = SessionArg::New { uuid: None };
    let (exe, sock) = (Path::new("/opt/anthrex"), Path::new("/tmp/sock"));
    let args = codex_args(
        &spec.headless,
        &new,
        &spec.first_turn,
        exe,
        7,
        sock,
        &CLI_CAPS,
    );
    assert!(args.contains(&"--ephemeral".to_string()), "{args:?}");
    assert!(!args.contains(&"resume".to_string()), "{args:?}");
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Running]
    );
    assert!(agents(&fx)[1].unsubmitted);
    assert!(notes(&fx).is_empty());
    let record = (fx.run().role_routing_decisions.iter())
        .find(|d| d.session_id == "codex/1")
        .unwrap();
    assert_eq!(record.outcome, Some(proto::RoleOutcome::Failed));
}

/// A second end without a submission fails it, `ended twice without submitting`, and
/// the one-failed path goes on with the other brainstormer's draft.
#[test]
fn a_second_end_without_submitting_fails_it() {
    let mut fx = brainstorming();
    unsubmitted_end(&mut fx, "codex", 1);
    started(&mut fx, "codex", 803);
    let before = launches(&fx).len();
    unsubmitted_end(&mut fx, "codex", 2);
    assert_eq!(launches(&fx).len(), before, "no third launch");
    let reason = "ended twice without submitting";
    assert_eq!(states(&fx)[1], DesignAgentState::Failed(reason.into()));
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    assert_eq!(
        notes(&fx),
        [
            format!(
                "one brainstormer failed (codex: {reason}); read the other draft with get_doc and submit the merged report"
            ),
            template(&fx, Some(("codex", reason)))
        ]
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
}

/// A relaunch the restart interrupted is queued once and launched once, still as the
/// one relaunch, with its line.
#[test]
fn a_relaunch_survives_a_restore_once() {
    let mut fx = brainstorming();
    unsubmitted_end(&mut fx, "codex", 1);
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    let before = launches(&fx).len();
    resume(&mut fx);
    let codex: Vec<(u32, String)> = (launches(&fx).into_iter().skip(before))
        .filter(|(_, s)| s.kind.label() == "codex")
        .map(|(_, s)| (s.session, s.first_turn))
        .collect();
    assert_eq!(codex.len(), 1, "{codex:?}");
    assert_eq!(codex[0].0, 3);
    assert!(codex[0].1.ends_with(LINE), "{}", codex[0].1);
}

/// Both brainstormers ending twice without submitting is DF §3.5's both-failed halt,
/// with that reason each (task 8's test gap). Task 8's re-review (m1): `run resume` then
/// relaunches both with their one relaunch given back, so a Codex brainstormer's first
/// end without submitting relaunches it again instead of failing it.
#[test]
fn both_ending_twice_halts_and_a_resume_gives_each_its_relaunch_again() {
    let mut fx = brainstorming();
    unsubmitted_end(&mut fx, "claude", 1);
    unsubmitted_end(&mut fx, "codex", 1);
    started(&mut fx, "claude", 803);
    started(&mut fx, "codex", 804);
    unsubmitted_end(&mut fx, "claude", 2);
    unsubmitted_end(&mut fx, "codex", 2);
    let reason = "ended twice without submitting";
    let halted = format!("design flow: both brainstormers failed: {reason}; {reason}");
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(halted.as_str()));
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert!(
        agents(&fx).iter().all(|a| !a.unsubmitted),
        "{:?}",
        agents(&fx)
    );
    started(&mut fx, "codex", 805);
    let before = launches(&fx).len();
    unsubmitted_end(&mut fx, "codex", 3);
    assert_eq!(launches(&fx).len(), before + 1, "relaunched, not failed");
    let (_, spec) = launches(&fx).last().cloned().unwrap();
    assert_eq!((spec.kind.label(), spec.session), ("codex".to_string(), 4));
    assert!(spec.first_turn.ends_with(LINE), "{}", spec.first_turn);
    assert_eq!(states(&fx)[1], DesignAgentState::Running);
}
