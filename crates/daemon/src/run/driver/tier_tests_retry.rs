//! Milestone 9.1 task M9.1.9: decision 33's retry, as ruling C-7 amends it, on the
//! rig of `tier_tests.rs` (split out for the 600-line rule).

use super::*;

#[tokio::test]
async fn flaky_test_passes_on_retry_and_is_recorded() {
    let rig = Rig::new();
    rig.state("out-b-1", &libtest(&["tests::flaky_one"]));
    rig.state("code-b-1", "101");
    let outcome = rig.tier(1, &rig.spec(tiered(), None)).await;
    assert!(outcome.ok, "{outcome:?}");
    let b = &outcome.steps[1];
    assert!(b.ok && b.retried, "{b:?}");
    assert_eq!(b.flaky, ["tests::flaky_one"]);
    assert!(b.failing.is_empty());
    let log = rig.log();
    let ones: Vec<&String> = log.iter().filter(|l| l.starts_with("one ")).collect();
    assert_eq!(ones.len(), 1, "{log:#?}");
    assert!(ones[0].starts_with("one tests::flaky_one "), "{log:#?}");
    // Ruling C-7: the whole step ran once more after the names passed.
    let b_runs: Vec<&String> = log.iter().filter(|l| l.starts_with("test b ")).collect();
    assert_eq!(b_runs.len(), 2, "{log:#?}");
}

#[tokio::test]
async fn a_by_name_pass_is_not_green_when_the_whole_step_still_fails() {
    // Ruling C-7: the names read are a strict subset of what failed (the step also
    // fails for a reason no test name carries); the by-name retry passes, the whole
    // step does not, so the step stays red and nothing is flaky.
    let rig = Rig::new();
    let named = format!(
        "{}error: the lint step failed too\n",
        libtest(&["tests::a"])
    );
    for n in 1..=2 {
        rig.state(&format!("out-b-{n}"), &named);
        rig.state(&format!("code-b-{n}"), "1");
    }
    let outcome = rig.tier(1, &rig.spec(tiered(), None)).await;
    assert!(!outcome.ok);
    let b = outcome.steps.last().unwrap();
    assert_eq!(b.kind, StepKind::Tests);
    assert!(!b.ok && b.retried, "{b:?}");
    assert!(b.flaky.is_empty(), "{b:?}");
    assert_eq!(b.failing, ["tests::a"]);
    let log = rig.log();
    assert_eq!(
        log.iter()
            .filter(|l| l.starts_with("one tests::a "))
            .count(),
        1
    );
    assert_eq!(
        log.iter().filter(|l| l.starts_with("test b ")).count(),
        2,
        "{log:#?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("test c ")),
        "stops at the red step"
    );
}

#[tokio::test]
async fn retry_reruns_the_whole_step_without_single_test_or_with_many_names() {
    // No single_test: the whole step again.
    let rig = Rig::new();
    rig.state("out-b-1", &libtest(&["tests::x"]));
    rig.state("code-b-1", "101");
    let mut spec = rig.spec(tiered(), None);
    spec.single_test = None;
    let outcome = rig.tier(1, &spec).await;
    assert!(outcome.ok, "{outcome:?}");
    assert_eq!(outcome.steps[1].flaky, ["tests::x"]);
    let log = rig.log();
    assert!(!log.iter().any(|l| l.starts_with("one ")), "{log:#?}");
    assert_eq!(log.iter().filter(|l| l.starts_with("test b ")).count(), 2);

    // Eleven names (more than RETRY_NAMES_MAX): the whole step again, no single_test.
    let rig = Rig::new();
    let eleven: Vec<String> = (1..=11).map(|n| format!("tests::t{n}")).collect();
    let refs: Vec<&str> = eleven.iter().map(String::as_str).collect();
    rig.state("out-b-1", &libtest(&refs));
    rig.state("code-b-1", "101");
    let outcome = rig.tier(1, &rig.spec(tiered(), None)).await;
    assert!(outcome.ok, "{outcome:?}");
    assert!(outcome.steps[1].retried);
    assert_eq!(outcome.steps[1].flaky, eleven);
    let log = rig.log();
    assert!(!log.iter().any(|l| l.starts_with("one ")), "{log:#?}");
    assert_eq!(log.iter().filter(|l| l.starts_with("test b ")).count(), 2);
}

#[tokio::test]
async fn build_and_timed_out_steps_are_not_retried() {
    let rig = Rig::new();
    rig.state("build-fail", "");
    let outcome = rig.tier(1, &rig.spec(tiered(), None)).await;
    assert!(!outcome.ok);
    assert_eq!(outcome.steps.len(), 1);
    assert_eq!(outcome.steps[0].kind, StepKind::Build);
    assert!(!outcome.steps[0].retried);
    assert!(outcome.tail.contains("build broke"), "{}", outcome.tail);
    let log = rig.log();
    assert_eq!(log.len(), 1, "one build, no test: {log:#?}");

    let rig = Rig::new();
    rig.state("sleep-b-1", "");
    let mut spec = rig.spec(tiered(), None);
    spec.timeout_secs = 1;
    let outcome = rig.tier(1, &spec).await;
    assert!(!outcome.ok);
    let b = outcome.steps.last().unwrap();
    assert!(b.timed_out && !b.retried, "{b:?}");
    let log = rig.log();
    assert_eq!(
        log.iter().filter(|l| l.starts_with("test b ")).count(),
        1,
        "{log:#?}"
    );
}

#[tokio::test]
async fn red_step_reports_failing_names_and_a_tail() {
    let rig = Rig::new();
    let out = libtest(&["tests::one", "tests::two"]);
    rig.state("out-b-1", &out);
    rig.state("code-b-1", "101");
    rig.state("one-fail", "");
    let outcome = rig.tier(1, &rig.spec(tiered(), None)).await;
    assert!(!outcome.ok);
    let b = outcome.steps.last().unwrap();
    assert_eq!(b.failing, ["tests::one", "tests::two"]);
    assert!(b.retried && b.flaky.is_empty());
    // M8a's tail: the red run's output as `exec` keeps it, which `summary` shortens.
    let expected: Vec<&str> = out.trim_end_matches('\n').lines().collect();
    assert_eq!(outcome.tail, expected.join("\n"));
    assert!(summary(&outcome.tail).ends_with("finished in 0.00s"));
}
