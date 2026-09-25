//! Ruling T22-I1b: the runtimes a run can reach, through every rung and retry of
//! decision 39's escalation, rung 3's re-resolved reviewer, the roster's fallbacks and
//! each route's reviewer. Probes A and B are the re-review's.

use proto::{ModelEntry, Runtime, Strength};

use super::*;
use crate::run::roster::escalate;
use crate::run::test_support::{PROFILE, build_with, plan_with, task_toml};

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.to_string(),
        strength,
        note: String::new(),
    }
}

fn config(models: Vec<ModelEntry>, review_small: bool) -> config::Orchestrator {
    config::Orchestrator {
        models,
        review_small,
        ..config::Orchestrator::default()
    }
}

/// One `size` task owning `owns`, routed to `model` at `effort` on `runtime`.
fn run_of(
    config: &config::Orchestrator,
    size: &str,
    owns: &str,
    runtime: &str,
    model: &str,
    effort: &str,
) -> Run {
    let route = format!(
        "[task.route]\nruntime = \"{runtime}\"\nmodel = \"{model}\"\neffort = \"{effort}\""
    );
    let text = plan_with(PROFILE, &[task_toml("t1", size, owns, &route)]);
    build_with(&text, config).unwrap_or_else(|e| panic!("fixture plan must build: {e:?}"))
}

const DOCS: &str = "[\"docs/a.md\"]";
const HUB: &str = "[\"crates/proto/**\"]";

/// Probe A: an unreviewed S task on `claude-sonnet-5` at `medium`. Two escalations take
/// it to Codex (medium, high, then the peer), and rung 3's reviewer runs on Codex too.
#[test]
fn probe_a_an_unreviewed_claude_task_reaches_codex() {
    let run = run_of(
        &config(config::default_roster(), false),
        "S",
        DOCS,
        "claude",
        "claude-sonnet-5",
        "medium",
    );
    let t1 = &run.tasks[0];
    assert_eq!(t1.review_level, None, "the probe's task is not reviewed");
    let twice = escalate(&run.roster, &escalate(&run.roster, &t1.route));
    assert_eq!(twice.runtime, Runtime::Codex);
    assert_eq!(
        reachable_runtimes(&run),
        vec![Runtime::Claude, Runtime::Codex]
    );
}

/// Repeated escalation alone: a hub task is reviewed at `frontier`, which no Codex
/// entry meets, so its reviewers stay on Claude; only the third escalation (low,
/// medium, high, then the peer at the same strength) reaches Codex.
#[test]
fn every_rung_and_retry_of_escalation_is_reached() {
    let roster = vec![
        entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        entry(Runtime::Codex, "gpt-5-codex", Strength::Standard),
    ];
    let run = run_of(
        &config(roster, true),
        "M",
        HUB,
        "claude",
        "claude-sonnet-5",
        "low",
    );
    assert_eq!(
        reachable_runtimes(&run),
        vec![Runtime::Claude, Runtime::Codex]
    );
}

/// The escalation terms on their own (mutant M-esc drops them): the task already runs
/// at `high`, and its only way to Codex is the next rung's peer route.
#[test]
fn the_escalated_route_is_reached() {
    let roster = vec![
        entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        entry(Runtime::Codex, "gpt-5-codex", Strength::Standard),
    ];
    let run = run_of(
        &config(roster, true),
        "M",
        HUB,
        "claude",
        "claude-sonnet-5",
        "high",
    );
    let t1 = &run.tasks[0];
    assert_eq!(
        t1.review_route.as_ref().map(|r| r.runtime),
        Some(Runtime::Claude),
        "the reviewer must stay on Claude for this case to isolate escalation"
    );
    assert_eq!(
        reachable_runtimes(&run),
        vec![Runtime::Claude, Runtime::Codex]
    );
}

/// Rung 3's re-resolution alone: an unreviewed S task never escalates off Claude (no
/// Codex entry at its strength, no stronger Claude entry), but raised to M it is
/// reviewed, and its reviewer is the Codex entry.
#[test]
fn rung_3s_reviewer_is_reached_when_the_task_is_unreviewed() {
    let roster = vec![
        entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        entry(Runtime::Codex, "gpt-5-codex", Strength::Frontier),
    ];
    let run = run_of(
        &config(roster, false),
        "S",
        DOCS,
        "claude",
        "claude-sonnet-5",
        "high",
    );
    assert_eq!(run.tasks[0].review_level, None);
    assert_eq!(
        reachable_runtimes(&run),
        vec![Runtime::Claude, Runtime::Codex]
    );
}

/// A one-runtime roster: every escalation and every reviewer falls back to that
/// runtime, so the set holds it alone (the brief's Claude-only and Codex-only cases).
#[test]
fn a_one_runtime_roster_reaches_that_runtime_alone() {
    let claude = vec![
        entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        entry(Runtime::Claude, "claude-opus-5", Strength::Frontier),
    ];
    let run = run_of(
        &config(claude, true),
        "M",
        DOCS,
        "claude",
        "claude-sonnet-5",
        "low",
    );
    assert_eq!(reachable_runtimes(&run), vec![Runtime::Claude]);

    let codex = vec![entry(Runtime::Codex, "gpt-5-codex", Strength::Standard)];
    let run = run_of(
        &config(codex, true),
        "M",
        DOCS,
        "codex",
        "gpt-5-codex",
        "low",
    );
    assert_eq!(reachable_runtimes(&run), vec![Runtime::Codex]);
}

/// A Codex entry below every strength a Claude task's reviewers need and no Codex entry
/// at its strength: the task stays on Claude through every rung (the e2e edit test's
/// roster).
#[test]
fn a_peer_entry_no_rung_can_reach_is_not_counted() {
    let roster = vec![
        entry(Runtime::Claude, "claude-sonnet-5", Strength::Standard),
        entry(Runtime::Claude, "claude-opus-5", Strength::Frontier),
        entry(Runtime::Codex, "gpt-5-codex-mini", Strength::Fast),
    ];
    let run = run_of(
        &config(roster, false),
        "S",
        DOCS,
        "claude",
        "claude-sonnet-5",
        "medium",
    );
    assert_eq!(run.tasks[0].review_level, None);
    assert_eq!(reachable_runtimes(&run), vec![Runtime::Claude]);
}

/// T22-P2 (F4): only an edit that adds or changes a task can widen the runtimes a run
/// reaches, so only such a batch needs the git probe; a pause, an answer or a cancel
/// never fails on git.
#[test]
fn only_task_edits_may_widen_the_reach() {
    use proto::PlanEdit;
    let quiet = [
        PlanEdit::Pause,
        PlanEdit::Resume,
        PlanEdit::Finish,
        PlanEdit::CancelTask {
            task_id: "t1".into(),
        },
        PlanEdit::Answer {
            task_id: "t1".into(),
            text: "a".into(),
        },
        PlanEdit::AddDep {
            task_id: "t1".into(),
            dep: "t2".into(),
        },
    ];
    assert!(!edits_may_widen(&quiet));
    let amend = PlanEdit::AmendTask {
        task_id: "t1".into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: Some(proto::Size::M),
    };
    assert!(edits_may_widen(&[PlanEdit::Pause, amend]));
    assert!(edits_may_widen(&[PlanEdit::SplitTask {
        task_id: "t1".into(),
        into: Vec::new(),
    }]));
}
