//! Milestone 9.1 task M9.1.5: the tier keys, decisions 5 to 8 and 11.

use proto::{ModuleNames, ProfileSpec, RepoProfile};

use super::command::{Placeholders, filter_expr, substitute};
use super::*;
use crate::run::plan::resolve_profile;
use crate::run::test_support::{EXAMPLE_PLAN, PROFILE, plan_with, run_ok, task_toml};

fn s(items: &[&str]) -> Vec<String> {
    items.iter().map(|x| x.to_string()).collect()
}

/// M8b's stored-profile shape: every M8b key set, no tier key.
fn m8b_stored() -> RepoProfile {
    RepoProfile {
        languages: s(&["rust"]),
        modules: s(&["crates/*"]),
        hub: s(&["crates/proto/**"]),
        source: s(&["crates/*/src/**"]),
        generated: s(&["Cargo.lock"]),
        setup: Some("cargo fetch".into()),
        check: Some("cargo test".into()),
        check_timeout_secs: Some(600),
        single_test: Some("cargo test -- --exact {test}".into()),
        test_passed: Some("test {test} ... ok".into()),
        sample_test: Some("a::b".into()),
        manifests: s(&["Cargo.toml"]),
        env: [("RUST_LOG".to_string(), "info".to_string())].into(),
        ..Default::default()
    }
}

/// Each tier key alone, set to a value that is not what its absence means.
fn each_key_alone() -> Vec<(&'static str, ProfileSpec)> {
    let one = |f: fn(&mut ProfileSpec)| {
        let mut spec = ProfileSpec::default();
        f(&mut spec);
        spec
    };
    vec![
        (
            "build_check",
            one(|p| p.build_check = Some("make build".into())),
        ),
        (
            "module_test",
            one(|p| p.module_test = Some("t {module}".into())),
        ),
        (
            "module_tests",
            one(|p| p.module_tests = Some("t {modules}".into())),
        ),
        (
            "module_graph",
            one(|p| p.module_graph = Some("cargo".into())),
        ),
        (
            "module_names",
            one(|p| p.module_names = Some(ModuleNames::Cargo)),
        ),
        (
            "full_triggers",
            one(|p| p.full_triggers = Some(s(&["Cargo.lock"]))),
        ),
        (
            "slow_tests",
            one(|p| p.slow_tests = Some("test(e2e)".into())),
        ),
        (
            "timing_tests",
            one(|p| p.timing_tests = Some("test(t)".into())),
        ),
        (
            "skip_markers",
            one(|p| p.skip_markers = Some(s(&["#[ignore"]))),
        ),
        (
            "test_paths",
            one(|p| p.test_paths = Some(s(&["**/tests/**"]))),
        ),
        ("full_shards", one(|p| p.full_shards = Some(4))),
        (
            "toolchain_id",
            one(|p| p.toolchain_id = Some("rustc -V".into())),
        ),
    ]
}

#[test]
fn untiered_profile_is_not_tiered() {
    assert!(!TierProfile::from_repo(&m8b_stored()).is_tiered());
    assert!(!TierProfile::from_repo(&RepoProfile::default()).is_tiered());
    assert!(!run_ok(EXAMPLE_PLAN).profile.tiers.is_tiered());
    let m8a = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    ));
    assert!(!m8a.profile.tiers.is_tiered());
    assert_eq!(m8a.profile.tiers, TierProfile::default());

    let keys = each_key_alone();
    assert_eq!(keys.len(), 12);
    for (key, spec) in keys {
        let from_plan = TierProfile::resolve(&spec, &ProfileSpec::default());
        assert!(from_plan.is_tiered(), "{key} from the plan");
        let from_config = TierProfile::resolve(&ProfileSpec::default(), &spec);
        assert!(from_config.is_tiered(), "{key} from the config");
        let stored = RepoProfile {
            build_check: spec.build_check.clone(),
            module_test: spec.module_test.clone(),
            module_tests: spec.module_tests.clone(),
            module_graph: spec.module_graph.clone(),
            module_names: spec.module_names,
            full_triggers: spec.full_triggers.clone().unwrap_or_default(),
            slow_tests: spec.slow_tests.clone(),
            timing_tests: spec.timing_tests.clone(),
            skip_markers: spec.skip_markers.clone().unwrap_or_default(),
            test_paths: spec.test_paths.clone().unwrap_or_default(),
            full_shards: spec.full_shards,
            toolchain_id: spec.toolchain_id.clone(),
            ..m8b_stored()
        };
        assert!(TierProfile::from_repo(&stored).is_tiered(), "{key} stored");
    }
}

