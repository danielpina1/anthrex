//! Task M9.5.8: threshold and routing proposals, the quality evidence that holds a class
//! (ruling RH-2), model lists, dismissing and applying.

use std::path::Path;

use config::{Candidate, Pick, RouteList};
use proto::{
    BisectLine, ClassRoute, Effort, HISTORY_VERSION, HistoryLine, RevertRecord, Runtime,
    SizeThresholds, Strength, TaskOrigin, TuningChange, TuningFile,
};

use super::tests::{NOW, fixture_records, record};
use super::{
    M_ROUTE_LADDER, S_ROUTE_LADDER, SizeClass, apply, dismiss, failed_on_quality, proposals, refit,
    report, route_samples,
};

fn route(strength: Strength, effort: Effort) -> ClassRoute {
    ClassRoute { strength, effort }
}

fn ids(lines: &[HistoryLine], file: &TuningFile, cfg: &config::Orchestrator) -> Vec<String> {
    proposals(lines, file, cfg)
        .into_iter()
        .map(|p| p.id)
        .collect()
}

fn revert(n: u32, run: &str, task: Option<&str>, reverted: &str) -> HistoryLine {
    HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/revert/{n}"),
        at: 1_790_400_000 + u64::from(n),
        run_id: run.into(),
        task_id: task.map(str::to_string),
        reverted: reverted.into(),
        revert_commit: format!("rev{n}"),
    })
}

fn bisect(run: &str, culprit: &str) -> HistoryLine {
    HistoryLine::Bisect(BisectLine {
        v: HISTORY_VERSION,
        record_id: format!("{run}/bisect/1/1"),
        at: 1_790_400_000,
        run_id: run.into(),
        stage: 1,
        head: "abc".into(),
        tests: vec!["t".into()],
        range: 4,
        probes: 2,
        culprit: Some(culprit.into()),
        reason: None,
        fix_task: None,
    })
}

/// A CI fix task of `run`'s `stage`: never a sample, but it fails the stage's plan
/// tasks on quality.
fn ci_fix(run: &str, stage: u16, origin: TaskOrigin) -> HistoryLine {
    let mut r = record(run, 90, 1_790_400_000, 5, 60, 2);
    r.task_id = "ci-1".into();
    r.record_id = format!("{run}/ci-1");
    r.origin = origin;
    r.stage = stage;
    r.max_rung = 2;
    HistoryLine::Task(r)
}

#[test]
fn thresholds_m_is_dropped_when_not_above_s() {
    // 30 M tasks of 41..=70 lines: p90 is 67, rounded up to 70.
    let lines: Vec<HistoryLine> = (0..30u32)
        .map(|i| {
            let mut r = record(
                &format!("m{}", i / 10),
                i,
                100 + u64::from(i),
                60,
                1500,
                41 + i,
            );
            r.final_size = proto::Size::M;
            r.planned_size = proto::Size::M;
            HistoryLine::Task(r)
        })
        .collect();
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let p = proposals(&lines, &file, &cfg);
    assert_eq!(p.len(), 1, "{p:?}");
    assert_eq!(p[0].id, "thresholds.m");
    assert_eq!(
        p[0].text,
        "M line threshold 100 → 70 (p90 of 30 merged M tasks)"
    );
    assert_eq!(
        p[0].change,
        TuningChange::Threshold {
            class: "m".into(),
            lines: 70
        }
    );
    // Not above an applied S threshold of 70: dropped.
    let mut applied = TuningFile {
        thresholds: Some(SizeThresholds {
            s_lines: 70,
            m_lines: 100,
        }),
        ..TuningFile::default()
    };
    assert!(ids(&lines, &applied, &cfg).is_empty());
    applied.thresholds = Some(SizeThresholds {
        s_lines: 65,
        m_lines: 100,
    });
    assert_eq!(ids(&lines, &applied, &cfg), ["thresholds.m"]);
}

