//! Milestone 9.1 task M9.1.6: decisions 20, 21 and 25, the steps of a tier job.

use super::steps::plan;
use super::*;

const SLOW: &str = "package(anthrex)";
const TIMING: &str = "test(/timing/)";
const GATE: &str = "'not (package(anthrex)) and not (test(/timing/))'";
const GATE_TIMING: &str = "'(test(/timing/)) and not (package(anthrex))'";

fn modules(names: &[&str]) -> Affected {
    Affected::Modules(names.iter().map(|x| x.to_string()).collect())
}

fn step(kind: StepKind, command: &str, exclusive: bool, key: &str) -> Step {
    Step {
        kind,
        command: command.to_string(),
        exclusive,
        affected_key: key.to_string(),
        per_module: false,
    }
}

/// A step a module command made (milestone 9.14 decision 26).
fn module(kind: StepKind, command: &str, exclusive: bool, key: &str) -> Step {
    Step {
        per_module: true,
        ..step(kind, command, exclusive, key)
    }
}

fn build() -> Step {
    step(StepKind::Build, "cargo build --workspace", false, "-")
}

fn profile() -> TierProfile {
    TierProfile {
        build_check: Some("cargo build --workspace".into()),
        module_test: Some("cargo nextest run -p {module} {filter:-E %}".into()),
        slow_tests: Some(SLOW.into()),
        timing_tests: Some(TIMING.into()),
        ..TierProfile::default()
    }
}

#[test]
fn gate_plan_for_modules_is_build_tests_timing() {
    for tier in [1, 2] {
        let got = plan(tier, &modules(&["b", "a"]), &profile(), Some("cargo test"));
        assert_eq!(got.scope, Scope::Gate);
        assert_eq!(got.affected, modules(&["a", "b"]));
        assert_eq!(
            got.steps,
            vec![
                build(),
                module(
                    StepKind::Tests,
                    &format!("cargo nextest run -p 'a' -E {GATE}"),
                    false,
                    "a,b"
                ),
                module(
                    StepKind::Tests,
                    &format!("cargo nextest run -p 'b' -E {GATE}"),
                    false,
                    "a,b"
                ),
                module(
                    StepKind::Timing,
                    &format!("cargo nextest run -p 'a' -E {GATE_TIMING}"),
                    true,
                    "a,b"
                ),
                module(
                    StepKind::Timing,
                    &format!("cargo nextest run -p 'b' -E {GATE_TIMING}"),
                    true,
                    "a,b"
                ),
            ],
            "tier {tier}"
        );
    }
    // Without timing_tests there is no timing step, and the filter keeps only its
    // slow clause.
    let untimed = TierProfile {
        timing_tests: None,
        ..profile()
    };
    assert_eq!(
        plan(1, &modules(&["a"]), &untimed, None).steps,
        vec![
            build(),
            module(
                StepKind::Tests,
                "cargo nextest run -p 'a' -E 'not (package(anthrex))'",
                false,
                "a"
            ),
        ]
    );
    // No module command (decision 20): `check` runs in place of the tests, with the
    // gate filter, and the timing step runs it with the timing filter.
    let no_module = TierProfile {
        module_test: None,
        ..profile()
    };
    assert_eq!(
        plan(
            1,
            &modules(&["a"]),
            &no_module,
            Some("sh check.sh {filter:-E %}")
        )
        .steps,
        vec![
            build(),
            step(
                StepKind::Tests,
                &format!("sh check.sh -E {GATE}"),
                false,
                "a"
            ),
            step(
                StepKind::Timing,
                &format!("sh check.sh -E {GATE_TIMING}"),
                true,
                "a"
            ),
        ]
    );
}

#[test]
fn gate_plan_prefers_module_tests_over_module_test() {
    let both = TierProfile {
        module_tests: Some("cargo nextest run {modules:-p %} {filter:-E %}".into()),
        ..profile()
    };
    assert_eq!(
        plan(2, &modules(&["b", "a"]), &both, Some("cargo test")).steps,
        vec![
            build(),
            module(
                StepKind::Tests,
                &format!("cargo nextest run -p 'a' -p 'b' -E {GATE}"),
                false,
                "a,b"
            ),
            module(
                StepKind::Timing,
                &format!("cargo nextest run -p 'a' -p 'b' -E {GATE_TIMING}"),
                true,
                "a,b"
            ),
        ]
    );
}

#[test]
fn full_plan_shards_and_timing() {
    let check = "cargo nextest run --partition count:{shard}/{shards} {filter:-E %}";
    let sharded = TierProfile {
        full_shards: 3,
        ..profile()
    };
    let full = Affected::Full("tier 3".into());
    let got = plan(3, &full, &sharded, Some(check));
    assert_eq!(got.scope, Scope::Full);
    let shard = |k: u8| {
        step(
            StepKind::Shard { k, of: 3 },
            &format!("cargo nextest run --partition count:{k}/3 -E 'not (test(/timing/))'"),
            false,
            "full:tier 3",
        )
    };
    assert_eq!(
        got.steps,
        vec![
            shard(1),
            shard(2),
            shard(3),
            step(
                StepKind::Timing,
                "cargo nextest run --partition count:1/1 -E '(test(/timing/))'",
                true,
                "full:tier 3"
            ),
        ]
    );
    // One shard: a single tests step; tier 3 never runs build_check or module tests,
    // whatever the affected set.
    assert_eq!(
        plan(
            3,
            &modules(&["a"]),
            &profile(),
            Some("cargo test {filter:-- %}")
        )
        .steps,
        vec![
            step(
                StepKind::Tests,
                "cargo test -- 'not (test(/timing/))'",
                false,
                "a"
            ),
            step(
                StepKind::Timing,
                "cargo test -- '(test(/timing/))'",
                true,
                "a"
            ),
        ]
    );
    // No check: tier 3 has nothing to run (the run is unverified, decision 20).
    assert_eq!(plan(3, &full, &profile(), None).steps, vec![]);
}

