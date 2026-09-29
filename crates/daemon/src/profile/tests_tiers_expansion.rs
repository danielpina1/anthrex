//! M9.1.5 review round 2 (ruling C-3): every command a confirmed profile keeps runs as
//! verification ran it; a module command without a filter slot goes, not the filters;
//! any kept graph keeps the module commands; `module_dirs` agrees with `path_module`.

use std::path::Path;
use std::time::Duration;

use proto::{CommandCheck, ModuleNames, ProfileVerification, RepoProfile};

use super::proposal::apply_verification;
use super::proposal_tiers::{NEEDS_GRAPH, UNTIERED_CHECK, module_command, verified_form};
use super::tests_tiers::{check_of, log, scratch, strings};
use super::verify::run_commands;
use crate::run::globs::path_module;

fn ok(command: &str) -> Option<CommandCheck> {
    Some(CommandCheck {
        command: command.to_string(),
        ok: true,
        code: Some(0),
        timed_out: false,
        secs: 0,
        tail: String::new(),
    })
}

fn drops(dropped: &[proto::DroppedCommand]) -> Vec<(&str, &str)> {
    dropped
        .iter()
        .map(|d| (d.key.as_str(), d.reason.as_str()))
        .collect()
}

/// C-3 (a) over a kept profile: each kept command expands as it did when verified.
pub(super) fn assert_runs_as_verified(proposed: &RepoProfile, kept: &RepoProfile) {
    for key in ["check", "module_test", "module_tests"] {
        if let Some(form) = verified_form(kept, key) {
            assert_eq!(
                Some(form),
                verified_form(proposed, key),
                "{key} of {kept:?}"
            );
        }
    }
}

/// Case A: `module_test` without `{module}` is dropped, nothing tiered is left, and the
/// `check` verification ran with its placeholders filled in would now run with them
/// as literal text, so it goes too.
#[test]
fn case_a_an_untiered_check_with_tier_placeholders_is_dropped() {
    let dir = scratch(true, true);
    let proposed = RepoProfile {
        modules: strings(&["mods/*"]),
        check: Some("sh check.sh {shard} {filter:--skip %}".into()),
        module_test: Some("sh test.sh".into()),
        ..Default::default()
    };
    let v = run_commands(dir.path(), &proposed, None, Duration::from_secs(30), 1);
    assert_eq!(log(dir.path())[0], "check.sh 1");
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    assert_eq!(
        drops(&dropped),
        [
            ("module_test", "must contain {module} exactly once"),
            ("check", UNTIERED_CHECK),
        ]
    );
    assert_eq!(
        UNTIERED_CHECK,
        "check uses tier placeholders but no tier command was kept"
    );
    assert_eq!(kept.check, None);
    assert_runs_as_verified(&proposed, &kept);

    // The same when a failing verification drops the only tier command.
    let proposed = RepoProfile {
        check: Some("sh check.sh --shard {shard}/{shards}".into()),
        build_check: Some("sh build.sh".into()),
        ..Default::default()
    };
    let v = run_commands(dir.path(), &proposed, None, Duration::from_secs(30), 1);
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    let keys: Vec<&str> = dropped.iter().map(|d| d.key.as_str()).collect();
    assert!(keys.contains(&"build_check"), "{dropped:?}");
    assert!(
        drops(&dropped).contains(&("check", UNTIERED_CHECK)),
        "{dropped:?}"
    );
    assert_eq!(kept.check, None);
    assert_runs_as_verified(&proposed, &kept);
}

