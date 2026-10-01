//! `[orchestrator]`'s ranges: an out-of-range value falls back with a problem, and every
//! range is the `proto::settings` constant the Settings screen reads. Split out of
//! `orchestrator_tests.rs` to keep that file under the 600-line rule.

use super::*;

#[test]
fn out_of_range_values_fall_back_with_problems() {
    let (config, problems) = parse(
        r#"
[orchestrator]
max_writers = 0
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.max_writers".to_string()])
    );
    assert_eq!(config.orchestrator.max_writers, 3);
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.max_writers: must be between 1 and 8 (using 3)"
    );

    let (config, problems) = parse(
        r#"
[orchestrator]
max_writers = 9
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.max_writers".to_string()])
    );
    assert_eq!(config.orchestrator.max_writers, 3);

    let (config, problems) = parse(
        r#"
[orchestrator]
stall_after_secs = 1
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.stall_after_secs".to_string()])
    );
    assert_eq!(config.orchestrator.stall_after_secs, 600);

    let (config, problems) = parse(
        r#"
[orchestrator]
git_timeout_secs = 4
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.git_timeout_secs".to_string()])
    );
    assert_eq!(config.orchestrator.git_timeout_secs, 60);

    let (config, problems) = parse(
        r#"
[orchestrator]
worker_codex_sandbox = "yolo"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.worker_codex_sandbox".to_string()])
    );
    assert_eq!(config.orchestrator.worker_codex_sandbox, "workspace-write");

    let (config, problems) = parse(
        r#"
[orchestrator]
default_runtime = "shell"
"#,
    );
    assert_eq!(
        keys(&problems),
        HashSet::from(["orchestrator.default_runtime".to_string()])
    );
    assert_eq!(config.orchestrator.default_runtime, proto::Runtime::Claude);
}

/// Milestone 9.0.6 decision 23: the parser's five ranges are the `proto::settings`
/// constants the Settings screen also reads, so one range governs both. Each edge is
/// accepted and the value one past it is refused, with the constants' own bounds in the
/// message.
#[test]
fn ranges_are_the_proto_constants() {
    use proto::settings::{
        BUDGET_MIN, MAX_BOUNCES_RANGE, MAX_READERS_RANGE, MAX_WRITERS_RANGE, STALL_AFTER_SECS_RANGE,
    };
    let u8s = [
        ("max_writers", MAX_WRITERS_RANGE),
        ("max_readers", MAX_READERS_RANGE),
        ("max_bounces", MAX_BOUNCES_RANGE),
    ];
    for (key, range) in u8s {
        for edge in [*range.start(), *range.end()] {
            let (_, problems) = parse(&format!("[orchestrator]\n{key} = {edge}\n"));
            assert!(problems.is_empty(), "{key} = {edge}: {problems:?}");
        }
        for out in [i64::from(*range.start()) - 1, i64::from(*range.end()) + 1] {
            let (_, problems) = parse(&format!("[orchestrator]\n{key} = {out}\n"));
            assert_eq!(
                problems[0].message,
                format!("must be between {} and {}", range.start(), range.end())
            );
        }
    }
    let stall = STALL_AFTER_SECS_RANGE;
    for edge in [*stall.start(), *stall.end()] {
        let (_, problems) = parse(&format!("[orchestrator]\nstall_after_secs = {edge}\n"));
        assert!(problems.is_empty(), "{problems:?}");
    }
    for out in [*stall.start() - 1, *stall.end() + 1] {
        let (_, problems) = parse(&format!("[orchestrator]\nstall_after_secs = {out}\n"));
        assert_eq!(
            problems[0].message,
            format!("must be between {} and {}", stall.start(), stall.end())
        );
    }
    let (_, problems) = parse(&format!(
        "[orchestrator.budget.s]\ntool_calls = {}\n",
        BUDGET_MIN - 1
    ));
    assert_eq!(problems[0].message, "must be at least 1");
    let (_, problems) = parse(&format!(
        "[orchestrator.budget.s]\nminutes = {BUDGET_MIN}\n"
    ));
    assert!(problems.is_empty());
}
