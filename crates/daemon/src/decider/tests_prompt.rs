//! Decision 17's prompts: exact texts, omitted sections, the cutting order, determinism.

use super::prompt::{CUT_MARKER, PROMPT_MAX_BYTES, render};
use super::*;
use proto::Size;

/// The triage prompt M8b.1's probe sent to the real `claude` (the fixture
/// `claude-2.1.280-decider-triage.meta.json`'s `command`), byte for byte.
const TRIAGE_GOLDEN: &str = "[anthrex decider] triage v1\nYou label a coding goal for an orchestration engine. Answer with one JSON object that matches the schema, and nothing else.\nkinds: every kind the goal needs. code changes behaviour; docs changes documentation, comments or configuration nothing executes; research investigates and reports without changing code; review reviews an existing branch or commit range.\nscale: single when one task of size S or M does the whole goal; plan when it needs 2 to 12 tasks; large when it needs more, or two or more separate areas that each need several tasks.\nSize S: one file, no interface change, about 20 changed lines or fewer. Size M: 1 to 3 files inside one module, about 100 changed lines or fewer. Anything bigger is not single.\nWhen scale is single, give task: a short title; a brief a worker can follow without asking anything; acceptance criteria; the paths it owns, as globs relative to the repository root and as narrow as possible; its size; whether it changes an interface other code uses; its test mode (tdd for any change in behaviour, check for behaviour-preserving work already covered by tests, none for docs) with a one-line reason unless tdd; and for tdd the name of the test to write. Otherwise task is null.\n\nGoal:\nFix the typo \"recieve\" in README.md.\n\nTracked files (2 of 2):\nREADME.md\nsrc/main.rs";

fn triage(goal: &str) -> TriageInput {
    TriageInput {
        thresholds: Default::default(),
        goal: goal.into(),
        profile_summary: String::new(),
        report_summary: None,
        report_files: vec![],
        files: vec![],
        files_total: 0,
        planner_task_cap: 12,
    }
}

fn size_task(id: &str) -> SizeCheckTask {
    SizeCheckTask {
        id: id.into(),
        title: format!("Title {id}"),
        brief: format!("Brief {id}"),
        acceptance: vec!["it works".into(), "tests pass".into()],
        owns: vec!["src/a.rs".into(), "src/b/**".into()],
        deps: vec![],
        size: Size::S,
        interface_change: false,
        hub: false,
    }
}

fn evidence(id: &str, summary: &str) -> Evidence {
    Evidence {
        id: id.into(),
        summary: summary.into(),
        files: vec!["src/a.rs".into()],
        modules: vec!["src/**".into()],
        interfaces: vec!["fn a()".into()],
    }
}

fn size_check(tasks: Vec<SizeCheckTask>, evidence: Vec<Evidence>) -> SizeCheckInput {
    SizeCheckInput {
        thresholds: Default::default(),
        tasks,
        evidence_refs: vec![],
        evidence,
        modules: vec![],
        hub: vec![],
    }
}

fn check_summary(tail: &str) -> CheckSummaryInput {
    CheckSummaryInput {
        task_id: "t1".into(),
        command: "cargo test".into(),
        code: Some(101),
        timed_out: false,
        tail: tail.into(),
    }
}

fn every_kind() -> Vec<DeciderRequest> {
    vec![
        DeciderRequest::Triage(triage("Fix it")),
        DeciderRequest::SizeCheck(size_check(vec![size_task("t1")], vec![])),
        DeciderRequest::CheckSummary(check_summary("error: boom")),
        DeciderRequest::BlockedReason(BlockedReasonInput {
            task_id: "t1".into(),
            title: "Add a retry".into(),
            reason: "cc is missing".into(),
        }),
    ]
}

#[test]
fn prompts_start_with_the_kind_line() {
    for request in every_kind() {
        let prompt = render(&request);
        let first = prompt.lines().next().unwrap();
        assert_eq!(
            first,
            format!("[anthrex decider] {} v1", request.kind().label())
        );
    }
}