#[test]
fn route_down_needs_no_escalation_and_no_quality_failure() {
    let base = fixture_records("quality");
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let p = proposals(&base, &file, &cfg);
    let down: Vec<_> = p.iter().filter(|p| p.id == "route.s").collect();
    assert_eq!(down.len(), 1, "{p:?}");
    assert_eq!(
        down[0].change,
        TuningChange::Route {
            class: "s".into(),
            route: route(Strength::Fast, Effort::Medium)
        }
    );
    assert_eq!(
        down[0].text,
        "S route standard/low → fast/medium (none of 30 S tasks reached rung 2 or higher or failed on quality)"
    );

    let with = |extra: Vec<HistoryLine>| {
        let mut lines = base.clone();
        lines.extend(extra);
        lines
    };
    let variants = [
        (
            "a revert in effect",
            with(vec![revert(1, "q1", Some("t0"), "m0")]),
        ),
        ("a run's revert", with(vec![revert(1, "q3", None, "acc")])),
        ("a bisect culprit", with(vec![bisect("q2", "t3")])),
        ("a CI fix task", with(vec![ci_fix("q3", 1, TaskOrigin::Ci)])),
        (
            "a review fix task",
            with(vec![ci_fix("q1", 1, TaskOrigin::Review)]),
        ),
    ];
    for (what, lines) in &variants {
        assert!(
            !ids(lines, &file, &cfg).contains(&"route.s".to_string()),
            "{what} holds the class"
        );
    }
    // Evidence elsewhere does not: another stage's fix task, a bisect without the
    // task, a bisect fix task.
    for (what, lines) in [
        ("another stage", with(vec![ci_fix("q3", 2, TaskOrigin::Ci)])),
        ("another culprit", with(vec![bisect("q2", "t77")])),
        (
            "a bisect fix",
            with(vec![ci_fix("q3", 1, TaskOrigin::Bisect)]),
        ),
    ] {
        assert!(
            ids(&lines, &file, &cfg).contains(&"route.s".to_string()),
            "{what} must not hold the class"
        );
    }
    // A revert reverted again gives the proposal back; a third revert holds it again.
    let twice = with(vec![
        revert(1, "q1", Some("t0"), "m0"),
        revert(2, "q1", Some("t0"), "rev1"),
    ]);
    assert!(ids(&twice, &file, &cfg).contains(&"route.s".to_string()));
    let mut thrice = twice.clone();
    thrice.push(revert(3, "q1", Some("t0"), "rev2"));
    assert!(!ids(&thrice, &file, &cfg).contains(&"route.s".to_string()));

    // The evidence, record by record.
    let t = config::Tuning::default();
    let lines = &variants[2].1;
    let failed: Vec<&str> = route_samples(lines, SizeClass::S, &t, super::tests::S_AT)
        .into_iter()
        .filter(|r| failed_on_quality(r, lines))
        .map(|r| r.record_id.as_str())
        .collect();
    assert_eq!(failed, ["q2/t3"]);
    let lines = &variants[3].1;
    let failed = route_samples(lines, SizeClass::S, &t, super::tests::S_AT)
        .into_iter()
        .filter(|r| failed_on_quality(r, lines))
        .count();
    assert_eq!(failed, 10, "every plan task of q3's stage 1");

    // A single escalation holds it too.
    let mut escalated = base.clone();
    if let HistoryLine::Task(r) = &mut escalated[7] {
        r.max_rung = 2;
    }
    assert!(!ids(&escalated, &file, &cfg).contains(&"route.s".to_string()));
}

