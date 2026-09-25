//! Pure tests of decisions 5 and 6: precedence, the built-in protected paths, derived
//! filter prefixes, the scout's findings, edits and validation (M8b.4).

use std::path::{Path, PathBuf};

use proto::{OutputFilter, Plan, ProfileSource, ProfileSpec, RepoProfile};

use super::proposal::{EDIT_KEYS, apply_edit, from_findings, validate};
use super::resolve::{apply_choice, derived_prefixes, run_profile};
use crate::run::model::Profile;
use crate::run::plan::{BUILTIN_PROTECTED, BuildContext, Preflight, build_run, resolve_profile};

fn stored_path() -> PathBuf {
    PathBuf::from("/data/repos/p-12345678/profile.toml")
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

/// What a run resolves: `run_profile`, then `choose_profile`'s changes to the plan and
/// the cloned config, then M8a's `resolve_profile`.
fn resolved(
    stored: Option<&RepoProfile>,
    plan: &ProfileSpec,
    config: &ProfileSpec,
) -> (Profile, Vec<String>) {
    let chosen = run_profile(stored, &stored_path(), plan, config);
    let (mut plan, mut config) = (plan.clone(), config.clone());
    apply_choice(&chosen, &mut plan, &mut config);
    (resolve_profile(&plan, &config), chosen.notes)
}

#[test]
fn stored_profile_replaces_the_plan_profile_entirely() {
    let stored = RepoProfile {
        check: Some("a".into()),
        ..Default::default()
    };
    let plan = ProfileSpec {
        check: Some("b".into()),
        single_test: Some("t {test}".into()),
        ..Default::default()
    };
    let chosen = run_profile(
        Some(&stored),
        &stored_path(),
        &plan,
        &ProfileSpec::default(),
    );
    assert_eq!(chosen.spec.check.as_deref(), Some("a"));
    assert_eq!(chosen.spec.single_test, None);
    assert_eq!(chosen.source, ProfileSource::Stored);
    let path = stored_path();
    assert_eq!(
        chosen.notes,
        vec![
            format!(
                "profile.check from the plan file is ignored: this repository has a stored profile ({})",
                path.display()
            ),
            format!(
                "profile.single_test from the plan file is ignored: this repository has a stored profile ({})",
                path.display()
            ),
        ]
    );
}

#[test]
fn a_stored_profile_leaves_no_gap_for_the_config() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        ..Default::default()
    };
    let config = ProfileSpec {
        single_test: Some("cargo test -- --exact {test}".into()),
        setup: Some("cargo fetch".into()),
        ..Default::default()
    };
    let (profile, _) = resolved(Some(&stored), &ProfileSpec::default(), &config);
    assert_eq!(profile.check.as_deref(), Some("cargo test"));
    assert_eq!(profile.single_test, None);
    assert_eq!(profile.setup, None);
}

#[test]
fn without_a_stored_profile_m8a_rule_holds() {
    let plan = ProfileSpec {
        check: Some("plan check".into()),
        ..Default::default()
    };
    let config = ProfileSpec {
        single_test: Some("config {test}".into()),
        ..Default::default()
    };
    let chosen = run_profile(None, &stored_path(), &plan, &config);
    assert_eq!(chosen.source, ProfileSource::Plan);
    assert!(chosen.notes.is_empty());
    let (profile, _) = resolved(None, &plan, &config);
    assert_eq!(profile.check.as_deref(), Some("plan check"));
    assert_eq!(profile.single_test.as_deref(), Some("config {test}"));
}

#[test]
fn nothing_anywhere_is_source_none() {
    let chosen = run_profile(
        None,
        &stored_path(),
        &ProfileSpec::default(),
        &ProfileSpec::default(),
    );
    assert_eq!(chosen.source, ProfileSource::None);
    assert_eq!(chosen.spec, ProfileSpec::default());
    assert!(chosen.notes.is_empty());
    assert!(chosen.filter_prefixes.is_empty());
}

