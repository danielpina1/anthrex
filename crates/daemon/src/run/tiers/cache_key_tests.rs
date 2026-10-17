//! Milestone 9.1 task M9.1.6: decision 30's cache key.

use super::cache_key::key;
use super::steps::plan;
use super::*;

fn ctx() -> CacheCtx {
    CacheCtx {
        profile_hash: "0123456789abcdef".into(),
        toolchain: "none".into(),
    }
}

fn profile() -> TierProfile {
    TierProfile {
        build_check: Some("cargo build".into()),
        module_tests: Some("cargo test {modules:-p %}".into()),
        ..TierProfile::default()
    }
}

fn modules(names: &[&str]) -> Affected {
    Affected::Modules(names.iter().map(|x| x.to_string()).collect())
}

/// Each step's key for `tree`.
fn keys(tier: u8, affected: &Affected, tree: &str) -> Vec<cache_key::CacheKey> {
    let plan = plan(tier, affected, &profile(), Some("cargo test"));
    plan.steps
        .iter()
        .map(|step| key(tree, plan.scope, &step.command, &step.affected_key, &ctx()))
        .collect()
}

#[test]
fn cache_key_uses_scope_not_tier() {
    let set = modules(&["a", "b"]);
    let tier1 = keys(1, &set, "t1");
    assert_eq!(tier1.len(), 2);
    assert_eq!(tier1, keys(2, &set, "t1"));
    let tests = &tier1[1];
    assert_eq!(tests.tree, "t1");
    assert_eq!(tests.scope, Scope::Gate);
    assert_eq!(tests.command, "cargo test -p 'a' -p 'b'");
    assert_eq!(tests.affected, "a,b");
    assert_eq!(tests.profile_hash, "0123456789abcdef");
    assert_eq!(tests.toolchain, "none");
    // Tier 3's scope is `full`: the same command's key differs.
    let full = keys(3, &set, "t1");
    assert_eq!(full[0].command, "cargo test");
    assert_eq!(full[0].scope, Scope::Full);
    assert_ne!(full[0], key("t1", Scope::Gate, "cargo test", "a,b", &ctx()));
    // Every part counts.
    let base = key("t", Scope::Gate, "c", "a", &ctx());
    let other = CacheCtx {
        toolchain: "unknown".into(),
        ..ctx()
    };
    for changed in [
        key("u", Scope::Gate, "c", "a", &ctx()),
        key("t", Scope::Full, "c", "a", &ctx()),
        key("t", Scope::Gate, "d", "a", &ctx()),
        key("t", Scope::Gate, "c", "b", &ctx()),
        key("t", Scope::Gate, "c", "a", &other),
        key(
            "t",
            Scope::Gate,
            "c",
            "a",
            &CacheCtx {
                profile_hash: "x".into(),
                ..ctx()
            },
        ),
    ] {
        assert_ne!(changed, base);
    }
}

#[test]
fn build_step_key_ignores_the_affected_set() {
    let one = keys(1, &modules(&["a"]), "t1");
    let two = keys(2, &modules(&["a", "b"]), "t1");
    let docs = keys(1, &modules(&[]), "t1");
    assert_eq!(one[0].affected, "-");
    assert_eq!(one[0], two[0]);
    assert_eq!(one[0], docs[0]);
    // The tests steps differ.
    assert_ne!(one[1], two[1]);
}

mod toolchain_env {
    use super::cache_key::{TOOLCHAIN_ENV_VARS, with_env_from};

