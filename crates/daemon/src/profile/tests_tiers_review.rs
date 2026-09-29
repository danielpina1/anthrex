//! M9.1.5 review fixes: a verified proposal always keeps the stored profile valid (I1),
//! verification's module pick (m1, m3), and the pins of m2.

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::{CommandCheck, ModuleNames, ProfileSpec, ProfileVerification, RepoProfile};

use super::proposal::{apply_verification, from_findings, validate};
use super::proposal_tiers::NEEDS_GRAPH;
use super::resolve::run_profile;
use super::tests_tiers::{check_of, log, scratch, strings, tiered};
use super::verify::run_commands;
use super::verify_tiers::graph_timeout;
use crate::run::plan::Preflight;
use crate::run::plan::{BuildContext, build_run, parse_plan};
use crate::run::tiers::GRAPH_TIMEOUT;

const RUN_ID: &str = "tiers-0001";

fn preflight() -> Preflight {
    Preflight {
        root: PathBuf::from("/tmp/x"),
        project: PathBuf::from("/tmp/p"),
        git_common_dir: PathBuf::from("/tmp/p/.git"),
        base_branch: "main".to_string(),
        base_sha: "b".repeat(40),
        protected_files: Vec::new(),
    }
}

fn record(command: &str, ok: bool) -> Option<CommandCheck> {
    Some(CommandCheck {
        command: command.to_string(),
        ok,
        code: Some(if ok { 0 } else { 1 }),
        timed_out: false,
        secs: 0,
        tail: String::new(),
    })
}

/// A verification in which every proposed command passed, except those in `failed`.
fn verified(p: &RepoProfile, failed: &[&str]) -> ProfileVerification {
    let rec = |key: &str, value: &Option<String>| {
        value
            .as_deref()
            .and_then(|c| record(c, !failed.contains(&key)))
    };
    ProfileVerification {
        at: 0,
        confined: false,
        setup: rec("setup", &p.setup),
        check: rec("check", &p.check),
        single_test: None,
        build_check: rec("build_check", &p.build_check),
        module_graph: rec("module_graph", &p.module_graph),
        module_test: rec("module_test", &p.module_test),
        module_tests: rec("module_tests", &p.module_tests),
        toolchain_id: rec("toolchain_id", &p.toolchain_id),
    }
}

/// What a confirmed proposal must always be: valid as a stored profile and as the
/// profile of a plan (`check_plan`, through `build_run`).
fn assert_confirmable(kept: &RepoProfile) {
    assert_eq!(validate(kept), Vec::<String>::new(), "{kept:?}");
    let text = "goal = \"g\"\n\n[[task]]\nid = \"t1\"\ntitle = \"T\"\nsize = \"S\"\n\
                owns = [\"mods/a/x.sh\"]\nbrief = \"B\"\nacceptance = [\"A\"]\n";
    let mut plan = parse_plan(text).unwrap();
    plan.profile = kept.spec();
    let config = config::Orchestrator::default();
    let built = build_run(
        plan,
        preflight(),
        BuildContext {
            id: RUN_ID.to_string(),
            wt_dir: PathBuf::from("/tmp/wt"),
            data_dir: PathBuf::from(format!("/tmp/data/runs/{RUN_ID}")),
            config: &config,
            testing: &config::Testing::default(),
            now: 1_000,
            yes: false,
        },
    );
    if let Err(errors) = built {
        panic!("{kept:?} is refused by a plan: {errors:?}");
    }
}

/// A proposal, the commands whose verification failed, and the drops expected.
type Case = (
    RepoProfile,
    Vec<&'static str>,
    Vec<(&'static str, &'static str)>,
);

fn base() -> RepoProfile {
    RepoProfile {
        modules: strings(&["mods/*"]),
        check: Some("sh check.sh".into()),
        ..Default::default()
    }
}

