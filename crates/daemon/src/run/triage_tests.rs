//! M8b.14: triage routing, the fast-path plan and its messages (decisions 22–24).

use proto::{
    DeciderSource, PlanTask, RepoProfile, RouteSpec, RunPath, Scale, Size, TaskKind, TestMode,
    TokenUsage, TriageInfo,
};

use super::*;
use crate::decider::fallback::{OFF_REASON, fallback_decision};
use crate::decider::{
    DeciderAnswer, DeciderKind, DeciderRequest, Decision, TriageInput, TriageTask,
};
use crate::run::plan::{BuildContext, build_run};
use crate::run::test_support::preflight;

fn triage_task(size: Size, owns: &[&str]) -> TriageTask {
    TriageTask {
        title: "Fix the reset link".into(),
        brief: "The reset link points at the wrong host.".into(),
        acceptance: vec!["the link uses the configured host".into()],
        owns: owns.iter().map(|o| o.to_string()).collect(),
        size,
        interface_change: false,
        test_mode: TestMode::Check,
        test_mode_reason: Some("a one-line config fix".into()),
        test_to_write: None,
    }
}

fn decision(kinds: &[TaskKind], scale: Scale, task: Option<TriageTask>) -> Decision {
    Decision {
        kind: DeciderKind::Triage,
        answer: DeciderAnswer::Triage(crate::decider::TriageAnswer {
            kinds: kinds.to_vec(),
            scale,
            reason: "one small change in one module".into(),
            task,
        }),
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: Some(TokenUsage {
            input: 900,
            output: 80,
            cache_read: 0,
            cache_write: 0,
        }),
        secs: 3,
    }
}

fn single(size: Size, owns: &[&str]) -> Decision {
    decision(
        &[TaskKind::Code],
        Scale::Single,
        Some(triage_task(size, owns)),
    )
}

fn plan_reason(route: TriageRoute) -> String {
    match route {
        TriageRoute::Plan { reason } => reason,
        other => panic!("not the plan path: {other:?}"),
    }
}

#[test]
fn single_code_goal_takes_the_fast_path() {
    let TriageRoute::Fast(task) = route(&single(Size::S, &["crates/auth/src/link.rs"]), true)
    else {
        panic!("not the fast path");
    };
    assert_eq!(
        *task,
        PlanTask {
            id: "t1".into(),
            title: "Fix the reset link".into(),
            epic: None,
            kind: TaskKind::Code,
            size: Size::S,
            interface_change: false,
            test_mode: Some(TestMode::Check),
            test_mode_reason: Some("a one-line config fix".into()),
            owns: vec!["crates/auth/src/link.rs".into()],
            deps: vec![],
            priority: 0,
            brief: "The reset link points at the wrong host.".into(),
            acceptance: vec!["the link uses the configured host".into()],
            test_to_write: None,
            scout_refs: vec![],
            route: RouteSpec::default(),
            budget: None,
            review_target: None,
            stage: 1,
            atomic: false,
            atomic_reason: None,
            addresses: Vec::new(),
            race: false,
            pair: false,
        }
    );
    // A docs goal is fast too, with the kind carried.
    let docs = decision(
        &[TaskKind::Docs],
        Scale::Single,
        Some(triage_task(Size::M, &["docs/reset.md"])),
    );
    let TriageRoute::Fast(task) = route(&docs, true) else {
        panic!("docs is fast");
    };
    assert_eq!((task.kind, task.size), (TaskKind::Docs, Size::M));
    let r = route(&single(Size::S, &["a.rs"]), true);
    let info = info(&single(Size::S, &["a.rs"]), &r, 77);
    assert_eq!(
        info,
        TriageInfo {
            kinds: vec![TaskKind::Code],
            scale: Scale::Single,
            path: RunPath::Fast,
            reason: "one small change in one module".into(),
            source: DeciderSource::Decider,
            fallback_reason: None,
            at: 77,
        }
    );
}

#[test]
fn plan_and_large_scales_route_with_the_reason() {
    let plan = decision(&[TaskKind::Code], Scale::Plan, None);
    assert_eq!(
        route(&plan, true),
        TriageRoute::Plan {
            reason: "one small change in one module".into()
        }
    );
    let large = decision(&[TaskKind::Code, TaskKind::Docs], Scale::Large, None);
    let r = route(&large, true);
    assert_eq!(
        r,
        TriageRoute::Large {
            reason: "one small change in one module".into()
        }
    );
    let info = info(&large, &r, 5);
    assert_eq!(
        (info.path, info.scale, info.kinds),
        (
            RunPath::Large,
            Scale::Large,
            vec![TaskKind::Code, TaskKind::Docs]
        )
    );
}