    fn env(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    fn none(_: &str) -> Option<String> {
        None
    }

    fn part(step: &[(&str, &str)], daemon: &dyn Fn(&str) -> Option<String>) -> String {
        with_env_from("rustc 1", &env(step), daemon, &[])
    }

    /// The part with the daemon's matching variables given as a list (decision 8).
    fn part_with(step: &[(&str, &str)], daemon_vars: &[(&str, &str)]) -> String {
        with_env_from("rustc 1", &env(step), &none, &env(daemon_vars))
    }

    #[test]
    fn rustflags_moves_the_toolchain() {
        let base = part(&[("PATH", "/bin")], &none);
        let step = part(&[("PATH", "/bin"), ("RUSTFLAGS", "-Copt-level=1")], &none);
        assert_ne!(base, step);
        let daemon = |n: &str| (n == "RUSTFLAGS").then(|| "-Copt-level=1".to_string());
        assert_eq!(part(&[("PATH", "/bin")], &daemon), step);
        // The step env's value wins over the daemon's.
        let other = |n: &str| (n == "RUSTFLAGS").then(|| "-Copt-level=3".to_string());
        assert_eq!(
            part(&[("PATH", "/bin"), ("RUSTFLAGS", "-Copt-level=1")], &other),
            step
        );
    }

    #[test]
    fn each_listed_variable_counts() {
        let base = part(&[("PATH", "/bin")], &none);
        let mut seen = vec![base.clone()];
        for name in TOOLCHAIN_ENV_VARS {
            let moved = part(&[("PATH", "/bin"), (name, "x")], &none);
            assert_ne!(moved, base, "{name}");
            assert!(!seen.contains(&moved), "{name} collides");
            seen.push(moved);
        }
    }

    #[test]
    fn without_any_the_part_is_todays() {
        // The old `with_path("rustc 1", "/bin")` output, pinned.
        assert_eq!(
            part(&[("PATH", "/bin")], &none),
            "rustc 1 path:1e38b39ca6823f4b"
        );
    }

    #[test]
    fn slot_variables_and_the_target_dir_never_count() {
        let base = part(&[("PATH", "/bin")], &none);
        for name in [
            "CARGO_BUILD_JOBS",
            "NEXTEST_TEST_THREADS",
            "RUST_TEST_THREADS",
            "ANTHREX_TEST_SLOTS",
            "ANTHREX_TEST_LOAD",
            "CARGO_TARGET_DIR",
            // M9.18.6: `CARGO_TARGET_` counts only with `_RUSTFLAGS` after a triple.
            "CARGO_TARGET_TMPDIR",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "CARGO_TARGET_RUSTFLAGS",
        ] {
            assert_eq!(
                part(&[("PATH", "/bin"), (name, "7")], &none),
                base,
                "{name}"
            );
            assert_eq!(
                part_with(&[("PATH", "/bin")], &[(name, "7")]),
                base,
                "{name}"
            );
        }
    }

    #[test]
    fn a_cargo_profile_variable_moves_the_toolchain() {
        let base = part(&[("PATH", "/bin")], &none);
        let step = part(
            &[("PATH", "/bin"), ("CARGO_PROFILE_DEV_OPT_LEVEL", "1")],
            &none,
        );
        assert_ne!(base, step);
        // From the daemon's list it moves the part the same way.
        assert_eq!(
            part_with(&[("PATH", "/bin")], &[("CARGO_PROFILE_DEV_OPT_LEVEL", "1")]),
            step
        );
        // The step env's value wins over the daemon's.
        assert_eq!(
            part_with(
                &[("PATH", "/bin"), ("CARGO_PROFILE_DEV_OPT_LEVEL", "1")],
                &[("CARGO_PROFILE_DEV_OPT_LEVEL", "3")]
            ),
            step
        );
    }

    #[test]
    fn target_rustflags_build_rustflags_and_incremental_count() {
        let base = part(&[("PATH", "/bin")], &none);
        let mut seen = vec![base.clone()];
        for (name, value) in [
            (
                "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS",
                "-Copt-level=1",
            ),
            ("CARGO_BUILD_RUSTFLAGS", "-Copt-level=1"),
            ("CARGO_INCREMENTAL", "0"),
        ] {
            let moved = part(&[("PATH", "/bin"), (name, value)], &none);
            assert_ne!(moved, base, "{name}");
            assert!(!seen.contains(&moved), "{name} collides");
            assert_eq!(part_with(&[("PATH", "/bin")], &[(name, value)]), moved);
            seen.push(moved);
        }
    }

    #[test]
    fn the_order_of_variables_does_not_matter() {
        let (a, b) = (
            ("CARGO_PROFILE_DEV_DEBUG", "0"),
            ("CARGO_PROFILE_TEST_OPT_LEVEL", "2"),
        );
        let one = part(&[("PATH", "/bin"), a, b], &none);
        assert_eq!(part(&[("PATH", "/bin"), b, a], &none), one);
        assert_eq!(part_with(&[("PATH", "/bin")], &[b, a]), one);
        assert_eq!(part_with(&[("PATH", "/bin"), b], &[a]), one);
    }

    #[test]
    fn the_five_keep_todays_key_and_the_new_ones_follow_sorted() {
        // Ruling C10 (a): two of the five, set in the reverse of their array order,
        // hash in array order, as before M9.18.6 (pinned from that code).
        assert_eq!(
            part(
                &[
                    ("PATH", "/bin"),
                    ("RUSTC_WRAPPER", "sccache"),
                    ("RUSTFLAGS", "-Copt-level=1"),
                ],
                &none
            ),
            "rustc 1 path:1e38b39ca6823f4b env:44a7def3d03c3543"
        );
        // FNV-1a of `RUSTFLAGS=a\nCARGO_INCREMENTAL=0\nCARGO_PROFILE_DEV_DEBUG=0\n`.
        assert_eq!(
            part(
                &[
                    ("PATH", "/bin"),
                    ("CARGO_PROFILE_DEV_DEBUG", "0"),
                    ("RUSTFLAGS", "a"),
                    ("CARGO_INCREMENTAL", "0"),
                ],
                &none
            ),
            "rustc 1 path:1e38b39ca6823f4b env:6f724e2a72bcdf35"
        );
    }
}