#[test]
fn route_ladders_stop_at_their_ends() {
    assert_eq!(
        S_ROUTE_LADDER,
        [
            route(Strength::Fast, Effort::Low),
            route(Strength::Fast, Effort::Medium),
            route(Strength::Standard, Effort::Low),
            route(Strength::Standard, Effort::Medium),
        ]
    );
    assert_eq!(
        M_ROUTE_LADDER,
        [
            route(Strength::Standard, Effort::Medium),
            route(Strength::Standard, Effort::High),
            route(Strength::Frontier, Effort::Medium),
            route(Strength::Frontier, Effort::High),
        ]
    );
    let cfg = config::Orchestrator::default();
    let at = |route: ClassRoute| {
        let mut file = TuningFile::default();
        file.routes.insert("s".into(), route);
        file
    };
    // All quiet at the bottom of S, run there: no step down (only its threshold).
    let quiet = fixture_records("quality");
    let bottom = S_ROUTE_LADDER[0];
    assert_eq!(
        ids(&ran_on(&quiet, bottom), &at(bottom), &cfg),
        ["thresholds.s"]
    );
    // Escalating at the top of S, run there: no step up.
    let busy = fixture_records("refit");
    let top = S_ROUTE_LADDER[3];
    assert_eq!(ids(&ran_on(&busy, top), &at(top), &cfg), ["thresholds.s"]);
    // A route outside the ladder (a hand edit) proposes nothing.
    let outside = route(Strength::Frontier, Effort::High);
    let on_outside = ran_on(&busy, outside);
    assert_eq!(ids(&on_outside, &at(outside), &cfg), ["thresholds.s"]);
    // From an applied step, the next one once samples ran on it (whole-branch review
    // B, I2); the samples of the step before are no evidence about it.
    let middle = S_ROUTE_LADDER[1];
    let p = proposals(&ran_on(&busy, middle), &at(middle), &cfg);
    assert_eq!(p[1].proposed, "standard/low");
    assert_eq!(p[1].current, "fast/medium");
    assert_eq!(ids(&busy, &at(middle), &cfg), ["thresholds.s"]);
}

/// `lines` with every record's first worker on `route` (the class default there).
fn ran_on(lines: &[HistoryLine], route: ClassRoute) -> Vec<HistoryLine> {
    let mut lines = lines.to_vec();
    for line in &mut lines {
        if let HistoryLine::Task(r) = line {
            for d in &mut r.routing_decisions {
                (d.chosen.strength, d.chosen.effort) = (route.strength, route.effort);
            }
        }
    }
    lines
}

#[test]
fn a_listed_class_gets_no_route_proposal() {
    let lines = fixture_records("refit");
    let mut cfg = config::Orchestrator::default();
    cfg.tuning.routes.s = RouteList {
        candidates: vec![Candidate {
            runtime: Runtime::Claude,
            model: "claude-sonnet-5".into(),
            effort: None,
        }],
        pick: Pick::First,
    };
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    assert_eq!(ids(&lines, &file, &cfg), ["thresholds.s"]);
    let r = report(&lines, &file, &cfg, Path::new("/tmp/t/tuning.toml"));
    assert_eq!(
        r.classes[0].route,
        "list (config): claude/claude-sonnet-5 low"
    );
    assert_eq!(r.classes[1].route, "standard/medium");
    assert_eq!(r.classes[2].route, "frontier/high");
    let text = super::super::refit_render::render(&r);
    assert!(
        text.contains(
            "  * derived from another class's median\n  route S: list (config): claude/claude-sonnet-5 low\ntuning proposals:\n"
        ),
        "{text}"
    );

    // The orchestrator's list is reported by its first candidate (ruling RH-5).
    cfg.tuning.routes.orchestrator = RouteList {
        candidates: vec![
            Candidate {
                runtime: Runtime::Codex,
                model: "gpt-6.1-sol".into(),
                effort: Some(Effort::High),
            },
            Candidate {
                runtime: Runtime::Claude,
                model: "claude-opus-5-5".into(),
                effort: None,
            },
        ],
        pick: Pick::First,
    };
    let r = report(&lines, &file, &cfg, Path::new("/tmp/t/tuning.toml"));
    assert_eq!(
        r.orchestrator_list.as_deref(),
        Some("codex/gpt-6.1-sol high")
    );
}

#[test]
fn dismissed_is_not_proposed_until_the_value_changes() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let current = proposals(&lines, &file, &cfg);
    let dismissed = dismiss(&file, &current, &["thresholds.s".to_string()]).unwrap();
    assert_eq!(
        dismissed.dismissed.get("thresholds.s").map(String::as_str),
        Some("35")
    );
    assert_eq!(ids(&lines, &dismissed, &cfg), ["route.s"]);
    // History moves the proposal to 40: it is proposed again.
    let mut more = lines.clone();
    for line in &mut more {
        if let HistoryLine::Task(r) = line
            && let Some(d) = r.diff.as_mut()
        {
            d.added += 5;
        }
    }
    assert_eq!(ids(&more, &dismissed, &cfg), ["thresholds.s", "route.s"]);
    assert_eq!(proposals(&more, &dismissed, &cfg)[0].proposed, "40");
    // A dismissed value is not current, so it cannot be applied.
    let current = proposals(&lines, &dismissed, &cfg);
    assert!(apply(&dismissed, &current, &["thresholds.s".to_string()]).is_err());
}