#[test]
fn no_build_check_plans_check_alone() {
    let no_build = TierProfile {
        build_check: None,
        slow_tests: None,
        timing_tests: None,
        ..profile()
    };
    let check = step(StepKind::Tests, "sh check.sh", false, "a");
    assert_eq!(
        plan(1, &modules(&["a"]), &no_build, Some("sh check.sh")).steps,
        vec![check]
    );
    // A docs-only change and a full one run `check` too.
    assert_eq!(
        plan(2, &modules(&[]), &no_build, Some("sh check.sh")).steps,
        vec![step(StepKind::Tests, "sh check.sh", false, "")]
    );
    let full = Affected::Full("hub file: x".into());
    assert_eq!(
        plan(1, &full, &no_build, Some("sh check.sh")).steps,
        vec![step(
            StepKind::Tests,
            "sh check.sh",
            false,
            "full:hub file: x"
        )]
    );
    // `Affected::Full` never runs build_check, even when it is set.
    assert_eq!(
        plan(1, &full, &profile(), Some("sh check.sh {filter:-E %}")).steps,
        vec![
            step(
                StepKind::Tests,
                &format!("sh check.sh -E {GATE}"),
                false,
                "full:hub file: x"
            ),
            step(
                StepKind::Timing,
                &format!("sh check.sh -E {GATE_TIMING}"),
                true,
                "full:hub file: x"
            ),
        ]
    );
    // `{shard}` and `{shards}` in a gate's `check` run the whole suite as one shard.
    assert_eq!(
        plan(1, &full, &no_build, Some("sh check.sh {shard}/{shards}")).steps,
        vec![step(
            StepKind::Tests,
            "sh check.sh 1/1",
            false,
            "full:hub file: x"
        )]
    );
}

#[test]
fn check_without_filter_is_exclusive_when_timing_is_set() {
    let full = Affected::Full("full trigger: Cargo.lock".into());
    let key = "full:full trigger: Cargo.lock";
    // It runs the timing tests and cannot leave them out: exclusive as a whole, and
    // no separate timing step.
    for tier in [1, 3] {
        assert_eq!(
            plan(tier, &full, &profile(), Some("cargo test")).steps,
            vec![step(StepKind::Tests, "cargo test", true, key)],
            "tier {tier}"
        );
    }
    // In place of module tests, too.
    let no_module = TierProfile {
        module_test: None,
        ..profile()
    };
    assert_eq!(
        plan(1, &modules(&["a"]), &no_module, Some("cargo test")).steps,
        vec![build(), step(StepKind::Tests, "cargo test", true, "a")]
    );
    // Without timing_tests, it is not exclusive.
    let untimed = TierProfile {
        timing_tests: None,
        ..profile()
    };
    assert_eq!(
        plan(1, &full, &untimed, Some("cargo test")).steps,
        vec![step(StepKind::Tests, "cargo test", false, key)]
    );
}

#[test]
fn module_steps_are_per_module_and_check_steps_are_not() {
    // Milestone 9.14 decision 26 (ruling R5): only a step a module command made, and
    // its timing step, keys on the inputs id.
    let both = TierProfile {
        module_tests: Some("cargo nextest run {modules:-p %} {filter:-E %}".into()),
        ..profile()
    };
    for (what, tiers) in [("module_test", profile()), ("module_tests", both)] {
        let steps = plan(1, &modules(&["a", "b"]), &tiers, Some("cargo test")).steps;
        assert_eq!(steps[0].kind, StepKind::Build, "{what}");
        assert!(
            !steps[0].per_module,
            "{what}: the build step keeps the tree id"
        );
        assert!(steps.len() > 2, "{what}: {steps:?}");
        for step in &steps[1..] {
            assert!(matches!(step.kind, StepKind::Tests | StepKind::Timing));
            assert!(step.per_module, "{what}: {step:?}");
        }
    }
    // A `check` standing in for module tests (no module command), its timing step
    // included, and every Scope::Full step keep the tree id.
    let no_module = TierProfile {
        module_test: None,
        ..profile()
    };
    let check = Some("sh check.sh {filter:-E %} {shard}/{shards}");
    let stand_in = plan(1, &modules(&["a"]), &no_module, check).steps;
    assert_eq!(stand_in.len(), 3, "{stand_in:?}");
    let sharded = TierProfile {
        full_shards: 2,
        ..profile()
    };
    let full = plan(3, &Affected::Full("tier 3".into()), &sharded, check).steps;
    assert_eq!(full.len(), 3, "{full:?}");
    let gate_full = plan(1, &Affected::Full("hub file: x".into()), &profile(), check).steps;
    for step in stand_in.iter().chain(&full).chain(&gate_full) {
        assert!(!step.per_module, "{step:?}");
    }
}