fn problems(tiers: TierProfile, check: Option<&str>, modules: &[&str]) -> Vec<String> {
    validate(&tiers, check, &s(modules))
        .into_iter()
        .map(|(key, message)| format!("{key}: {message}"))
        .collect()
}

fn tiered(f: impl FnOnce(&mut TierProfile)) -> TierProfile {
    let mut tiers = TierProfile {
        build_check: Some("make build".into()),
        ..TierProfile::default()
    };
    f(&mut tiers);
    tiers
}

#[test]
fn new_key_validation_messages_are_exact() {
    let modules = ["mods/*"];
    let check = Some("sh check.sh");
    let one = |tiers: TierProfile, check: Option<&str>, modules: &[&str]| -> Vec<String> {
        problems(tiers, check, modules)
    };
    // A valid tiered profile has no problem.
    let good = tiered(|t| {
        t.module_test = Some("sh test.sh {module} {filter:--filter %}".into());
        t.module_tests = Some("sh test.sh {modules:-p %} {filter:-E %}".into());
        t.module_graph = GraphSource::Command("sh graph.sh".into());
        t.slow_tests = Some("e2e".into());
        t.timing_tests = Some("timing".into());
        t.full_shards = 4;
        t.full_triggers = s(&["Cargo.lock", ".github/**"]);
        t.test_paths = s(&["**/tests/**"]);
        t.skip_markers = s(&["#[ignore"]);
        t.toolchain_id = Some("rustc -V".into());
    });
    assert_eq!(
        one(
            good,
            Some("sh check.sh {filter:-E %} --shard {shard}/{shards}"),
            &modules
        ),
        Vec::<String>::new()
    );

    let cases: Vec<(TierProfile, Option<&str>, &[&str], &str)> = vec![
        (
            tiered(|t| t.module_test = Some("sh test.sh".into())),
            check,
            &modules,
            "module_test: must contain {module} exactly once",
        ),
        (
            tiered(|t| t.module_test = Some("sh test.sh {module} {module}".into())),
            check,
            &modules,
            "module_test: must contain {module} exactly once",
        ),
        (
            tiered(|t| t.module_tests = Some("sh test.sh".into())),
            check,
            &modules,
            "module_tests: must contain {modules} or {modules:<template>}",
        ),
        (
            tiered(|t| t.module_test = Some("sh test.sh {module}".into())),
            check,
            &[],
            "module_test: needs modules",
        ),
        (
            tiered(|t| t.module_tests = Some("sh test.sh {modules}".into())),
            check,
            &[],
            "module_tests: needs modules",
        ),
        (
            tiered(|t| t.build_check = Some("make {module}".into())),
            check,
            &modules,
            "build_check: {module} is not allowed here",
        ),
        (
            tiered(|t| t.module_test = Some("t {module} {shard}".into())),
            check,
            &modules,
            "module_test: {shard} is not allowed here",
        ),
        (
            tiered(|t| t.module_tests = Some("t {modules} {module}".into())),
            check,
            &modules,
            "module_tests: {module} is not allowed here",
        ),
        (
            tiered(|_| {}),
            Some("sh check.sh {modules:-p %}"),
            &modules,
            "check: {modules:<template>} is not allowed here",
        ),
        (
            tiered(|t| t.toolchain_id = Some("rustc {filter:-E %}".into())),
            check,
            &modules,
            "toolchain_id: {filter:<template>} is not allowed here",
        ),
        (
            tiered(|t| {
                t.module_names = ModuleNames::Cargo;
                t.module_graph = GraphSource::Command("sh graph.sh".into());
            }),
            check,
            &modules,
            "module_names: cargo needs module_graph = \"cargo\"",
        ),
        (
            tiered(|t| {
                t.slow_tests = Some("e2e".into());
                t.module_test = Some("t {module}".into());
            }),
            check,
            &modules,
            "slow_tests: module_test must contain {filter:<template>} to leave slow tests out",
        ),
        (
            tiered(|t| {
                t.slow_tests = Some("e2e".into());
                t.module_tests = Some("t {modules}".into());
            }),
            check,
            &modules,
            "slow_tests: module_tests must contain {filter:<template>} to leave slow tests out",
        ),
        (
            tiered(|t| {
                t.timing_tests = Some("timing".into());
                t.module_test = Some("t {module}".into());
            }),
            check,
            &modules,
            "timing_tests: module_test must contain {filter:<template>} to leave timing tests out",
        ),
        (
            tiered(|t| t.full_shards = 0),
            check,
            &modules,
            "full_shards: must be between 1 and 16",
        ),
        (
            tiered(|t| t.full_shards = 17),
            check,
            &modules,
            "full_shards: must be between 1 and 16",
        ),
        (
            tiered(|t| t.full_shards = 2),
            Some("sh check.sh --shard {shard}"),
            &modules,
            "full_shards: check must contain {shard} and {shards}",
        ),
        (
            tiered(|t| t.full_triggers = s(&["/abs"])),
            check,
            &modules,
            "full_triggers: /abs must not be absolute",
        ),
        (
            tiered(|t| t.test_paths = s(&["../x/**"])),
            check,
            &modules,
            "test_paths: ../x/** must not contain ..",
        ),
        (
            tiered(|t| t.skip_markers = vec!["x".to_string(); 33]),
            check,
            &modules,
            "skip_markers: at most 32 markers, each 1 to 64 characters",
        ),
        (
            tiered(|t| t.skip_markers = vec!["x".repeat(65)]),
            check,
            &modules,
            "skip_markers: at most 32 markers, each 1 to 64 characters",
        ),
        (
            tiered(|t| t.skip_markers = vec![String::new()]),
            check,
            &modules,
            "skip_markers: at most 32 markers, each 1 to 64 characters",
        ),
        (
            tiered(|t| t.build_check = Some("  ".into())),
            check,
            &modules,
            "build_check: must not be blank",
        ),
        (
            tiered(|t| t.module_graph = GraphSource::Command(String::new())),
            check,
            &modules,
            "module_graph: must not be blank",
        ),
        (
            tiered(|t| t.slow_tests = Some("x".repeat(2001))),
            check,
            &modules,
            "slow_tests: at most 2000 characters",
        ),
        (
            tiered(|t| t.build_check = Some("x".repeat(2001))),
            check,
            &modules,
            "build_check: at most 2000 characters",
        ),
    ];
    for (tiers, check, modules, expected) in cases {
        assert_eq!(one(tiers, check, modules), vec![expected.to_string()]);
    }

    // An untiered profile's check is M8a's: its placeholders are not the tiers'.
    assert_eq!(
        one(TierProfile::default(), Some("echo {shard} {module}"), &[]),
        Vec::<String>::new()
    );
    // A blank stored command is refused; a blank plan one resolves to absent.
    let stored = RepoProfile {
        toolchain_id: Some(String::new()),
        ..RepoProfile::default()
    };
    assert_eq!(
        problems(TierProfile::from_repo(&stored), None, &[]),
        ["toolchain_id: must not be blank"]
    );
    let plan = ProfileSpec {
        toolchain_id: Some(" ".into()),
        ..ProfileSpec::default()
    };
    assert_eq!(
        TierProfile::resolve(&plan, &ProfileSpec::default()).toolchain_id,
        None
    );
}

