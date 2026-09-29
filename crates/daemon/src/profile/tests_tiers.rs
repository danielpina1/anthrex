//! Milestone 9.1 task M9.1.5: a stored profile's tier keys — verification (decision
//! 12), `profile show`, `profile edit` and the summary line.

use std::path::Path;
use std::time::Duration;

use proto::{CommandCheck, ModuleNames, ProfileVerification, RepoProfile};

use super::proposal::{apply_edit, apply_verification, show_text};
use super::proposal_tiers::{NEEDS_GRAPH, SHARDS_NEED_CHECK};
use super::summary;
use super::verify::run_commands;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// A scratch checkout: `mods/{b,a}`, and scripts that log their name and arguments to
/// `log.txt` in order.
fn scratch(graph_ok: bool, check_ok: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for module in ["mods/b", "mods/a", "mods/.hidden", "docs"] {
        std::fs::create_dir_all(root.join(module)).unwrap();
    }
    std::fs::write(root.join("mods/not-a-dir"), "").unwrap();
    let log = root.join("log.txt");
    let log = log.to_str().unwrap();
    let script = |name: &str, body: &str| {
        std::fs::write(
            root.join(name),
            format!("echo \"{name} $*\" >> '{log}'\n{body}\n"),
        )
        .unwrap();
    };
    script("build.sh", "echo 'error: build broke'; exit 1");
    script(
        "check.sh",
        if check_ok {
            "exit 0"
        } else {
            "echo 'check red'; exit 1"
        },
    );
    script(
        "graph.sh",
        if graph_ok {
            r#"echo '{"a": [], "b": ["a"]}'"#
        } else {
            "echo 'no graph here' >&2; exit 2"
        },
    );
    script("test.sh", "exit 0");
    script("toolchain.sh", "echo 'rustc 1.92.0'");
    dir
}

fn log(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("log.txt"))
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

fn tiered() -> RepoProfile {
    RepoProfile {
        modules: strings(&["mods/*"]),
        check: Some("sh check.sh --shard {shard}/{shards}".into()),
        build_check: Some("sh build.sh".into()),
        module_graph: Some("sh graph.sh".into()),
        module_test: Some("sh test.sh {module} {filter:--filter %}".into()),
        module_tests: Some("sh test.sh all {modules:-m %}".into()),
        slow_tests: Some("slow".into()),
        timing_tests: Some("timing".into()),
        full_shards: Some(2),
        toolchain_id: Some("sh toolchain.sh".into()),
        ..Default::default()
    }
}

fn check_of<'a>(v: &'a ProfileVerification, key: &str) -> &'a CommandCheck {
    let check = match key {
        "build_check" => &v.build_check,
        "module_graph" => &v.module_graph,
        "module_test" => &v.module_test,
        "module_tests" => &v.module_tests,
        "toolchain_id" => &v.toolchain_id,
        _ => unreachable!(),
    };
    check
        .as_ref()
        .unwrap_or_else(|| panic!("{key} not verified: {v:?}"))
}

