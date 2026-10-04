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
use crate::run::engine::{Effect, EngineState, EventKind, OpResult, ScoutEnd};

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
    // Task 7's concern 5: halted as every halt is, its log line and its wake note.
    assert!(log_lines(&fx).contains(&format!("halted: {text}")));
    assert_eq!(notes(&fx), [format!("the run halted: {text}")]);
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
    // Fix round 1 (m5): the log says the brainstormers relaunch, not that a clock runs.
    let lines = log_lines(&fx);
    assert!(lines.contains(&"resumed; the brainstormers are relaunched".to_string()));
    assert!(!lines.contains(&"resumed; the phase's clock restarts".to_string()));
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

/// DF §3.5 and §8.4, ruling T8-1: at a daemon restart a running brainstormer, and one
/// whose draft was held (lost with the old daemon's memory), are both relaunched fresh
/// with the same pack (its answers are kept), as new sessions, once the run resumes.
#[test]
fn a_restart_with_one_draft_held_relaunches_both_brainstormers() {
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
    let windows: Vec<(DesignAgentState, Option<u32>)> = (agents(&fx).into_iter())
        .map(|a| (a.state, a.window_id))
        .collect();
    let queued = (DesignAgentState::Queued, None);
    assert_eq!(windows, [queued.clone(), queued]);
    let before = launches(&fx).len();
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let relaunched: Vec<(String, u32)> = (launches(&fx).into_iter().skip(before))
        .map(|(_, s)| (s.kind.label(), s.session))
        .collect();
    assert_eq!(relaunched, [("claude".into(), 2), ("codex".into(), 2)]);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(
        design.versions.is_empty(),
        "the held draft was never stored"
    );
    assert_eq!(design.answers.as_deref(), Some("skip"));
    // The old sessions' records were closed by the restore.
    let old = (fx.run().role_routing_decisions.iter())
        .find(|d| d.session_id == "codex/1")
        .unwrap();
    assert_eq!(old.outcome, Some(proto::RoleOutcome::Interrupted));
}