#[test]
fn a_plan_with_a_bad_tier_key_is_refused_with_the_key() {
    let text = plan_with(
        &format!("{PROFILE}module_test = \"cargo test\"\n"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let errors = crate::run::test_support::errors_of(&text);
    assert_eq!(
        errors,
        vec![crate::run::test_support::err(
            None,
            "profile.module_test",
            "profile",
            "must contain {module} exactly once",
        )]
    );
}

#[test]
fn plan_and_config_profiles_carry_the_new_keys_with_decision_7_precedence() {
    let config = ProfileSpec {
        build_check: Some("config build".into()),
        module_test: Some("config {module}".into()),
        module_graph: Some("cargo".into()),
        full_triggers: Some(s(&["Cargo.lock"])),
        slow_tests: Some("config-slow".into()),
        full_shards: Some(3),
        toolchain_id: Some("rustc -V".into()),
        ..ProfileSpec::default()
    };
    let plan = ProfileSpec {
        build_check: Some("plan build".into()),
        module_graph: Some("sh graph.sh".into()),
        full_triggers: Some(s(&["plan.lock"])),
        slow_tests: Some("  ".into()),
        timing_tests: Some("plan-timing".into()),
        skip_markers: Some(s(&["xit("])),
        test_paths: Some(s(&["**/test_*.py"])),
        module_names: Some(ModuleNames::Dir),
        ..ProfileSpec::default()
    };
    let tiers = resolve_profile(&plan, &config).tiers;
    assert_eq!(
        tiers,
        TierProfile {
            build_check: Some("plan build".into()),
            module_test: Some("config {module}".into()),
            module_tests: None,
            module_graph: GraphSource::Command("sh graph.sh".into()),
            module_names: ModuleNames::Dir,
            full_triggers: s(&["plan.lock"]),
            // A blank plan string switches the configured key off.
            slow_tests: None,
            timing_tests: Some("plan-timing".into()),
            skip_markers: s(&["xit("]),
            test_paths: s(&["**/test_*.py"]),
            full_shards: 3,
            toolchain_id: Some("rustc -V".into()),
        }
    );
    // module_names defaults to cargo only with the cargo graph.
    let cargo = resolve_profile(&ProfileSpec::default(), &config).tiers;
    assert_eq!(cargo.module_graph, GraphSource::Cargo);
    assert_eq!(cargo.module_names, ModuleNames::Cargo);
    let none = ProfileSpec {
        module_graph: Some("none".into()),
        ..ProfileSpec::default()
    };
    let none = resolve_profile(&none, &config).tiers;
    assert_eq!(none.module_graph, GraphSource::None);
    assert_eq!(none.module_names, ModuleNames::Dir);

    // The stored profile's keys reach the run through `spec()`.
    let stored = RepoProfile {
        build_check: Some("stored build".into()),
        test_paths: s(&["tests/**"]),
        ..m8b_stored()
    };
    let tiers = resolve_profile(&stored.spec(), &ProfileSpec::default()).tiers;
    assert_eq!(tiers.build_check.as_deref(), Some("stored build"));
    assert_eq!(tiers.test_paths, s(&["tests/**"]));
}

#[test]
fn profile_hash_changes_with_any_profile_field() {
    let base = resolve_profile(&m8b_stored().spec(), &ProfileSpec::default());
    let hash = profile_hash(&base);
    assert_eq!(hash.len(), 16);
    assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(profile_hash(&base.clone()), hash);

    let mut env = base.clone();
    env.env.insert("RUST_LOG".into(), "debug".into());
    let mut tier = base.clone();
    tier.tiers.build_check = Some("cargo build".into());
    let mut check = base.clone();
    check.check = Some("cargo test --workspace".into());
    let hashes = [
        profile_hash(&env),
        profile_hash(&tier),
        profile_hash(&check),
    ];
    for (i, other) in hashes.iter().enumerate() {
        assert_ne!(*other, hash, "change {i}");
    }

    // `build_run` records it.
    let run = run_ok(EXAMPLE_PLAN);
    assert_eq!(run.profile_hash, profile_hash(&run.profile));
}

#[test]
fn placeholders_used_by_verification_substitute_and_quote() {
    let tiers = TierProfile {
        slow_tests: Some("test(e2e)".into()),
        timing_tests: Some("test(t)".into()),
        ..TierProfile::default()
    };
    let filter = filter_expr(&tiers, Scope::Gate, false);
    assert_eq!(filter.as_deref(), Some("not (test(e2e)) and not (test(t))"));
    let values = Placeholders {
        module: Some("a".into()),
        modules: Some(s(&["b", "a"])),
        filter,
        ..Placeholders::default()
    };
    assert_eq!(
        substitute("t {module} {filter:-E %} 100%% ${HOME} {x}", &values),
        "t 'a' -E 'not (test(e2e)) and not (test(t))' 100%% ${HOME} {x}"
    );
    assert_eq!(
        substitute("t {modules:-p %} {modules}", &values),
        "t -p 'a' -p 'b' 'a' 'b'"
    );
    // Inside a template `%%` is a literal `%`.
    assert_eq!(
        substitute("t {modules:--m=%%%}", &values),
        "t --m=%'a' --m=%'b'"
    );
    // No filter: `{filter:…}` is empty; an unset value keeps its placeholder.
    assert_eq!(
        substitute("t {module} {filter:-E %}", &Placeholders::default()),
        "t {module} "
    );
    assert_eq!(
        filter_expr(&TierProfile::default(), Scope::Gate, false),
        None
    );
}