#[test]
fn mixed_or_research_kinds_are_not_fast() {
    let task = || Some(triage_task(Size::S, &["a.rs"]));
    let cases = [
        (vec![TaskKind::Code, TaskKind::Docs], "code,docs"),
        (vec![TaskKind::Research], "research"),
        (vec![TaskKind::Review], "review"),
        (vec![TaskKind::Code, TaskKind::Code], "code,code"),
    ];
    for (kinds, label) in cases {
        let d = decision(&kinds, Scale::Single, task());
        assert_eq!(
            plan_reason(route(&d, true)),
            format!("a single-task goal of kinds {label} is not a fast-path goal")
        );
    }
}

#[test]
fn fast_path_disabled_routes_plan() {
    let r = route(&single(Size::S, &["a.rs"]), false);
    assert_eq!(
        plan_reason(r),
        "the fast path is disabled ([orchestrator] fast_path = false)"
    );
}

#[test]
fn fallback_triage_routes_plan_with_the_reason() {
    let request = DeciderRequest::Triage(TriageInput {
        goal: "g".into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec![],
        files_total: 0,
        planner_task_cap: 12,
    });
    let d = fallback_decision(&request, OFF_REASON.into());
    let r = route(&d, true);
    let reason = "triage fell back (deciders are off); without a decider the path is plan";
    assert_eq!(plan_reason(r.clone()), reason);
    // The fallback wins over a disabled fast path: it is checked first.
    assert_eq!(plan_reason(route(&d, false)), reason);
    let info = info(&d, &r, 9);
    assert_eq!(
        info,
        TriageInfo {
            kinds: vec![TaskKind::Code],
            scale: Scale::Plan,
            path: RunPath::Plan,
            reason: reason.into(),
            source: DeciderSource::Fallback,
            fallback_reason: Some("deciders are off".into()),
            at: 9,
        }
    );
}

/// A stored profile, as `choose_profile` would find it.
fn stored() -> RepoProfile {
    RepoProfile {
        modules: vec!["crates/*".into()],
        hub: vec!["crates/proto/**".into()],
        source: vec!["crates/*/src/**".into()],
        check: Some("cargo test".into()),
        single_test: Some("cargo test -- --exact {test}".into()),
        test_passed: Some("test {test} ... ok".into()),
        ..RepoProfile::default()
    }
}

/// `decision`'s fast plan through M8a's `build_run`, as `build_plan` does it (`yes`).
fn build_fast(decision: &Decision) -> Result<crate::run::model::Run, Vec<PlanError>> {
    let TriageRoute::Fast(task) = route(decision, true) else {
        panic!("not the fast path");
    };
    let plan = fast_plan("Fix the reset link", *task, stored().spec());
    let config = config::Orchestrator::default();
    build_run(
        plan,
        preflight(),
        BuildContext {
            id: "fix-the-reset-link-3f9a".into(),
            wt_dir: "/tmp/wt".into(),
            data_dir: "/tmp/data/runs/fix-the-reset-link-3f9a".into(),
            config: &config,
            testing: &config::Testing::default(),
            now: 1_000,
            yes: true,
            delivery: &config::Delivery::default(),
        },
    )
}

#[test]
fn fast_plan_builds_a_valid_run() {
    let d = single(Size::S, &["crates/auth/src/link.rs"]);
    let r = route(&d, true);
    let mut run = check_fast(build_fast(&d)).expect("a valid fast run");
    assert_eq!(run.goal, "Fix the reset link");
    assert_eq!(run.tasks.len(), 1);
    let task = &run.tasks[0];
    assert_eq!(
        (task.id(), task.size, task.hub, task.test_mode),
        ("t1", Size::S, false, TestMode::Check)
    );
    assert_eq!(run.profile.check.as_deref(), Some("cargo test"));
    assert_eq!(run.approved_by.as_deref(), Some("--yes"));
    let triage = info(&d, &r, 1_000);
    mark_fast(&mut run, triage.clone(), d.usage);
    assert_eq!(run.approved_by.as_deref(), Some("fast path"));
    assert_eq!(run.path, Some(RunPath::Fast));
    assert_eq!(run.triage, Some(triage));
    assert_eq!(run.triage_usage.input, 900);
}