#[test]
fn triage_prompt_is_exact_for_a_fixed_input() {
    let request = DeciderRequest::Triage(TriageInput {
        files: vec!["README.md".into(), "src/main.rs".into()],
        files_total: 2,
        ..triage("Fix the typo \"recieve\" in README.md.")
    });
    assert_eq!(render(&request), TRIAGE_GOLDEN);
}

/// M9.3: the plan scale's upper bound is `[orchestrator] planner_task_cap`, which the
/// driver passes in; with the default 12 the prompt is the golden one above.
#[test]
fn triage_prompt_uses_planner_task_cap() {
    let prompt = render(&DeciderRequest::Triage(TriageInput {
        planner_task_cap: 7,
        ..triage("G")
    }));
    assert!(
        prompt.contains("plan when it needs 2 to 7 tasks;"),
        "{prompt}"
    );
    assert!(!prompt.contains("2 to 12 tasks"), "{prompt}");
    let default = render(&DeciderRequest::Triage(triage("G")));
    assert!(
        default.contains("plan when it needs 2 to 12 tasks;"),
        "{default}"
    );
}

#[test]
fn sections_with_empty_inputs_are_omitted() {
    // Every triage section filled: each appears once, in order.
    let full = DeciderRequest::Triage(TriageInput {
        thresholds: Default::default(),
        goal: "G".into(),
        profile_summary: "languages: rust".into(),
        report_summary: Some("It is a Rust workspace.".into()),
        report_files: vec!["Cargo.toml".into(), "src/lib.rs".into()],
        files: vec!["Cargo.toml".into()],
        files_total: 9,
        planner_task_cap: 12,
    });
    let head = TRIAGE_GOLDEN.split("\n\nGoal:").next().unwrap();
    assert_eq!(
        render(&full),
        format!(
            "{head}\n\nGoal:\nG\n\nRepository profile:\nlanguages: rust\n\nOnboarding scout report:\nIt is a Rust workspace.\nFiles it named: Cargo.toml, src/lib.rs\n\nTracked files (1 of 9):\nCargo.toml"
        )
    );
    // Nothing but the goal: no other heading.
    let bare = render(&DeciderRequest::Triage(triage("G")));
    assert_eq!(bare, format!("{head}\n\nGoal:\nG"));
    // A report with files but no summary keeps only its file line.
    let files_only = render(&DeciderRequest::Triage(TriageInput {
        report_files: vec!["a".into()],
        ..triage("G")
    }));
    assert!(files_only.ends_with("\n\nOnboarding scout report:\nFiles it named: a"));

    // Size check: no Modules/Hub lines, no depends-on line, no evidence section.
    let sc = render(&DeciderRequest::SizeCheck(size_check(
        vec![size_task("t1")],
        vec![],
    )));
    assert!(sc.ends_with(
        "\n\nTasks:\n- id t1, stated size S, hub no, interface change no\n  title: Title t1\n  owns: src/a.rs, src/b/**\n  acceptance: it works; tests pass\n  brief: Brief t1"
    ), "{sc}");
    assert!(!sc.contains("Modules:") && !sc.contains("Hub:") && !sc.contains("depends on"));
    assert!(!sc.contains("Scout evidence"));
    // …and with them.
    let mut input = size_check(vec![size_task("t2")], vec![evidence("s1", "Summary.")]);
    input.modules = vec!["crates/*".into()];
    input.hub = vec!["crates/proto/**".into()];
    input.tasks[0].deps = vec!["t1".into()];
    input.tasks[0].size = Size::M;
    input.tasks[0].hub = true;
    input.tasks[0].interface_change = true;
    let sc = render(&DeciderRequest::SizeCheck(input));
    assert!(sc.contains("\n\nModules: crates/*\nHub: crates/proto/**\n\nTasks:\n- id t2, stated size M, hub yes, interface change yes\n  title: Title t2\n  owns: src/a.rs, src/b/**\n  depends on: t1\n"), "{sc}");
    assert!(sc.ends_with("\n\nScout evidence:\n## s1\nSummary.\nFiles: src/a.rs\nModules: src/**\nInterfaces: fn a()"), "{sc}");

    // Check summary with an empty tail: no output lines.
    let cs = render(&DeciderRequest::CheckSummary(check_summary("")));
    assert!(
        cs.ends_with("\n\nCommand: cargo test\nResult: exit 101"),
        "{cs}"
    );
    let cs = render(&DeciderRequest::CheckSummary(CheckSummaryInput {
        timed_out: true,
        code: None,
        ..check_summary("a\nb")
    }));
    assert!(
        cs.ends_with("\n\nCommand: cargo test\nResult: timed out\nOutput (last 2 lines):\na\nb"),
        "{cs}"
    );
}