/// I1: whatever the scout proposes and whatever verification drops, what is left
/// passes decision 8, and each key dropped for it is listed with the problem.
#[test]
fn a_verified_proposal_always_passes_the_tier_rules() {
    let cases: Vec<Case> = vec![
        // Repro 1: no `{module}`, and a slow filter it cannot take.
        (
            RepoProfile {
                module_test: Some("sh test.sh".into()),
                slow_tests: Some("slow".into()),
                ..base()
            },
            vec![],
            vec![(
                "module_test",
                "must contain {module} exactly once\n\
                 slow_tests: module_test must contain {filter:<template>} to leave slow tests out",
            )],
        ),
        // Repro 2: the cargo graph fails; its names go with it.
        (
            RepoProfile {
                module_graph: Some("cargo".into()),
                module_names: Some(ModuleNames::Cargo),
                module_test: Some("sh test.sh {module}".into()),
                ..base()
            },
            vec!["module_graph"],
            vec![
                ("module_graph", "exit 1 after 0s"),
                ("module_test", NEEDS_GRAPH),
                ("module_names", "cargo needs module_graph = \"cargo\""),
            ],
        ),
        (
            RepoProfile {
                build_check: Some("make {module}".into()),
                ..base()
            },
            vec![],
            vec![("build_check", "{module} is not allowed here")],
        ),
        (
            RepoProfile {
                toolchain_id: Some("rustc {shard}".into()),
                ..base()
            },
            vec![],
            vec![("toolchain_id", "{shard} is not allowed here")],
        ),
        (
            RepoProfile {
                module_graph: Some("sh graph.sh {module}".into()),
                ..base()
            },
            vec![],
            vec![("module_graph", "{module} is not allowed here")],
        ),
        (
            RepoProfile {
                module_tests: Some("sh test.sh".into()),
                ..base()
            },
            vec![],
            vec![(
                "module_tests",
                "must contain {modules} or {modules:<template>}",
            )],
        ),
        // A module command that cannot leave timing tests out.
        (
            RepoProfile {
                module_test: Some("sh test.sh {module}".into()),
                timing_tests: Some("timing".into()),
                ..base()
            },
            vec![],
            // Ruling C-3 (c): the command without a filter slot goes, not the filter.
            vec![(
                "module_test",
                "timing_tests: module_test must contain {filter:<template>} to leave timing tests out",
            )],
        ),
        // A tiered `check` with a placeholder it may not hold.
        (
            RepoProfile {
                check: Some("sh check.sh {module}".into()),
                build_check: Some("make".into()),
                ..base()
            },
            vec![],
            vec![("check", "{module} is not allowed here")],
        ),
    ];
    for (proposed, failed, expected) in cases {
        let found = from_findings(&proposed);
        let v = verified(&found, &failed);
        let (kept, dropped) = apply_verification(&found, &v, Path::new("/work/app"));
        let got: Vec<(&str, &str)> = dropped
            .iter()
            .map(|d| (d.key.as_str(), d.reason.as_str()))
            .collect();
        assert_eq!(got, expected, "{proposed:?}");
        assert_confirmable(&kept);
        // Ruling C-3 (a): what is kept runs as it was verified.
        super::tests_tiers_expansion::assert_runs_as_verified(&found, &kept);
    }
}

/// I1: blank tier values never reach verification.
#[test]
fn blank_tier_values_are_not_proposed() {
    let proposed = RepoProfile {
        build_check: Some("  ".into()),
        module_graph: Some("".into()),
        slow_tests: Some("\t".into()),
        toolchain_id: Some(" ".into()),
        ..base()
    };
    let found = from_findings(&proposed);
    assert_eq!(found.build_check, None);
    assert_eq!(found.module_graph, None);
    assert_eq!(found.slow_tests, None);
    assert_eq!(found.toolchain_id, None);
    assert_confirmable(&found);
}

/// m1 and m3: the module verification picks is never a name a shell would read as an
/// option, nor a directory whose name is a glob (which `path_module` never names).
#[test]
fn verification_skips_option_like_and_glob_named_module_directories() {
    let dir = scratch(true, true);
    for name in ["-rf", "[a]", "a*", "?"] {
        std::fs::create_dir_all(dir.path().join("mods").join(name)).unwrap();
    }
    let v = run_commands(dir.path(), &tiered(), None, Duration::from_secs(30), 1);
    assert!(check_of(&v, "module_test").ok);
    let tests: Vec<String> = log(dir.path())
        .into_iter()
        .filter(|l| l.starts_with("test.sh"))
        .collect();
    assert_eq!(
        tests,
        [
            "test.sh a --filter not (slow) and not (timing)",
            "test.sh all -m a --filter not (slow) and not (timing)"
        ]
    );
}