#[test]
fn apply_writes_only_the_named_change() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let (file, _) = refit(&lines, &TuningFile::default(), &cfg, NOW);
    let current = proposals(&lines, &file, &cfg);
    let applied = apply(&file, &current, &["route.s".to_string()]).unwrap();
    let mut expected = file.clone();
    expected
        .routes
        .insert("s".into(), route(Strength::Standard, Effort::Medium));
    assert_eq!(applied, expected);
    let applied = apply(&file, &current, &["thresholds.s".to_string()]).unwrap();
    let mut expected = file.clone();
    expected.thresholds = Some(SizeThresholds {
        s_lines: 35,
        m_lines: 100,
    });
    assert_eq!(applied, expected);
    // Once applied, neither is proposed again.
    let both = apply(
        &file,
        &current,
        &["thresholds.s".to_string(), "route.s".to_string()],
    )
    .unwrap();
    assert!(proposals(&lines, &both, &cfg).is_empty());
}

#[test]
fn an_unknown_id_is_refused_whole() {
    let lines = fixture_records("refit");
    let cfg = config::Orchestrator::default();
    let file = TuningFile::default();
    let current = proposals(&lines, &file, &cfg);
    let named = ["route.s".to_string(), "route.m".to_string()];
    let refusal = "no current proposal route.m; run anthrex run stats to see the proposals";
    assert_eq!(apply(&file, &current, &named), Err(refusal.to_string()));
    assert_eq!(dismiss(&file, &current, &named), Err(refusal.to_string()));
}

/// Whole-branch review B, I2: a route proposal is evidence about the route its samples
/// ran on. Once `--apply` moves M up a step, the same history proposes nothing more for
/// M; S walks down one step the same way, and no further. A sample whose first worker
/// took an explicit route is never evidence about the class default.
#[test]
fn an_applied_route_step_is_not_proposed_again_on_the_same_history() {
    let cfg = config::Orchestrator::default();
    let m: Vec<HistoryLine> = (0..30u32)
        .map(|j| {
            let mut r =
                super::tests::sized_m(record(&format!("m{}", j / 10), j, 100, 60, 1500, 50), false);
            r.max_rung = if j < 12 { 2 } else { 0 };
            HistoryLine::Task(r)
        })
        .collect();
    let file = TuningFile::default();
    let current = proposals(&m, &file, &cfg);
    let up: Vec<&str> = current.iter().map(|p| p.id.as_str()).collect();
    assert!(up.contains(&"route.m"), "{current:?}");
    let applied = apply(&file, &current, &["route.m".to_string()]).unwrap();
    assert_eq!(applied.routes["m"], M_ROUTE_LADDER[1]);
    assert!(!ids(&m, &applied, &cfg).contains(&"route.m".to_string()));

    let quiet = fixture_records("quality");
    let current = proposals(&quiet, &file, &cfg);
    let down = current
        .iter()
        .find(|p| p.id == "route.s")
        .expect("S steps down");
    assert_eq!(down.proposed, "fast/medium");
    let applied = apply(&file, &current, &["route.s".to_string()]).unwrap();
    assert!(!ids(&quiet, &applied, &cfg).contains(&"route.s".to_string()));

    // The planner's own routes: no sample, so no proposal.
    let explicit: Vec<HistoryLine> = (quiet.into_iter())
        .map(|mut line| {
            if let HistoryLine::Task(r) = &mut line {
                r.routing_decisions[0].source = "explicit_task".into();
            }
            line
        })
        .collect();
    assert!(!ids(&explicit, &file, &cfg).contains(&"route.s".to_string()));
}