#[test]
fn blocked_reason_prompt_is_exact() {
    // The prompt M8b.1's `blocked_reason` probe sent (claude-2.1.280-decider.meta.json).
    let prompt = render(&DeciderRequest::BlockedReason(BlockedReasonInput {
        task_id: "t1".into(),
        title: "Add a retry to the fetch helper".into(),
        reason: "cargo test fails before my change: the linker `cc` is not installed in this checkout's environment.".into(),
    }));
    assert_eq!(
        prompt,
        "[anthrex decider] blocked_reason v1\nA coding agent stopped its task and gave the reason below without saying what kind of block it is. Classify it. question: it needs an answer or a decision about the task. mis_sized: the task is bigger than one task, or needs changes outside the paths it owns. environment: a tool, command, permission, dependency or setup is broken or missing. Answer with one JSON object that matches the schema, and nothing else.\n\nTask t1: Add a retry to the fetch helper\nReason:\ncargo test fails before my change: the linker `cc` is not installed in this checkout's environment."
    );
}

fn many_paths(n: usize) -> Vec<String> {
    (0..n)
        .map(|i| format!("crates/some-module/src/deeply/nested/file_{i:05}.rs"))
        .collect()
}

#[test]
fn oversized_inputs_are_cut_in_order_with_the_marker() {
    // 5000 tracked paths: they are cut first, from the end; the report is untouched.
    let paths = many_paths(5000);
    let request = DeciderRequest::Triage(TriageInput {
        thresholds: Default::default(),
        goal: "G".into(),
        profile_summary: String::new(),
        report_summary: Some("S".into()),
        report_files: vec!["a".into(), "b".into()],
        files: paths.clone(),
        files_total: 5000,
        planner_task_cap: 12,
    });
    let prompt = render(&request);
    assert!(prompt.len() <= PROMPT_MAX_BYTES, "{}", prompt.len());
    assert!(prompt.contains("\n\nOnboarding scout report:\nS\nFiles it named: a, b\n\n"));
    let tracked = prompt.split("\n\nTracked files (").nth(1).unwrap();
    let (header, body) = tracked.split_once("):\n").unwrap();
    let lines: Vec<&str> = body.split('\n').collect();
    assert_eq!(*lines.last().unwrap(), CUT_MARKER);
    let kept = &lines[..lines.len() - 1];
    assert!(kept.len() > 1000 && kept.len() < 5000, "{}", kept.len());
    assert_eq!(kept, &paths[..kept.len()]);
    assert_eq!(header, format!("{} of 5000", kept.len()));
    // Kept as many as fit: one more path would pass the limit.
    assert!(prompt.len() + paths[kept.len()].len() + 1 > PROMPT_MAX_BYTES);

    // A huge report summary: every tracked file and every report file is cut first,
    // then the summary is cut to 8000 characters; then the refill (ruling M2) brings
    // back the report's files whole, then as many tracked paths as fit.
    let request = DeciderRequest::Triage(TriageInput {
        thresholds: Default::default(),
        goal: "G".into(),
        profile_summary: String::new(),
        report_summary: Some("x".repeat(200_000)),
        report_files: vec!["a".into(), "b".into()],
        files: paths.clone(),
        files_total: 5000,
        planner_task_cap: 12,
    });
    let prompt = render(&request);
    assert!(prompt.len() <= PROMPT_MAX_BYTES);
    assert!(prompt.contains(&format!(
        "\n\nOnboarding scout report:\n{}{CUT_MARKER}\nFiles it named: a, b\n\nTracked files (",
        "x".repeat(8000)
    )));
    let tracked = prompt.split("\n\nTracked files (").nth(1).unwrap();
    let (header, body) = tracked.split_once("):\n").unwrap();
    let lines: Vec<&str> = body.split('\n').collect();
    assert_eq!(*lines.last().unwrap(), CUT_MARKER);
    let kept = &lines[..lines.len() - 1];
    assert!(kept.len() > 1000, "{}", kept.len());
    assert_eq!(kept, &paths[..kept.len()]);
    assert_eq!(header, format!("{} of 5000", kept.len()));
    assert!(prompt.len() + paths[kept.len()].len() + 1 > PROMPT_MAX_BYTES);

    // Size check: evidence summaries to 4000 characters first, then reports from the
    // last, then briefs to 2000 characters.
    let big = "e".repeat(60_000);
    let mut tasks: Vec<SizeCheckTask> = (1..=3).map(|i| size_task(&format!("t{i}"))).collect();
    let input = size_check(
        tasks.clone(),
        vec![
            evidence("s1", &big),
            evidence("s2", &big),
            evidence("s3", &big),
        ],
    );
    let prompt = render(&DeciderRequest::SizeCheck(input));
    assert!(prompt.len() <= PROMPT_MAX_BYTES);
    let cut_summary = format!("{}{CUT_MARKER}", "e".repeat(4000));
    assert_eq!(prompt.matches(&cut_summary).count(), 3);
    for task in &mut tasks {
        task.brief = "b".repeat(60_000);
    }
    // Briefs that alone overflow: every report is dropped before the briefs are cut,
    // and the refill (ruling M2) then brings every report back, since they fit.
    let input = size_check(
        tasks.clone(),
        vec![evidence("s1", &big), evidence("s2", &big)],
    );
    let prompt = render(&DeciderRequest::SizeCheck(input));
    assert!(prompt.len() <= PROMPT_MAX_BYTES, "{}", prompt.len());
    let cut_brief = format!("  brief: {}{CUT_MARKER}", "b".repeat(2000));
    assert_eq!(prompt.matches(&cut_brief).count(), 3);
    assert!(prompt.contains(&format!("\n\nScout evidence:\n## s1\n{cut_summary}\n")));
    assert!(prompt.contains(&format!("\n\n## s2\n{cut_summary}\n")));
    assert!(prompt.ends_with("Interfaces: fn a()"), "evidence was cut");
    // Reports that do not all fit even at 4000 characters: dropped from the last,
    // and the refill keeps the same count (no more fit).
    for task in &mut tasks {
        task.brief = "Brief".into();
    }
    let reports: Vec<Evidence> = (1..=40).map(|i| evidence(&format!("s{i}"), &big)).collect();
    let prompt = render(&DeciderRequest::SizeCheck(size_check(tasks, reports)));
    assert!(prompt.len() <= PROMPT_MAX_BYTES);
    assert!(prompt.ends_with(&format!("\n\n{CUT_MARKER}")));
    let kept = prompt.matches("\n## s").count();
    assert!(kept > 5 && kept < 40, "{kept}");
    for i in 1..=kept {
        assert!(prompt.contains(&format!("\n## s{i}\n")), "s{i}");
    }
    assert!(!prompt.contains(&format!("\n## s{}\n", kept + 1)));

    // Check summary: the tail from its start, keeping its end.
    let tail: String = (0..20_000).map(|i| format!("line {i}\n")).collect();
    let tail = tail.trim_end();
    let prompt = render(&DeciderRequest::CheckSummary(check_summary(tail)));
    assert!(prompt.len() <= PROMPT_MAX_BYTES);
    assert!(prompt.ends_with("\nline 19999"));
    let output = prompt.split("Output (last ").nth(1).unwrap();
    let (n, body) = output.split_once(" lines):\n").unwrap();
    let (first, rest) = body.split_once('\n').unwrap();
    assert_eq!(first, CUT_MARKER);
    assert_eq!(rest.lines().count().to_string(), n);
    assert!(rest.starts_with("line "));

    // A tail that is one oversized line keeps that line's end (review I1), ASCII and
    // with multi-byte characters. Commands of 1 to 3 bytes move the cut point across
    // every byte of a 3-byte character, so it must move to a character boundary.
    let tails = [
        format!("{}END", "z".repeat(200_000)),
        format!("{}世END", "世".repeat(70_000)),
        format!("{}END", "é".repeat(100_001)),
    ];
    for (tail, command) in tails
        .iter()
        .flat_map(|t| ["c", "cc", "ccc"].map(|c| (t, c)))
    {
        let input = CheckSummaryInput {
            command: command.into(),
            ..check_summary(tail)
        };
        let prompt = render(&DeciderRequest::CheckSummary(input));
        assert!(prompt.len() <= PROMPT_MAX_BYTES, "{}", prompt.len());
        assert!(
            prompt.len() + 4 > PROMPT_MAX_BYTES,
            "room left: {}",
            prompt.len()
        );
        assert_eq!(prompt.matches(CUT_MARKER).count(), 1);
        assert!(prompt.ends_with("END"));
        let body = prompt.split(&format!("{CUT_MARKER}\n")).nth(1).unwrap();
        assert!(tail.ends_with(body));
        assert!(prompt.contains("Output (last 1 lines):\n"));
    }
}