#[test]
fn a_hub_task_falls_back_to_plan() {
    let d = single(Size::M, &["crates/proto/src/wire.rs"]);
    assert_eq!(
        check_fast(build_fast(&d)).unwrap_err(),
        "the fast path does not apply: task t1 touches a hub file"
    );
    let r = route(&d, true);
    let info = not_fast(info(&d, &r, 1), check_fast(build_fast(&d)).unwrap_err());
    assert_eq!(info.path, RunPath::Plan);
    assert_eq!(
        info.reason,
        "the fast path does not apply: task t1 touches a hub file"
    );
}

#[test]
fn an_l_task_falls_back_to_plan() {
    // Two modules with an interface change: M8a's rule 7.2.2 raises it to L, and
    // `build_run` refuses an L task (rule 7.2.4); the first error is the reason.
    let mut two = triage_task(Size::S, &["crates/auth/src/a.rs", "crates/mail/src/b.rs"]);
    two.interface_change = true;
    let d = decision(&[TaskKind::Code], Scale::Single, Some(two));
    let built = build_fast(&d);
    let first = built.as_ref().unwrap_err()[0].to_string();
    assert!(first.contains("L tasks are never executed"), "{first}");
    assert_eq!(
        check_fast(built).unwrap_err(),
        format!("the fast path does not apply: {first}")
    );
    // An L size on a run that built anyway (a later rule change) is still refused.
    let mut run = check_fast(build_fast(&single(Size::S, &["crates/auth/src/a.rs"]))).unwrap();
    run.tasks[0].size = Size::L;
    assert_eq!(
        check_fast(Ok(run)).unwrap_err(),
        "the fast path does not apply: task t1 is L"
    );
}

#[test]
fn messages_are_exact() {
    let d = single(Size::S, &["crates/auth/src/link.rs"]);
    let r = route(&d, true);
    let triage = info(&d, &r, 1);
    let run = check_fast(build_fast(&d)).unwrap();
    assert_eq!(
        started_message(&triage, "fix-the-reset-link-3f9a", &run.tasks[0]),
        "triage: code/single (decider)\n\
         fast path: one task, no plan gate\n  \
         t1  S  check  Fix the reset link\n\
         watch with: anthrex run status fix-the-reset-link-3f9a"
    );
    // Milestone 9 decision 26: the planned and large paths start a planned run, so
    // M8b's refusal text is gone (`run::orch::contract::planned_message`).
}

#[test]
fn tracked_files_are_sorted_capped_and_counted() {
    let listing = "b.rs\0a.rs\0c/d.rs\0";
    assert_eq!(
        tracked_files(listing),
        (vec!["a.rs".into(), "b.rs".into(), "c/d.rs".into()], 3)
    );
    let many: String = (0..2000).map(|i| format!("f{i:05}\0")).collect();
    let (kept, total) = tracked_files(&many);
    assert_eq!((kept.len(), total), (FILES_MAX, 2000));
    assert_eq!(kept[0], "f00000");
    // 100-byte paths: 48 KiB holds 486 of them with their newlines.
    let long: String = (0..600).map(|i| format!("{i:0>100}\0")).collect();
    let (kept, total) = tracked_files(&long);
    assert_eq!((kept.len(), total), (FILES_BYTES / 101, 600));
    assert_eq!(goal_input(&"é".repeat(5000)).chars().count(), GOAL_CHARS);
}

/// Review I1: the one check the driver, `build_plan` and the engine share. A fast-path
/// run has exactly one task, and it is neither hub nor L.
#[test]
fn fast_refusal_needs_one_task_neither_hub_nor_l() {
    let ok = check_fast(build_fast(&single(Size::S, &["crates/auth/src/link.rs"]))).unwrap();
    assert_eq!(fast_refusal(&ok), None);
    let hub = build_fast(&single(Size::M, &["crates/proto/src/wire.rs"])).unwrap();
    assert_eq!(
        fast_refusal(&hub).as_deref(),
        Some("the fast path does not apply: task t1 touches a hub file")
    );
    let mut l = ok.clone();
    l.tasks[0].size = Size::L;
    assert_eq!(
        fast_refusal(&l).as_deref(),
        Some("the fast path does not apply: task t1 is L")
    );
    let mut two = ok.clone();
    two.tasks.push(ok.tasks[0].clone());
    let many = "the fast path does not apply: a fast-path run has exactly one task, not 2";
    assert_eq!(fast_refusal(&two).as_deref(), Some(many));
    assert_eq!(check_fast(Ok(two)).unwrap_err(), many);
    let mut none = ok.clone();
    none.tasks.clear();
    assert_eq!(
        fast_refusal(&none).as_deref(),
        Some("the fast path does not apply: a fast-path run has exactly one task, not 0")
    );
}