#[test]
fn verification_runs_the_new_commands_in_order_and_drops_failures() {
    let timeout = Duration::from_secs(30);
    // A command graph and directory names: the first module by name is `a`.
    let dir = scratch(true, true);
    let proposed = tiered();
    let v = run_commands(dir.path(), &proposed, None, timeout, 1);
    assert_eq!(
        log(dir.path()),
        [
            // M8b's check, run whole: `{shard}/{shards}` is 1 of 1.
            "check.sh --shard 1/1",
            "build.sh ",
            "graph.sh ",
            "test.sh a --filter not (slow) and not (timing)",
            "test.sh all -m a",
            "toolchain.sh ",
        ]
    );
    let build = check_of(&v, "build_check");
    assert!(!build.ok);
    assert_eq!(build.code, Some(1));
    assert!(build.tail.contains("error: build broke"), "{build:?}");
    for key in [
        "module_graph",
        "module_test",
        "module_tests",
        "toolchain_id",
    ] {
        assert!(check_of(&v, key).ok, "{key}: {v:?}");
    }
    assert_eq!(check_of(&v, "module_graph").command, "sh graph.sh");
    assert_eq!(
        check_of(&v, "module_test").command,
        "sh test.sh {module} {filter:--filter %}"
    );
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    assert_eq!(kept.build_check, None);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].key, "build_check");
    assert_eq!(dropped[0].reason, format!("exit 1 after {}s", build.secs));
    assert!(dropped[0].tail.contains("error: build broke"));
    assert_eq!(kept.module_graph.as_deref(), Some("sh graph.sh"));
    assert_eq!(kept.module_test, proposed.module_test);
    assert_eq!(kept.module_tests, proposed.module_tests);
    assert_eq!(kept.toolchain_id, proposed.toolchain_id);
    assert_eq!(kept.full_shards, Some(2));

    // The cargo graph and cargo names: no workspace here, so `cargo metadata` fails, the
    // graph is dropped, and the module commands go with it unrun. A red `check` takes
    // `full_shards` with it; a failing `toolchain_id` is dropped.
    let dir = scratch(false, false);
    std::fs::write(dir.path().join("toolchain.sh"), "exit 3\n").unwrap();
    let proposed = RepoProfile {
        module_graph: Some("cargo".into()),
        module_names: Some(ModuleNames::Cargo),
        ..tiered()
    };
    let v = run_commands(dir.path(), &proposed, None, timeout, 1);
    assert!(!check_of(&v, "module_graph").ok, "{v:?}");
    assert_eq!(check_of(&v, "module_graph").command, "cargo");
    assert_eq!(v.module_test, None);
    assert_eq!(v.module_tests, None);
    assert!(!check_of(&v, "toolchain_id").ok);
    assert!(!log(dir.path()).iter().any(|l| l.starts_with("test.sh")));
    let (kept, dropped) = apply_verification(&proposed, &v, Path::new("/work/app"));
    let reasons: Vec<(&str, &str)> = dropped
        .iter()
        .map(|d| (d.key.as_str(), d.reason.lines().next().unwrap_or_default()))
        .collect();
    assert_eq!(reasons[0].0, "check");
    assert_eq!(reasons[1].0, "build_check");
    assert_eq!(reasons[2].0, "module_graph");
    assert_eq!(
        &reasons[3..],
        [
            ("module_test", NEEDS_GRAPH),
            ("module_tests", NEEDS_GRAPH),
            ("toolchain_id", "exit 3 after 0s"),
            ("full_shards", SHARDS_NEED_CHECK),
        ]
    );
    for key in [
        &kept.module_graph,
        &kept.module_test,
        &kept.module_tests,
        &kept.toolchain_id,
        &kept.build_check,
    ] {
        assert_eq!(*key, None);
    }
    assert_eq!(kept.full_shards, None);

    // A graph that prints something other than a module graph is unknown.
    let dir = scratch(true, true);
    std::fs::write(dir.path().join("graph.sh"), "echo '{\"a\": [\"zz\"]}'\n").unwrap();
    let v = run_commands(dir.path(), &tiered(), None, timeout, 1);
    let graph = check_of(&v, "module_graph");
    assert!(!graph.ok);
    assert!(
        graph
            .tail
            .ends_with("module graph unknown: zz is not a module"),
        "{graph:?}"
    );
    // With directory names the module commands still run without a graph.
    assert!(check_of(&v, "module_test").ok);
}

#[test]
fn a_module_command_with_no_module_directory_is_not_verified() {
    let dir = scratch(true, true);
    let proposed = RepoProfile {
        modules: strings(&["nowhere/*"]),
        ..tiered()
    };
    let v = run_commands(dir.path(), &proposed, None, Duration::from_secs(30), 1);
    let test = check_of(&v, "module_test");
    assert!(!test.ok);
    assert!(
        test.tail
            .contains("no module directory matches modules (nowhere/*)")
    );
}