/// Case B: `module_tests` has no filter slot while both filters are set. It goes; the
/// filters stay, so the kept `module_test` runs with the filter it was verified with.
#[test]
fn case_b_a_module_command_without_a_filter_slot_is_dropped_not_the_filters() {
    let dir = scratch(true, true);
    let proposed = RepoProfile {
        modules: strings(&["mods/*"]),
        check: Some("sh check.sh".into()),
        module_graph: Some("sh graph.sh".into()),
        module_test: Some("sh test.sh {module} {filter:--filter %}".into()),
        module_tests: Some("sh test.sh all {modules:-m %}".into()),
        slow_tests: Some("slow".into()),
        timing_tests: Some("timing".into()),
        ..Default::default()
    };
    let v = run_commands(dir.path(), &proposed, None, Duration::from_secs(30), 1);
    assert!(check_of(&v, "module_test").ok && check_of(&v, "module_tests").ok);
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    assert_eq!(
        drops(&dropped),
        [(
            "module_tests",
            "slow_tests: module_tests must contain {filter:<template>} to leave slow tests out\n\
             timing_tests: module_tests must contain {filter:<template>} to leave timing tests out"
        )]
    );
    assert_eq!(kept.slow_tests.as_deref(), Some("slow"));
    assert_eq!(kept.timing_tests.as_deref(), Some("timing"));
    // The command the stored profile runs is the one verification ran.
    let ran = module_command(&kept, kept.module_test.as_deref().unwrap(), false, "a");
    assert_eq!(ran, "sh test.sh 'a' --filter 'not (slow) and not (timing)'");
    assert!(
        log(dir.path()).contains(&"test.sh a --filter not (slow) and not (timing)".to_string()),
        "{:?}",
        log(dir.path())
    );
    assert_runs_as_verified(&proposed, &kept);
}

/// C-3 (a): a kept module command whose module naming changed with a later drop goes
/// too, naming the key. Minor 1: a kept graph command keeps the module commands
/// (decision 12 drops them only with a dropped graph).
#[test]
fn a_kept_graph_command_keeps_the_module_commands_and_a_changed_naming_drops_them() {
    let proposed = RepoProfile {
        modules: strings(&["mods/*"]),
        check: Some("sh check.sh".into()),
        module_graph: Some("sh graph.sh".into()),
        module_test: Some("sh test.sh {module}".into()),
        ..Default::default()
    };
    let v = ProfileVerification {
        at: 0,
        confined: false,
        setup: None,
        check: ok("sh check.sh"),
        single_test: None,
        build_check: None,
        module_graph: ok("sh graph.sh"),
        module_test: ok("sh test.sh {module}"),
        module_tests: None,
        toolchain_id: None,
    };
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    assert_eq!(drops(&dropped), Vec::<(&str, &str)>::new());
    assert_eq!(kept.module_test, proposed.module_test);

    // Cargo names with a command graph break decision 8: `module_names` goes, which
    // changes how `module_test`'s module is named, so it goes too — not for a graph
    // that was kept.
    let proposed = RepoProfile {
        module_names: Some(ModuleNames::Cargo),
        ..proposed
    };
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    assert_eq!(
        drops(&dropped),
        [
            ("module_names", "cargo needs module_graph = \"cargo\""),
            (
                "module_test",
                "it would no longer run as verified: module_names was dropped"
            ),
        ]
    );
    assert!(dropped.iter().all(|d| d.reason != NEEDS_GRAPH));
    assert_eq!(kept.module_graph.as_deref(), Some("sh graph.sh"));
    assert_runs_as_verified(&proposed, &kept);
}

/// Minor 2: only a whole `*` component is a wildcard, as in `path_module`; `mods/a*`
/// names no module for either.
#[test]
fn a_partial_wildcard_component_names_no_module_for_verification_or_path_module() {
    let dir = scratch(true, true);
    let modules = strings(&["mods/a*"]);
    assert_eq!(path_module("mods/a/x.sh", &modules), None);
    let proposed = RepoProfile {
        modules: modules.clone(),
        module_test: Some("sh test.sh {module}".into()),
        ..Default::default()
    };
    let v = run_commands(dir.path(), &proposed, None, Duration::from_secs(30), 1);
    let test = check_of(&v, "module_test");
    assert!(!test.ok);
    assert!(
        test.tail
            .contains("no module directory matches modules (mods/a*)"),
        "{test:?}"
    );
    assert!(!log(dir.path()).iter().any(|l| l.starts_with("test.sh")));
}