/// m2: an untiered `check` runs exactly as written, placeholders and all.
#[test]
fn an_untiered_check_is_verified_as_written() {
    let dir = scratch(true, true);
    let profile = RepoProfile {
        check: Some("sh check.sh {shard} {filter:x}".into()),
        ..Default::default()
    };
    let v = run_commands(dir.path(), &profile, None, Duration::from_secs(30), 1);
    assert!(v.check.unwrap().ok);
    assert_eq!(log(dir.path()), ["check.sh {shard} {filter:x}"]);
}

/// m2: the graph's stdout goes to a file in the command's `TMPDIR`, never into the
/// checkout.
#[test]
fn the_graph_output_file_is_in_tmpdir_not_the_checkout() {
    let dir = scratch(true, true);
    let seen = dir.path().join("seen.txt");
    std::fs::write(
        dir.path().join("graph.sh"),
        format!(
            "ls -a \"${{TMPDIR:-/tmp}}\" | grep anthrex-graph- | sed 's/^/tmp: /' >> '{seen}'\n\
             ls -a . | grep anthrex-graph- | sed 's/^/checkout: /' >> '{seen}'\n\
             echo '{{\"a\": [], \"b\": [\"a\"]}}'\n",
            seen = seen.display()
        ),
    )
    .unwrap();
    let v = run_commands(dir.path(), &tiered(), None, Duration::from_secs(30), 1);
    assert!(check_of(&v, "module_graph").ok, "{v:?}");
    let seen = std::fs::read_to_string(&seen).unwrap();
    assert!(seen.lines().any(|l| l.starts_with("tmp: ")), "{seen}");
    assert!(!seen.contains("checkout: "), "{seen}");
    let left: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("anthrex-graph-")
        })
        .collect();
    assert!(left.is_empty());
}

/// m2: the graph is bounded by the shorter of its own bound and verification's.
#[test]
fn the_graph_timeout_is_the_shorter_bound() {
    assert_eq!(
        graph_timeout(Duration::from_secs(1)),
        Duration::from_secs(1)
    );
    assert_eq!(graph_timeout(Duration::from_secs(3600)), GRAPH_TIMEOUT);
    let dir = scratch(true, true);
    std::fs::write(dir.path().join("graph.sh"), "sleep 20\n").unwrap();
    let profile = RepoProfile {
        module_test: None,
        module_tests: None,
        slow_tests: None,
        timing_tests: None,
        ..tiered()
    };
    let v = run_commands(dir.path(), &profile, None, Duration::from_secs(1), 1);
    assert!(check_of(&v, "module_graph").timed_out, "{v:?}");
}

/// m2: a plan's tier key that a stored profile overrides is noted, key by key.
#[test]
fn a_plan_tier_key_under_a_stored_profile_is_noted() {
    let plan = ProfileSpec {
        build_check: Some("b".into()),
        module_test: Some("t {module}".into()),
        module_tests: Some("t {modules}".into()),
        module_graph: Some("cargo".into()),
        module_names: Some(ModuleNames::Dir),
        full_triggers: Some(strings(&["x"])),
        slow_tests: Some("s".into()),
        timing_tests: Some("t".into()),
        skip_markers: Some(strings(&["m"])),
        test_paths: Some(strings(&["p/**"])),
        full_shards: Some(2),
        toolchain_id: Some("v".into()),
        ..ProfileSpec::default()
    };
    let path = Path::new("/data/repos/r/profile.toml");
    let chosen = run_profile(Some(&base()), path, &plan, &ProfileSpec::default());
    let keys: Vec<&str> = chosen
        .notes
        .iter()
        .map(|n| {
            n.strip_prefix("profile.")
                .and_then(|n| n.split(' ').next())
                .unwrap()
        })
        .collect();
    assert_eq!(
        keys,
        [
            "build_check",
            "module_test",
            "module_tests",
            "module_graph",
            "module_names",
            "full_triggers",
            "slow_tests",
            "timing_tests",
            "skip_markers",
            "test_paths",
            "full_shards",
            "toolchain_id",
        ]
    );
    assert_eq!(
        chosen.notes[0],
        "profile.build_check from the plan file is ignored: this repository has a stored profile (/data/repos/r/profile.toml)"
    );
}