#[test]
fn profile_show_prints_every_new_key() {
    let profile = RepoProfile {
        module_names: Some(ModuleNames::Dir),
        full_triggers: strings(&["Cargo.lock"]),
        skip_markers: strings(&["#[ignore"]),
        test_paths: strings(&["**/tests/**"]),
        ..tiered()
    };
    let check = |command: &str, ok: bool| CommandCheck {
        command: command.to_string(),
        ok,
        code: Some(if ok { 0 } else { 1 }),
        timed_out: false,
        secs: 2,
        tail: String::new(),
    };
    let v = ProfileVerification {
        at: 0,
        confined: false,
        setup: None,
        check: None,
        single_test: None,
        build_check: Some(check("sh build.sh", false)),
        module_graph: Some(check("sh graph.sh", true)),
        module_test: Some(check("sh test.sh {module} {filter:--filter %}", true)),
        module_tests: Some(check("sh test.sh all {modules:-m %}", true)),
        toolchain_id: Some(check("sh toolchain.sh", true)),
    };
    let text = show_text(&profile, Some(&v), &[]);
    let expected = r##"modules = ["mods/*"]
check = "sh check.sh --shard {shard}/{shards}"
output_filter = "failures-only"
build_check = "sh build.sh"
module_test = "sh test.sh {module} {filter:--filter %}"
module_tests = "sh test.sh all {modules:-m %}"
module_graph = "sh graph.sh"
module_names = "dir"
full_triggers = ["Cargo.lock"]
slow_tests = "slow"
timing_tests = "timing"
skip_markers = ["#[ignore"]
test_paths = ["**/tests/**"]
full_shards = 2
toolchain_id = "sh toolchain.sh"

# protected: built-in .claude/**, .mcp.json, .codex/**, **/CLAUDE.md, **/AGENTS.md + no extras
# verification 1970-01-01 00:00 (unconfined)
#   build_check  fail   2s   sh build.sh
#   module_graph ok     2s   sh graph.sh
#   module_test  ok     2s   sh test.sh {module} {filter:--filter %}
#   module_tests ok     2s   sh test.sh all {modules:-m %}
#   toolchain_id ok     2s   sh toolchain.sh
"##;
    assert_eq!(text, expected);
}

#[test]
fn edits_of_tier_keys_are_validated_across_keys_and_reverify() {
    let stored = tiered();
    let (edited, reverify) = apply_edit(&stored, "build_check", Some("make")).unwrap();
    assert_eq!(edited.build_check.as_deref(), Some("make"));
    assert!(reverify);
    let (edited, reverify) = apply_edit(&stored, "test_paths", Some(r#"["tests/**"]"#)).unwrap();
    assert_eq!(edited.test_paths, ["tests/**"]);
    assert!(!reverify);
    let (edited, reverify) = apply_edit(&stored, "full_shards", Some("4")).unwrap();
    assert_eq!(edited.full_shards, Some(4));
    assert!(reverify);
    assert_eq!(
        apply_edit(&stored, "module_names", Some("cargo")).unwrap_err(),
        "module_names: cargo needs module_graph = \"cargo\""
    );
    // Removing the filter from module_test is a problem named by slow_tests and
    // timing_tests, and is refused all the same.
    assert_eq!(
        apply_edit(&stored, "module_test", Some("sh test.sh {module}")).unwrap_err(),
        "slow_tests: module_test must contain {filter:<template>} to leave slow tests out\n\
         timing_tests: module_test must contain {filter:<template>} to leave timing tests out"
    );
    assert_eq!(
        apply_edit(&stored, "full_shards", Some("40")).unwrap_err(),
        "full_shards: must be between 1 and 16"
    );
}

#[test]
fn summary_has_the_tiers_line_only_when_tiered() {
    let untiered = RepoProfile {
        check: Some("cargo test".into()),
        ..Default::default()
    };
    assert!(
        !summary(&untiered).lines().any(|l| l.starts_with("tiers:")),
        "{}",
        summary(&untiered)
    );
    let profile = RepoProfile {
        build_check: Some("cargo build".into()),
        module_graph: Some("cargo".into()),
        module_test: Some("cargo test -p {module}".into()),
        slow_tests: Some("test(e2e)".into()),
        full_triggers: strings(&["Cargo.lock", ".github/**"]),
        ..untiered.clone()
    };
    assert_eq!(
        summary(&profile).lines().last(),
        Some("tiers: build_check, module tests by cargo graph, slow filter, 2 full triggers")
    );
    let command = RepoProfile {
        module_graph: Some("sh graph.sh".into()),
        module_tests: Some("t {modules}".into()),
        ..untiered.clone()
    };
    assert_eq!(
        summary(&command).lines().last(),
        Some("tiers: module tests by command graph")
    );
}