/// Whole-branch review I1: a fast-path task whose `owns` names or covers a protected
/// agent-config path takes the planned path, where the user sees the grant. Literal
/// entries match decision 56's patterns as its done gate does (ignoring case, `<dir>/**`
/// covering `<dir>`); a glob counts when it covers a tracked protected file or a
/// protected path at the repository's root.
#[test]
fn a_fast_task_owning_a_protected_file_takes_the_plan_path() {
    let ok = check_fast(build_fast(&single(Size::S, &["crates/auth/src/link.rs"]))).unwrap();
    let why = |path: &str| {
        format!("the fast path does not apply: task t1 owns a protected file ({path})")
    };
    let owning = |owns: &[&str], tracked: &[&str], extra: &[&str]| {
        let mut run = ok.clone();
        run.tasks[0].spec.owns = owns.iter().map(|o| o.to_string()).collect();
        run.protected_files = tracked.iter().map(|t| t.to_string()).collect();
        run.profile
            .protected
            .extend(extra.iter().map(|e| e.to_string()));
        fast_refusal(&run)
    };
    // (owns, tracked protected files, profile extras, the path the reason names)
    type Case<'a> = (&'a [&'a str], &'a [&'a str], &'a [&'a str], &'a str);
    let refused: [Case; 15] = [
        (&["AGENTS.md"], &[], &[], "AGENTS.md"),
        (&["src/a.rs", "CLAUDE.md"], &[], &[], "CLAUDE.md"),
        (&["docs/AGENTS.md"], &[], &[], "docs/AGENTS.md"),
        (&["agents.md"], &[], &[], "agents.md"),
        (
            &[".claude/settings.json"],
            &[],
            &[],
            ".claude/settings.json",
        ),
        (&[".claude"], &[], &[], ".claude"),
        (&[".mcp.json"], &[], &[], ".mcp.json"),
        (&[".codex/config.toml"], &[], &[], ".codex/config.toml"),
        (&["**"], &[], &[], ".claude/**"),
        (&[".claude/*"], &[], &[], ".claude/**"),
        (&["*.md"], &[], &[], "**/CLAUDE.md"),
        (
            &["crates/auth/**"],
            &["crates/auth/AGENTS.md"],
            &[],
            "crates/auth/AGENTS.md",
        ),
        (
            &["docs/agents.txt"],
            &[],
            &["docs/agents.txt"],
            "docs/agents.txt",
        ),
        (&["docs/**"], &[], &["docs/*.txt"], "docs/*.txt"),
        // Re-review N1: `ſ` opens as `s` on a case-insensitive file system.
        (
            &["src/a.rs", "AGENT\u{17f}.md"],
            &[],
            &[],
            "AGENT\u{17f}.md",
        ),
    ];
    for (owns, tracked, extra, path) in refused {
        assert_eq!(
            owning(owns, tracked, extra),
            Some(why(path)),
            "{owns:?} {tracked:?} {extra:?}"
        );
    }
    let allowed: [(&[&str], &[&str], &[&str]); 6] = [
        (&["crates/auth/src/link.rs"], &["AGENTS.md"], &[]),
        (
            &["crates/auth/**"],
            &["AGENTS.md", ".claude/settings.json"],
            &[],
        ),
        (&["*.rs"], &[], &[]),
        (&["docs/*.md"], &[], &[]),
        (&["src/claude/x.rs", "claude.md.bak"], &[], &[]),
        (&["docs/**"], &[], &["notes/*.txt"]),
    ];
    for (owns, tracked, extra) in allowed {
        assert_eq!(
            owning(owns, tracked, extra),
            None,
            "{owns:?} {tracked:?} {extra:?}"
        );
    }
    // Through `build_run`, as `build_plan` builds it: the run's own protected list.
    assert_eq!(
        check_fast(build_fast(&single(Size::S, &["AGENTS.md"]))).unwrap_err(),
        why("AGENTS.md")
    );
}

/// Review m2: a blank goal is refused with `build_run`'s own wording.
#[test]
fn a_blank_goal_is_refused_with_build_runs_wording() {
    for goal in ["", "   ", "\n\t "] {
        assert_eq!(
            blank_goal(goal).as_deref(),
            Some("goal: must not be blank"),
            "{goal:?}"
        );
    }
    assert_eq!(blank_goal(" fix the link "), None);
}