#[test]
fn an_enormous_fixed_input_is_clamped_with_the_marker() {
    // No ordered cut applies to a goal, command, title or reason: the last-resort
    // clamp keeps the prompt's head, on a character boundary, and ends in the marker.
    let wide = |pad: usize| format!("{}{}", "x".repeat(pad), "世".repeat(100_000));
    for pad in 0..3 {
        let requests = [
            DeciderRequest::Triage(triage(&wide(pad))),
            DeciderRequest::CheckSummary(CheckSummaryInput {
                command: wide(pad),
                ..check_summary("error: boom")
            }),
            DeciderRequest::BlockedReason(BlockedReasonInput {
                task_id: "t1".into(),
                title: wide(pad),
                reason: "r".into(),
            }),
            DeciderRequest::BlockedReason(BlockedReasonInput {
                task_id: "t1".into(),
                title: "T".into(),
                reason: wide(pad),
            }),
        ];
        for request in requests {
            let prompt = render(&request);
            let kind = request.kind().label();
            assert!(prompt.len() <= PROMPT_MAX_BYTES, "{kind}: {}", prompt.len());
            assert!(
                prompt.len() + 4 > PROMPT_MAX_BYTES,
                "{kind}: {}",
                prompt.len()
            );
            assert!(prompt.ends_with(&format!("世\n{CUT_MARKER}")), "{kind}");
            assert_eq!(prompt.matches(CUT_MARKER).count(), 1, "{kind}");
            assert!(prompt.starts_with(&format!("[anthrex decider] {kind} v1\n")));
        }
    }
}

#[test]
fn prompt_is_deterministic() {
    let mut requests = every_kind();
    requests.push(DeciderRequest::Triage(TriageInput {
        files: many_paths(5000),
        files_total: 5000,
        planner_task_cap: 12,
        report_summary: Some("y".repeat(50_000)),
        ..triage("G")
    }));
    for request in &requests {
        assert_eq!(render(request), render(&request.clone()));
    }
}