#[test]
fn filter_fields_only_come_from_a_stored_profile() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        output_filter: OutputFilter::Tail,
        filter_prefixes: strings(&["make test"]),
        ..Default::default()
    };
    let plan = ProfileSpec {
        check: Some("npm test".into()),
        ..Default::default()
    };
    let chosen = run_profile(Some(&stored), &stored_path(), &plan, &plan);
    assert_eq!(chosen.output_filter, OutputFilter::Tail);
    assert_eq!(chosen.filter_prefixes, strings(&["make test"]));

    // A stored profile with no prefixes of its own gets decision 28's derived ones.
    let bare = RepoProfile {
        check: Some("cargo test".into()),
        ..Default::default()
    };
    let chosen = run_profile(Some(&bare), &stored_path(), &plan, &plan);
    assert_eq!(chosen.output_filter, OutputFilter::FailuresOnly);
    assert_eq!(chosen.filter_prefixes, strings(&["cargo test"]));

    // Without a stored profile: nothing, whatever the plan and the config say.
    let chosen = run_profile(None, &stored_path(), &plan, &plan);
    assert_eq!(chosen.output_filter, OutputFilter::default());
    assert!(chosen.filter_prefixes.is_empty());
}

#[test]
fn builtins_always_apply_whatever_the_stored_list() {
    let plan = ProfileSpec {
        protected: Some(strings(&["docs/agents/**"])),
        ..Default::default()
    };
    let config = ProfileSpec {
        protected: Some(strings(&["GEMINI.md"])),
        ..Default::default()
    };
    for extras in [Vec::new(), strings(&[".cursor/**"])] {
        let stored = RepoProfile {
            protected: extras.clone(),
            check: Some("cargo test".into()),
            ..Default::default()
        };
        let (profile, notes) = resolved(Some(&stored), &plan, &config);
        for builtin in BUILTIN_PROTECTED {
            assert!(
                profile.protected.iter().any(|p| p == builtin),
                "{builtin} missing from {:?}",
                profile.protected
            );
        }
        assert_eq!(
            profile.protected.iter().any(|p| p == ".cursor/**"),
            !extras.is_empty()
        );
        assert!(profile.protected.iter().any(|p| p == "docs/agents/**"));
        assert!(profile.protected.iter().any(|p| p == "GEMINI.md"));
        assert!(
            notes.iter().all(|n| !n.contains("protected")),
            "no ignored note for protected: {notes:?}"
        );
    }
}

fn confinement_plan() -> Plan {
    crate::run::plan::parse_plan(
        r#"
goal = "Confinement"
[profile]
check = "plan check"
[[task]]
id = "t1"
title = "One"
size = "S"
owns = ["src/a.rs"]
brief = "b"
acceptance = ["a"]
"#,
    )
    .unwrap()
}

