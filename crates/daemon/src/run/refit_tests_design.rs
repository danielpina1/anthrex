//! Milestone 9.6 decision 33 (DF §2.2, §8.4; task M9.6.13): the design agents' budgets
//! refit from the phase records' agents, as a task class's from its task records: the
//! `ok` samples of their role, at most `per_run_cap` per run and round, the newest
//! `window`, at least `min_samples`, times `budget_factor_percent`; an explicit
//! `[orchestrator.design.budget.<role>]` is only shown (ruling RH-5); a run freezes
//! the effective budgets at its start (ruling RH-8).

use proto::{
    AgentRole, Budget, ClassBudget, Effort, HISTORY_VERSION, HistoryLine, PhaseAgent, PhaseRecord,
    Route, Runtime, Strength, TuningFile,
};

use super::tests::NOW;
use super::{SizeClass, refit, tuned, tuned_with};

fn route() -> Route {
    Route {
        runtime: Runtime::Codex,
        model: "gpt-6".into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    }
}

fn agent(role: AgentRole, (calls, secs, tokens): (u32, u64, u64), outcome: &str) -> PhaseAgent {
    PhaseAgent {
        role,
        route: route(),
        calls,
        tokens,
        outcome: outcome.into(),
        secs,
    }
}

fn phase(run: u32, phase: &str, at: u64, agents: Vec<PhaseAgent>) -> HistoryLine {
    HistoryLine::Phase(PhaseRecord {
        v: HISTORY_VERSION,
        record_id: format!("run-{run}/phase/1/{phase}"),
        at,
        run_id: format!("run-{run}"),
        round: 1,
        phase: phase.into(),
        secs: 600,
        agents,
        gate_versions: 1,
        disputed: 0,
    })
}

/// 32 runs, each with two brainstormers (30 calls, 15 min) and a failed and an
/// over-budget one (never samples), a spec review (6 calls, 4 min) and a plan review
/// (6 calls, 4 min, 2 000 tokens).
fn history() -> Vec<HistoryLine> {
    let mut lines = Vec::new();
    for run in 0..32u32 {
        let at = 1_000 + u64::from(run) * 100;
        let ok = agent(AgentRole::Brainstormer, (30, 900, 50_000), "ok");
        let failed = agent(AgentRole::Brainstormer, (2, 10, 0), "failed: it crashed");
        let over = agent(AgentRole::Brainstormer, (90, 4_000, 0), "over budget");
        lines.push(phase(
            run,
            "brainstorming",
            at,
            vec![ok.clone(), ok, failed, over],
        ));
        let review = agent(AgentRole::DocReviewer, (6, 240, 2_000), "ok");
        lines.push(phase(run, "specifying", at + 10, vec![review.clone()]));
        lines.push(phase(run, "planning", at + 20, vec![review]));
    }
    lines
}

fn budget(tool_calls: u32, minutes: u32) -> Budget {
    Budget {
        tool_calls,
        minutes,
        tokens: None,
    }
}

#[test]
fn design_budgets_refit_from_phase_records() {
    let cfg = config::Orchestrator::default();
    assert_eq!(
        (
            cfg.design.budget.brainstormer,
            cfg.design.budget.doc_reviewer
        ),
        (budget(40, 15), budget(20, 10))
    );
    let (file, log) = refit(&history(), &TuningFile::default(), &cfg, NOW);
    // 64 brainstormer samples: 30 calls × 250% = 75, 15 min × 250% = 38 (rounded up).
    let brainstorm = ClassBudget {
        tool_calls: 75,
        minutes: 38,
        tokens: None,
        samples: 64,
        at: NOW,
    };
    assert_eq!(file.budgets.get("brainstorm"), Some(&brainstorm));
    // 64 reviewer samples, spec and plan: 6 calls → 15, 4 min → 10.
    let review = ClassBudget {
        tool_calls: 15,
        minutes: 10,
        tokens: None,
        samples: 64,
        at: NOW,
    };
    assert_eq!(file.budgets.get("doc_review"), Some(&review));
    assert!(
        log.contains(
            &"tuning: budget brainstorm 40 calls 15m → 75 calls 38m from 64 samples".to_string()
        ),
        "{log:?}"
    );
    assert!(
        log.contains(
            &"tuning: budget doc review 20 calls 10m → 15 calls 10m from 64 samples".to_string()
        ),
        "{log:?}"
    );
    // No task class has samples, so none is written.
    assert_eq!(file.budgets.len(), 2, "{:?}", file.budgets);

    // A run starting now uses them, and says so.
    let t = tuned(&file, &cfg);
    assert_eq!(t.effective(&cfg, SizeClass::Brainstorm), budget(75, 38));
    assert_eq!(t.effective(&cfg, SizeClass::DocReview), budget(15, 10));
    assert!(
        t.log
            .contains(&"tuning: budget brainstorm 75 calls 38m from 64 samples".to_string()),
        "{:?}",
        t.log
    );

    // Ruling RH-3: at most `per_run_cap` samples per run and round.
    let mut capped = cfg.clone();
    capped.tuning.table.per_run_cap = 1;
    let (file, _) = refit(&history(), &TuningFile::default(), &capped, NOW);
    assert_eq!(file.budgets["brainstorm"].samples, 32);

    // Too few samples: nothing is refitted.
    let few: Vec<HistoryLine> = history().into_iter().take(3 * 9).collect();
    let (file, _) = refit(&few, &TuningFile::default(), &cfg, NOW);
    assert!(file.budgets.is_empty(), "{:?}", file.budgets);

    // Ruling RH-5: an explicit `[orchestrator.design.budget.brainstormer]` is kept, and
    // the refit is only shown at the start.
    let mut explicit = cfg.clone();
    explicit.tuning.configured.brainstormer = true;
    let (file, _) = refit(&history(), &TuningFile::default(), &explicit, NOW);
    assert_eq!(file.budgets.get("brainstorm"), None);
    assert!(file.budgets.contains_key("doc_review"));
    let t = tuned_with(&history(), &file, &explicit);
    assert_eq!(
        t.effective(&explicit, SizeClass::Brainstorm),
        budget(40, 15)
    );
    let line = "tuning: budget brainstorm 40 calls 15m configured (refit would be 75 calls 38m)";
    assert!(t.log.contains(&line.to_string()), "{:?}", t.log);

    // Ruling RH-8: a run freezes them at its start.
    let (file, _) = refit(&history(), &TuningFile::default(), &cfg, NOW);
    let plan = crate::run::test_support::EXAMPLE_PLAN;
    let run = crate::run::test_support::build_tuned(plan, &cfg, tuned(&file, &cfg)).unwrap();
    assert_eq!(run.limits.orch.design.brainstormer, budget(75, 38));
    assert_eq!(run.limits.orch.design.doc_reviewer, budget(15, 10));
}