/// Ruling T8-1: no draft's file exists while the other brainstormer runs; both are
/// written, and indexed, when the drafts are in, and a failure's partner's draft too.
#[test]
fn the_drafts_are_written_only_once_both_brainstormers_have_ended() {
    let written = |effects: &[Effect]| -> Vec<String> {
        (effects.iter())
            .filter_map(|e| match e {
                Effect::WriteDoc { path, .. } => {
                    Some(path.file_name()?.to_string_lossy().into_owned())
                }
                _ => None,
            })
            .collect()
    };
    let mut fx = brainstorming();
    let codex = DRAFT.replace("Stored tokens.", "Signed links.");
    assert_eq!(
        written(&submit_draft(&mut fx, CLAUDE, DRAFT)),
        [] as [&str; 0]
    );
    let effects = submit_draft(&mut fx, CODEX, &codex);
    assert_eq!(written(&effects), ["draft-claude.md", "draft-codex.md"]);
    let texts: Vec<&str> = (effects.iter())
        .filter_map(|e| match e {
            Effect::WriteDoc { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(texts, [DRAFT, codex.as_str()]);
    let design = fx.run().orch.design.as_ref().unwrap();
    let labels: Vec<Option<&str>> = design.versions.iter().map(|v| v.label()).collect();
    assert_eq!(labels, [Some("claude"), Some("codex")]);
    assert!(design.held.is_empty());
    assert_eq!(
        states(&fx),
        [DesignAgentState::Done, DesignAgentState::Done]
    );
    // One draft and one failure: the draft is written when the failure ends the wait.
    let mut fx = brainstorming();
    assert_eq!(
        written(&submit_draft(&mut fx, CODEX, DRAFT)),
        [] as [&str; 0]
    );
    let failed = ScoutEnd::Failed { reason: "x".into() };
    assert_eq!(
        written(&ended(&mut fx, "claude", 1, failed)),
        ["draft-codex.md"]
    );
}

/// Fix round 1 (m1): the brainstorm does not settle while its run is paused; the
/// drafts are written and the orchestrator woken when it resumes.
#[test]
fn a_paused_run_settles_its_brainstorm_on_resume() {
    let mut fx = brainstorming();
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    let run = fx.run_mut();
    run.paused_from = Some(RunState::Brainstorming);
    run.state = RunState::Paused;
    let failed = ScoutEnd::Failed { reason: "x".into() };
    let effects = ended(&mut fx, "codex", 1, failed);
    assert!(
        !(effects.iter()).any(|e| matches!(e, Effect::WriteDoc { .. })),
        "{effects:?}"
    );
    assert!(notes(&fx).is_empty());
    let effects = resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert!(
        (effects.iter()).any(|e| matches!(e, Effect::WriteDoc { path, .. }
            if path.ends_with("brainstorm/draft-claude.md"))),
        "{effects:?}"
    );
    assert_eq!(notes(&fx).len(), 1, "{:?}", notes(&fx));
    assert!(notes(&fx)[0].starts_with("one brainstormer failed (codex: x)"));
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

/// Ruling T8-5: the drafts settle once per brainstorm round. Two pause and resume
/// cycles after the drafts are in, and a restart, leave the clock, the log line and
/// the wake note as they were, each single.
#[test]
fn the_drafts_settle_once_per_round() {
    let mut fx = brainstorming();
    answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    answered(&submit_draft(&mut fx, CODEX, DRAFT));
    let note = "both brainstorm drafts are in; read them with get_doc and submit the merged report";
    let seen = |fx: &Fixture| {
        let design = fx.run().orch.design.as_ref().unwrap();
        let line = "the brainstorm drafts are in";
        (
            design.phase_started,
            design.phase_paused_base,
            log_lines(fx).iter().filter(|l| *l == line).count(),
            notes(fx).iter().filter(|n| *n == note).count(),
        )
    };
    let first = seen(&fx);
    assert_eq!((first.2, first.3), (1, 1));
    for _ in 0..2 {
        fx.now += 60;
        let run = fx.run_mut();
        run.paused_from = Some(RunState::Brainstorming);
        run.state = RunState::Paused;
        resume(&mut fx);
        assert_eq!(fx.run().state, RunState::Brainstorming);
        assert_eq!(seen(&fx), first);
    }
    fx.now += 60;
    super::orch_restore::restart(&mut fx);
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    assert_eq!(seen(&fx), first);
    assert!(fx.run().orch.design.as_ref().unwrap().drafts_settled);
}

/// Ruling T8-6: the round's first start reports its pack file, and the round records
/// it, for every later start to be checked against.
#[test]
fn the_first_start_records_the_rounds_pack() {
    let fx = brainstorming();
    let frozen = fx.run().orch.design.as_ref().unwrap().pack.clone().unwrap();
    assert_eq!((frozen.round, frozen.file), (1, Some(PACK_FILE.clone())));
}

/// Ruling T8-6: a start whose pack cannot be read back halts the brainstorm with the
/// exact text, retryably; `run resume` relaunches that brainstormer, and the clock still
/// waits for the drafts.
#[test]
fn a_pack_that_cannot_be_read_back_halts_the_brainstorm() {
    let mut fx = design_launched(false);
    start_brainstorm(&mut fx);
    started(&mut fx, "claude", CLAUDE);
    let (op, _) = launches(&fx)[1].clone();
    let reason = "design/brainstorm/pack-r1.md: No such file or directory".to_string();
    fx.done(
        op,
        OpResult::DesignPackUnreadable {
            reason: reason.clone(),
        },
    );
    let text = format!("design flow: the brainstorm's input pack could not be read back: {reason}");
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text.as_str()));
    assert!(fx.run().halt_retryable);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Running, DesignAgentState::Queued]
    );
    let record = (fx.run().role_routing_decisions.iter())
        .find(|d| d.session_id == "codex/1")
        .unwrap();
    assert_eq!(record.outcome, Some(proto::RoleOutcome::Failed));
    let effects = resume(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    assert_eq!(fx.run().state, RunState::Brainstorming);
    let last = launches(&fx)
        .last()
        .map(|(_, s)| (s.kind.label(), s.session));
    assert_eq!(last, Some(("codex".into(), 2)));
    assert_eq!(fx.run().orch.design.as_ref().unwrap().phase_started, None);
    assert!(log_lines(&fx).contains(&"resumed; the brainstormers are relaunched".to_string()));
}

/// Fix round 2: a resume whose deferred settle halts the run (both brainstormers failed
/// while it was paused) says so in its reply.
#[test]
fn a_resume_that_ends_in_a_halt_says_so() {
    let mut fx = brainstorming();
    let run = fx.run_mut();
    run.paused_from = Some(RunState::Brainstorming);
    run.state = RunState::Paused;
    for (label, reason) in [("claude", "a"), ("codex", "b")] {
        let failed = ScoutEnd::Failed {
            reason: reason.into(),
        };
        ended(&mut fx, label, 1, failed);
    }
    assert_eq!(fx.run().state, RunState::Paused);
    let effects = resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Halted);
    let reason = "design flow: both brainstormers failed: a; b";
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} resumed and halted: {reason}"))]
    );
}
