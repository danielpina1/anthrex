//! Milestone 9.6 task M9.6.8: how the brainstorm ends (DF §3.4, §3.5, §8.4): both
//! drafts in, one failure, both failures and `run resume`, over budget, and a daemon
//! restart. The helpers are `design_agents.rs`'s.

use proto::{AgentRole, RunState};

use super::design_agents::*;
use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch_restore::resume;
use crate::run::design::state::DesignAgentState;
use crate::run::engine::{EngineState, EventKind, ScoutEnd};

/// DF §3.4: both drafts in wake the orchestrator once, exactly, and the brainstorming
/// clock starts then.
#[test]
fn both_drafts_in_wakes_the_orchestrator() {
    let mut fx = brainstorming();
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    assert!(notes(&fx).is_empty(), "one draft is not both");
    assert_eq!(fx.run().orch.design.as_ref().unwrap().phase_started, None);
    answered(&submit_draft(&mut fx, CODEX, DRAFT));
    let note = "both brainstorm drafts are in; read them with get_doc and submit the merged report";
    assert_eq!(notes(&fx), [note]);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.phase_started, Some(fx.now));
    // Their sessions end after the retire; that changes nothing but their usage.
    ended(&mut fx, "claude", 1, ScoutEnd::Reported);
    ended(&mut fx, "codex", 1, ScoutEnd::Reported);
    assert_eq!(notes(&fx), [note]);
    let a = &agents(&fx)[0];
    assert_eq!(
        (a.state.clone(), a.calls, a.tokens),
        (DesignAgentState::Done, 7, 120)
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let outcomes: Vec<Option<proto::RoleOutcome>> = (fx.run().role_routing_decisions.iter())
        .filter(|d| d.role == AgentRole::Brainstormer)
        .map(|d| d.outcome)
        .collect();
    assert_eq!(outcomes, [Some(proto::RoleOutcome::Completed); 2]);
}

/// DF §3.5: one brainstormer failing (here its session ends with no draft) continues
/// with the other's draft; the orchestrator is told which failed and why, and the
/// merged report must then name it.
#[test]
fn one_failure_continues_with_the_other() {
    let mut fx = brainstorming();
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    let reason = "the brainstormer's process exited without an accepted draft (code 1)";
    ended(
        &mut fx,
        "codex",
        1,
        ScoutEnd::Failed {
            reason: reason.into(),
        },
    );
    assert_eq!(states(&fx)[1], DesignAgentState::Failed(reason.into()));
    assert_eq!(
        notes(&fx),
        [format!(
            "one brainstormer failed (codex: {reason}); read the other draft with get_doc and submit the merged report"
        )]
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
    // A session that ends without a draft, and with no failure of its own, fails too.
    let mut fx = brainstorming();
    ended(&mut fx, "claude", 1, ScoutEnd::Reported);
    assert_eq!(
        states(&fx)[0],
        DesignAgentState::Failed("the brainstormer ended without an accepted draft".into())
    );
}

/// DF §3.5: both failing halt the run, with both reasons, exactly; `run resume`
/// relaunches both, fresh, and the brainstorming clock waits for their drafts again.
#[test]
fn both_failures_halt_and_resume_relaunches() {
    let mut fx = brainstorming();
    let failed = |reason: &str| ScoutEnd::Failed {
        reason: reason.into(),
    };
    ended(&mut fx, "claude", 1, failed("a"));
    assert_eq!(fx.run().state, RunState::Brainstorming);
    ended(&mut fx, "codex", 1, failed("b"));
    let text = "design flow: both brainstormers failed: a; b";
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
    assert!(fx.run().halt_retryable);
    assert!(log_lines(&fx).contains(&text.to_string()));
    assert!(notes(&fx).is_empty());
    let effects = resume(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let sessions: Vec<(String, u32)> = (launches(&fx).into_iter().skip(2))
        .map(|(_, s)| (s.kind.label(), s.session))
        .collect();
    assert_eq!(sessions, [("claude".into(), 2), ("codex".into(), 2)]);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Running]
    );
    assert_eq!(fx.run().orch.design.as_ref().unwrap().phase_started, None);
}

/// DF §2.2: a brainstormer over its budget (the scout machine stops it past its calls
/// or minutes) counts as failed, with the machine's reason.
#[test]
fn over_budget_counts_as_failed() {
    let mut fx = brainstorming();
    let spec = &launches(&fx)[0].1;
    let budget = fx.run().limits.orch.design.brainstormer;
    assert_eq!(
        (spec.max_tool_calls, spec.timeout_secs),
        (budget.tool_calls, u64::from(budget.minutes) * 60)
    );
    let reason = format!("the brainstormer ran longer than {} s", spec.timeout_secs);
    ended(
        &mut fx,
        "claude",
        1,
        ScoutEnd::Failed {
            reason: reason.clone(),
        },
    );
    assert_eq!(states(&fx)[0], DesignAgentState::Failed(reason.clone()));
    let record = (fx.run().role_routing_decisions.iter())
        .find(|d| d.session_id == "claude/1")
        .unwrap();
    assert_eq!(record.outcome, Some(proto::RoleOutcome::Failed));
    assert_eq!(record.result.as_deref(), Some(reason.as_str()));
}

/// DF §3.5 and §8.4: a brainstormer running at a daemon restart is relaunched fresh
/// with the same pack (its answers are kept), as a new session, once the run resumes;
/// a draft already in is kept.
#[test]
fn a_restart_relaunches_running_brainstormers_fresh() {
    let mut fx = brainstorming();
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    let stored = serde_json::to_string(fx.run()).unwrap();
    let run: crate::run::model::Run = serde_json::from_str(&stored).unwrap();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs: vec![run],
        replay: Vec::new(),
        held: Vec::new(),
    });
    assert_eq!(fx.run().state, RunState::Paused);
    let codex = agents(&fx)[1].clone();
    assert_eq!(
        (codex.state, codex.window_id),
        (DesignAgentState::Queued, None)
    );
    let before = launches(&fx).len();
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let relaunched: Vec<(String, u32)> = (launches(&fx).into_iter().skip(before))
        .map(|(_, s)| (s.kind.label(), s.session))
        .collect();
    assert_eq!(relaunched, [("codex".into(), 2)]);
    assert_eq!(states(&fx)[0], DesignAgentState::Done, "its draft is kept");
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.answers.as_deref(), Some("skip"));
    // The old session's record was closed by the restore.
    let old = (fx.run().role_routing_decisions.iter())
        .find(|d| d.session_id == "codex/1")
        .unwrap();
    assert_eq!(old.outcome, Some(proto::RoleOutcome::Interrupted));
}

/// A run rejected while it brainstorms stops its brainstormers: each live session is
/// stopped, and none settles the brainstorm afterwards. (`run cancel` is refused in
/// brainstorming, as in planning; `planners::halt_all` stops them where it applies.)
#[test]
fn a_rejected_run_stops_its_brainstormers() {
    let mut fx = brainstorming();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let stopped: Vec<u32> = (effects.iter())
        .filter_map(|e| match e {
            crate::run::engine::Effect::StopPlanner { window_id, .. } => Some(*window_id),
            _ => None,
        })
        .collect();
    assert_eq!(stopped, [CLAUDE, CODEX], "{effects:?}");
    let reason = "the run was rejected".to_string();
    assert_eq!(
        states(&fx),
        [
            DesignAgentState::Failed(reason.clone()),
            DesignAgentState::Failed(reason)
        ]
    );
    ended(&mut fx, "claude", 1, ScoutEnd::Reported);
    assert!(notes(&fx).iter().all(|n| !n.contains("brainstorm draft")));
}