#[test]
fn confinement_still_comes_from_the_users_config() {
    let root = PathBuf::from("/tmp/ax-profile-root-a");
    let other = "/tmp/ax-profile-root-b".to_string();
    let mut config = config::Orchestrator::default();
    config
        .cache_dirs
        .insert(root.display().to_string(), strings(&["~/.cache/mine"]));
    config
        .cache_dirs
        .insert(other.clone(), strings(&["~/.cache/other"]));
    config
        .confined_network
        .insert(root.display().to_string(), true);
    config.confined_network.insert(other, false);
    let stored = RepoProfile {
        check: Some("stored check".into()),
        ..Default::default()
    };
    let mut plan = confinement_plan();
    let chosen = run_profile(
        Some(&stored),
        &stored_path(),
        &plan.profile,
        &config.profile,
    );
    apply_choice(&chosen, &mut plan.profile, &mut config.profile);
    let pre = Preflight {
        root: root.clone(),
        project: root.clone(),
        git_common_dir: root.join(".git"),
        base_branch: "main".into(),
        base_sha: "b".repeat(40),
        protected_files: Vec::new(),
    };
    let ctx = BuildContext {
        id: "confinement-0001".into(),
        wt_dir: PathBuf::from("/tmp/wt"),
        data_dir: PathBuf::from("/tmp/data/runs/confinement-0001"),
        config: &config,
        now: 1,
        yes: false,
    };
    let run = build_run(plan, pre, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(run.profile.check.as_deref(), Some("stored check"));
    assert_eq!(run.profile.cache_dirs, strings(&["~/.cache/mine"]));
    assert!(run.profile.confined_network);
}

#[test]
fn derived_prefixes_from_check_and_single_test() {
    let profile = RepoProfile {
        check: Some("cargo build --all && cargo test -q; cargo fmt --check".into()),
        single_test: Some("cargo test -- --exact {test}".into()),
        ..Default::default()
    };
    assert_eq!(
        derived_prefixes(&profile),
        strings(&["cargo build", "cargo test", "cargo fmt"])
    );
}

#[test]
fn from_findings_keeps_only_extra_protected_entries() {
    let proposed = RepoProfile {
        protected: strings(&[".claude/**", "**/AGENTS.md", ".cursor/**", ".cursor/**"]),
        ..Default::default()
    };
    assert_eq!(from_findings(&proposed).protected, strings(&[".cursor/**"]));
}

#[test]
fn from_findings_drops_reserved_env_keys() {
    let mut proposed = RepoProfile::default();
    for key in [
        "PATH",
        "HOME",
        "BASH_ENV",
        "PYTHONPATH",
        "ANTHREX_X",
        "PYTHONUNBUFFERED",
        "RUST_LOG",
    ] {
        proposed.env.insert(key.into(), "1".into());
    }
    let kept: Vec<String> = from_findings(&proposed).env.into_keys().collect();
    assert_eq!(kept, strings(&["PYTHONUNBUFFERED", "RUST_LOG"]));
}

fn with_everything() -> RepoProfile {
    RepoProfile {
        check: Some("cargo test".into()),
        single_test: Some("cargo test -- --exact {test}".into()),
        ..Default::default()
    }
}

#[test]
fn edit_of_protected_refuses_builtins() {
    let stored = with_everything();
    assert_eq!(
        apply_edit(&stored, "protected", Some(r#"[".mcp.json"]"#)),
        Err("protected: .mcp.json is built in and always applies; list only extra entries".into())
    );
    let (edited, reverify) =
        apply_edit(&stored, "protected", Some(r#"[".cursor/**", "GEMINI.md"]"#)).unwrap();
    assert_eq!(edited.protected, strings(&[".cursor/**", "GEMINI.md"]));
    assert!(!reverify);
    let (unset, _) = apply_edit(&edited, "protected", None).unwrap();
    assert!(unset.protected.is_empty());
    let (profile, _) = resolved(
        Some(&unset),
        &ProfileSpec::default(),
        &ProfileSpec::default(),
    );
    assert_eq!(profile.protected, strings(BUILTIN_PROTECTED));
}

#[test]
fn edit_refuses_confinement_keys() {
    let stored = with_everything();
    let keys = EDIT_KEYS.join(", ");
    assert_eq!(
        apply_edit(&stored, "cache_dirs", Some(r#"["/tmp"]"#)),
        Err(format!("unknown key cache_dirs; one of {keys}"))
    );
    assert_eq!(
        apply_edit(&stored, "confined_network", Some("true")),
        Err(format!("unknown key confined_network; one of {keys}"))
    );
}

#[test]
fn edit_takes_a_bare_word_as_a_string_and_reports_reverification() {
    let stored = with_everything();
    let (edited, reverify) = apply_edit(&stored, "check", Some("cargo test --all")).unwrap();
    assert_eq!(edited.check.as_deref(), Some("cargo test --all"));
    assert!(reverify);
    let (edited, reverify) = apply_edit(&stored, "env.RUST_LOG", Some("debug")).unwrap();
    assert_eq!(edited.env["RUST_LOG"], "debug");
    assert!(reverify);
    let (edited, reverify) = apply_edit(&stored, "languages", Some(r#"["rust"]"#)).unwrap();
    assert_eq!(edited.languages, strings(&["rust"]));
    assert!(!reverify);
    assert!(apply_edit(&stored, "env.PATH", Some("/x")).is_err());
    assert!(apply_edit(&stored, "single_test", Some("cargo test")).is_err());
}

fn problems(profile: RepoProfile) -> Vec<String> {
    validate(&profile)
}

#[test]
fn validate_reports_each_bad_key() {
    assert!(problems(with_everything()).is_empty());
    assert_eq!(
        problems(RepoProfile {
            modules: strings(&["../x/*"]),
            ..Default::default()
        }),
        strings(&["modules: ../x/* must not contain .."])
    );
    assert_eq!(
        problems(RepoProfile {
            protected: strings(&["/etc/agents.md"]),
            ..Default::default()
        }),
        strings(&["protected: /etc/agents.md must not be absolute"])
    );
    assert_eq!(
        problems(RepoProfile {
            single_test: Some("cargo test".into()),
            ..Default::default()
        }),
        strings(&["single_test: must contain {test}"])
    );
    assert_eq!(
        problems(RepoProfile {
            test_passed: Some("ok".into()),
            ..Default::default()
        }),
        strings(&["test_passed: must contain {test}"])
    );
    let regex = problems(RepoProfile {
        test_passed: Some("({test}".into()),
        ..Default::default()
    });
    assert_eq!(regex.len(), 1, "{regex:?}");
    assert!(
        regex[0].starts_with("test_passed: is not a valid regular expression: "),
        "{regex:?}"
    );
    let mut env = RepoProfile::default();
    env.env.insert("1X".into(), "1".into());
    assert_eq!(
        problems(env),
        strings(&["env: key 1X must match [A-Za-z_][A-Za-z0-9_]*"])
    );
    let mut env = RepoProfile::default();
    env.env.insert("PATH".into(), "/x".into());
    let reason = config::reserved_env::reserved_env("PATH").unwrap();
    assert_eq!(
        problems(env),
        vec![format!(
            "env: key PATH may not be set by a profile: {reason}"
        )]
    );
    assert_eq!(
        problems(RepoProfile {
            generated: strings(&["/abs/Cargo.lock"]),
            ..Default::default()
        }),
        strings(&["generated: /abs/Cargo.lock must not be absolute"])
    );
}

#[test]
fn summary_prints_the_built_ins_and_the_degradations() {
    let text = super::summary(&RepoProfile {
        languages: strings(&["rust"]),
        protected: strings(&[".cursor/**"]),
        ..Default::default()
    });
    assert_eq!(
        text,
        "languages: rust\n\
         protected: built-in .claude/**, .mcp.json, .codex/**, **/CLAUDE.md, **/AGENTS.md + .cursor/**\n\
         setup: none\n\
         check: none (runs are unverified)\n\
         single test: none (tdd is impossible; code tasks use check)"
    );
    let bare = super::summary(&RepoProfile::default());
    assert!(bare.contains("+ no extras"), "{bare}");
}

#[test]
fn repo_dir_is_keyed_by_project() {
    let dir = super::repo_dir(Path::new("/data"), Path::new("/src/my repo"));
    assert_eq!(
        dir,
        crate::worktree::repo_worktrees_dir(Path::new("/data/repos"), Path::new("/src/my repo"))
    );
}

#[test]
fn a_stale_profile_gets_the_attention_line() {
    let mut config = config::Orchestrator::default();
    let mut plan = confinement_plan();
    let chosen = run_profile(None, &stored_path(), &plan.profile, &config.profile);
    apply_choice(&chosen, &mut plan.profile, &mut config.profile);
    let pre = Preflight {
        root: PathBuf::from("/tmp/ax-profile-root-a"),
        project: PathBuf::from("/tmp/ax-profile-root-a"),
        git_common_dir: PathBuf::from("/tmp/ax-profile-root-a/.git"),
        base_branch: "main".into(),
        base_sha: "b".repeat(40),
        protected_files: Vec::new(),
    };
    let ctx = BuildContext {
        id: "stale-0001".into(),
        wt_dir: PathBuf::from("/tmp/wt"),
        data_dir: PathBuf::from("/tmp/data/runs/stale-0001"),
        config: &config,
        now: 1,
        yes: false,
    };
    let mut run = build_run(plan, pre, ctx).unwrap_or_else(|e| panic!("{e:?}"));
    assert_eq!(run.stale_profile_line(), None);
    run.stale_profile = strings(&["Cargo.toml", "AGENTS.md"]);
    assert_eq!(
        run.stale_profile_line().as_deref(),
        Some(
            "the repository profile may be stale: Cargo.toml, AGENTS.md changed since it was confirmed; run anthrex profile detect"
        )
    );
}
