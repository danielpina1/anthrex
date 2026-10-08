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
        effort: Effort::HIGH,
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
        sessions: 1,
    }
}

fn phase(run: u32, phase: &str, at: u64, agents: Vec<PhaseAgent>) -> HistoryLine {
    approved(run, phase, (at, 1), agents)
}

/// Run `run`'s `phase` record of its gate's v`n` (ruling T13-4).
fn approved(run: u32, phase: &str, (at, n): (u64, u32), agents: Vec<PhaseAgent>) -> HistoryLine {
    HistoryLine::Phase(PhaseRecord {
        v: HISTORY_VERSION,
        record_id: format!("run-{run}/phase/1/{phase}/v{n}"),
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
    // Ruling T13-5 (m5): a design class's lines are a design run's only.
    let line = "tuning: budget brainstorm 75 calls 38m from 64 samples".to_string();
    assert!(t.design_lines.contains(&line), "{:?}", t.design_lines);
    assert!(!t.log.contains(&line), "{:?}", t.log);

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
    assert!(t.design_lines.contains(&line.to_string()), "{t:?}");

    // Ruling RH-8: a design run freezes them at its start (ruling T13-5, m5: when it
    // enters the design flow).
    let (file, _) = refit(&history(), &TuningFile::default(), &cfg, NOW);
    let plan = crate::run::test_support::EXAMPLE_PLAN;
    let mut run = crate::run::test_support::build_tuned(plan, &cfg, tuned(&file, &cfg)).unwrap();
    run.design_mode = proto::DesignMode::Full;
    crate::run::engine::design::enter(&mut run);
    assert_eq!(run.limits.orch.design.brainstormer, budget(75, 38));
    assert_eq!(run.limits.orch.design.doc_reviewer, budget(15, 10));
}

/// Ruling T13-5 (m5): a run without the design flow, in a repository whose
/// `tuning.toml` holds only design budgets, starts as 9.5 did: its log and REPORT.md's
/// `## Tuning` lines are those of an empty file, and the design budgets are not
/// frozen. A design run gets the design lines after them, the `none` line dropped.
#[test]
fn a_non_design_run_keeps_9_5s_tuning_lines() {
    let cfg = config::Orchestrator::default();
    let (file, _) = refit(&history(), &TuningFile::default(), &cfg, NOW);
    let none = tuned(&TuningFile::default(), &cfg).log;
    assert_eq!(tuned(&file, &cfg).log, none);
    let plan = crate::run::test_support::EXAMPLE_PLAN;
    let build = |file: &TuningFile| {
        crate::run::test_support::build_tuned(plan, &cfg, tuned(file, &cfg)).unwrap()
    };
    let (run, nine_five) = (build(&file), build(&TuningFile::default()));
    assert_eq!(run.tuning_lines, none);
    assert_eq!(run.tuning_lines, nine_five.tuning_lines);
    assert_eq!(run.log, nine_five.log);
    assert_eq!(run.limits.orch.design, nine_five.limits.orch.design);
    let tuning = |run: &crate::run::model::Run| {
        let report = crate::run::report::render(run, 2_000);
        let at = report.find("## Tuning").unwrap();
        let rest = &report[at..];
        rest[..rest[1..].find("\n## ").map_or(rest.len(), |n| n + 1)].to_string()
    };
    assert_eq!(tuning(&run), tuning(&nine_five));

    let mut design = build(&file);
    design.design_mode = proto::DesignMode::Full;
    crate::run::engine::design::enter(&mut design);
    let starts = [
        "tuning: budget brainstorm 75 calls 38m from 64 samples",
        "tuning: budget doc review 15 calls 10m from 64 samples",
    ];
    assert_eq!(design.tuning_lines, starts);
    let logged: Vec<&str> = (design.log.iter()).map(|e| e.text.as_str()).collect();
    assert!(starts.iter().all(|l| logged.contains(l)), "{logged:?}");
    assert!(
        !logged.iter().any(|l| l.starts_with("tuning: none")),
        "{logged:?}"
    );
}

/// Ruling T13-3: a design agent's budget is per session, so an agent whose sums cover
/// several sessions contributes its per-session mean, rounded.
#[test]
fn a_two_session_agent_contributes_its_per_session_mean() {
    let cfg = config::Orchestrator::default();
    let two = |calls, secs| PhaseAgent {
        sessions: 2,
        ..agent(AgentRole::Brainstormer, (calls, secs, 0), "ok")
    };
    let history = |calls, secs| -> Vec<HistoryLine> {
        (0..32)
            .map(|run| {
                phase(
                    run,
                    "brainstorming",
                    1_000 + u64::from(run),
                    vec![two(calls, secs)],
                )
            })
            .collect()
    };
    let (file, _) = refit(&history(60, 1_801), &TuningFile::default(), &cfg, NOW);
    // 30 calls and 901 s a session (1 801 / 2, rounded): 75 calls, 38 min.
    let fit = &file.budgets["brainstorm"];
    assert_eq!((fit.tool_calls, fit.minutes, fit.samples), (75, 38, 32));
    // 30.5 calls round to 31: × 250% is 77.5, rounded up to 78.
    let (file, _) = refit(&history(61, 1_800), &TuningFile::default(), &cfg, NOW);
    assert_eq!(file.budgets["brainstorm"].tool_calls, 78);
}

/// Ruling T13-4: a phase approved again after a back is a new record, `…/v<n>`; the
/// refit keeps only the highest version per run, round and phase.
#[test]
fn only_the_latest_approval_of_a_phase_is_a_sample() {
    let cfg = config::Orchestrator::default();
    let mut lines = Vec::new();
    for run in 0..32u32 {
        let at = 1_000 + u64::from(run) * 100;
        let early = agent(AgentRole::Brainstormer, (2, 60, 0), "ok");
        lines.push(approved(
            run,
            "brainstorming",
            (at, 1),
            vec![early.clone(), early],
        ));
        let late = agent(AgentRole::Brainstormer, (30, 900, 0), "ok");
        lines.push(approved(
            run,
            "brainstorming",
            (at + 50, 2),
            vec![late.clone(), late],
        ));
    }
    // The version decides, not the order in the file.
    lines.swap(0, 1);
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let fit = &file.budgets["brainstorm"];
    assert_eq!((fit.tool_calls, fit.minutes, fit.samples), (75, 38, 64));
}

/// Ruling T13-5 (m4): a design class is held to decision 6's floor and ceiling.
#[test]
fn a_design_class_is_held_to_the_floor_and_the_ceiling() {
    let cfg = config::Orchestrator::default();
    let lines = |calls, secs| -> Vec<HistoryLine> {
        (0..32)
            .map(|run| {
                let review = agent(AgentRole::DocReviewer, (calls, secs, 0), "ok");
                phase(run, "specifying", 1_000 + u64::from(run), vec![review])
            })
            .collect()
    };
    let (file, _) = refit(&lines(1, 30), &TuningFile::default(), &cfg, NOW);
    let fit = &file.budgets["doc_review"];
    assert_eq!((fit.tool_calls, fit.minutes), (10, 5));
    let (file, _) = refit(&lines(5_000, 100_000), &TuningFile::default(), &cfg, NOW);
    let fit = &file.budgets["doc_review"];
    assert_eq!((fit.tool_calls, fit.minutes), (2_000, 1_440));
}

/// Task M9.6.16 (carried from M9.6.13): `run stats` lists the design classes after the
/// task classes, as `brainstorm` and `doc review`, with their samples, budgets and
/// refits, `-` for a weight, and never a proposal (ruling T13-2). Milestone 9.8 (task
/// M9.8.13): no class shows a route any more. The class column widens to fit their labels.
#[test]
fn stats_list_the_design_classes_without_proposals() {
    use super::super::refit_render::render;
    let cfg = config::Orchestrator::default();
    let lines = history();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let r = super::report(&lines, &file, &cfg, std::path::Path::new("/tmp/t/t.toml"));
    let rows: Vec<(&str, u32)> = (r.classes.iter())
        .map(|c| (c.class.as_str(), c.samples))
        .collect();
    assert_eq!(rows.len(), 5, "{rows:?}");
    assert_eq!(rows[3..], [("brainstorm", 64), ("doc review", 64)]);
    let brainstorm = &r.classes[3];
    assert_eq!(brainstorm.budget, budget(75, 38));
    assert_eq!(brainstorm.refit, proto::RefitState::Written { at: NOW });
    assert_eq!(brainstorm.weight_secs, None);
    let names = |p: &proto::TuningProposal| p.id.contains("brainstorm") || p.id.contains("doc");
    assert!(!r.proposals.iter().any(names), "{:?}", r.proposals);
    let text = render(&r);
    assert!(
        text.contains("\n  CLASS       SAMPLES  BUDGET          WEIGHT  REFIT\n"),
        "{text}"
    );
    assert!(
        text.contains("\n  S           0/30     40 calls 15m"),
        "{text}"
    );
    assert!(
        text.contains("\n  brainstorm  64       75 calls 38m    -       "),
        "{text}"
    );
    assert!(
        text.contains("\n  doc review  64       15 calls 10m    -       "),
        "{text}"
    );
    // A design class below the minimum shows its count against it.
    let few: Vec<HistoryLine> = lines.into_iter().take(9).collect();
    let r = super::report(
        &few,
        &TuningFile::default(),
        &cfg,
        std::path::Path::new("/t"),
    );
    assert_eq!(
        (r.classes[3].samples, r.classes[3].refit),
        (6, proto::RefitState::NotYet)
    );
    assert!(render(&r).contains("\n  brainstorm  6/30     40 calls 15m    -       not yet\n"));
}

/// Task M9.6.16: a history with no design phase reports as 9.5 did: the three task
/// classes only, in the five-wide class column.
#[test]
fn stats_without_design_history_keep_the_task_classes_only() {
    use super::super::refit_render::render;
    let cfg = config::Orchestrator::default();
    let lines = super::tests::fixture_records("refit");
    assert!(!lines.iter().any(|l| matches!(l, HistoryLine::Phase(_))));
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let r = super::report(&lines, &file, &cfg, std::path::Path::new("/tmp/t/t.toml"));
    let classes: Vec<&str> = r.classes.iter().map(|c| c.class.as_str()).collect();
    assert_eq!(classes, ["S", "M", "hub"]);
    let text = render(&r);
    assert!(
        text.contains("\n  CLASS  SAMPLES  BUDGET          WEIGHT  REFIT\n"),
        "{text}"
    );
}
