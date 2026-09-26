//! M8b.10: decision 9's verification rules, decision 10's edit and show, and the
//! confinement verification runs under (pure parts, and `verify::confine_spec`).

use std::path::{Path, PathBuf};

use proto::{CommandCheck, DroppedCommand, ProfileVerification, RepoProfile};

use super::confined_hint;
use super::proposal::{EDIT_KEYS, SINGLE_TEST_NEEDS, apply_edit, apply_verification, show_text};
use super::verify::confine_spec;
use crate::run::plan::Preflight;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn root() -> PathBuf {
    PathBuf::from("/work/app")
}

fn passed(command: &str, secs: u64) -> CommandCheck {
    CommandCheck {
        command: command.to_string(),
        ok: true,
        code: Some(0),
        timed_out: false,
        secs,
        tail: String::new(),
    }
}

fn failed(command: &str, code: i32, tail: &str) -> CommandCheck {
    CommandCheck {
        command: command.to_string(),
        ok: false,
        code: Some(code),
        timed_out: false,
        secs: 1,
        tail: tail.to_string(),
    }
}

fn proposed() -> RepoProfile {
    RepoProfile {
        setup: Some("sh setup.sh".into()),
        check: Some("sh check.sh".into()),
        single_test: Some("sh tests/{test}.sh".into()),
        test_passed: Some("PASS {test}".into()),
        sample_test: Some("t_ok".into()),
        ..Default::default()
    }
}

fn verification(
    confined: bool,
    setup: Option<CommandCheck>,
    check: Option<CommandCheck>,
    single_test: Option<CommandCheck>,
) -> ProfileVerification {
    ProfileVerification {
        at: 1_790_000_000,
        confined,
        setup,
        check,
        single_test,
    }
}

#[test]
fn verification_drops_commands_that_did_not_pass() {
    let v = verification(
        false,
        Some(failed("sh setup.sh", 1, "setup.sh: not found")),
        Some(passed("sh check.sh", 2)),
        Some(passed("sh tests/{test}.sh", 1)),
    );
    let (profile, dropped) = apply_verification(&proposed(), &v, &root());
    assert_eq!(profile.setup, None);
    assert_eq!(profile.check.as_deref(), Some("sh check.sh"));
    assert_eq!(profile.single_test.as_deref(), Some("sh tests/{test}.sh"));
    assert_eq!(profile.test_passed.as_deref(), Some("PASS {test}"));
    assert_eq!(profile.sample_test.as_deref(), Some("t_ok"));
    assert_eq!(
        dropped,
        vec![DroppedCommand {
            key: "setup".into(),
            command: "sh setup.sh".into(),
            reason: "exit 1 after 1s".into(),
            tail: "setup.sh: not found".into(),
        }]
    );
    // A command the verification has no record of is not proposed either.
    let (profile, dropped) =
        apply_verification(&proposed(), &verification(false, None, None, None), &root());
    assert_eq!(profile.check, None);
    assert!(
        dropped
            .iter()
            .any(|d| d.key == "check" && d.reason == "it was not verified"),
        "{dropped:?}"
    );
    // A timeout says so.
    let mut hung = failed("sh check.sh", 0, "");
    hung.code = None;
    hung.timed_out = true;
    hung.secs = 2;
    let (_, dropped) = apply_verification(
        &RepoProfile {
            check: Some("sh check.sh".into()),
            ..Default::default()
        },
        &verification(false, None, Some(hung), None),
        &root(),
    );
    assert_eq!(dropped[0].reason, "timed out after 2s");
}

#[test]
fn single_test_needs_sample_and_a_matching_line() {
    // Exit 0, but no line matched `test_passed`: all three go, with the reason.
    let mut unmatched = passed("sh tests/{test}.sh", 1);
    unmatched.ok = false;
    unmatched.tail = "ran t_ok".into();
    let v = verification(false, None, Some(passed("sh check.sh", 1)), Some(unmatched));
    let (profile, dropped) = apply_verification(&proposed(), &v, &root());
    assert_eq!(
        (
            &profile.single_test,
            &profile.test_passed,
            &profile.sample_test
        ),
        (&None, &None, &None)
    );
    assert_eq!(profile.check.as_deref(), Some("sh check.sh"));
    assert_eq!(
        dropped.last().unwrap(),
        &DroppedCommand {
            key: "single_test".into(),
            command: "sh tests/{test}.sh".into(),
            reason: "exit 0 after 1s, but no output line matched test_passed for sample_test \
                     t_ok; test_passed and sample_test are dropped with it"
                .into(),
            tail: "ran t_ok".into(),
        }
    );
    // No sample test (or no pattern): nothing to verify it with.
    for strip in ["sample_test", "test_passed"] {
        let mut p = proposed();
        match strip {
            "sample_test" => p.sample_test = None,
            _ => p.test_passed = None,
        }
        let (profile, dropped) = apply_verification(
            &p,
            &verification(false, None, Some(passed("sh check.sh", 1)), None),
            &root(),
        );
        assert_eq!(
            (
                &profile.single_test,
                &profile.test_passed,
                &profile.sample_test
            ),
            (&None, &None, &None),
            "{strip}"
        );
        assert_eq!(
            dropped.last().unwrap().reason,
            format!("{SINGLE_TEST_NEEDS}; test_passed and sample_test are dropped with it")
        );
    }
    // M8a decision 7's rules on the two templates.
    let mut p = proposed();
    p.test_passed = Some("PASS".into());
    let (profile, dropped) =
        apply_verification(&p, &verification(false, None, None, None), &root());
    assert_eq!(profile.single_test, None);
    assert!(
        dropped
            .iter()
            .any(|d| d.key == "single_test"
                && d.reason.starts_with("test_passed: must contain {test}")),
        "{dropped:?}"
    );
    let mut p = proposed();
    p.single_test = Some("sh tests/all.sh".into());
    let (_, dropped) = apply_verification(&p, &verification(false, None, None, None), &root());
    assert!(
        dropped
            .iter()
            .any(|d| d.key == "single_test"
                && d.reason.starts_with("single_test: must contain {test}")),
        "{dropped:?}"
    );
}

#[test]
fn a_confined_drop_carries_the_confinement_hint() {
    let hint = "it ran confined, as runs do: if it needs the network, a cache directory, a Unix \
                socket or a localhost port, allow it for /work/app in your config \
                ([orchestrator.confined_network], [orchestrator.cache_dirs], \
                [orchestrator.confined_unix_sockets], [orchestrator.confined_localhost_ports]), \
                then run anthrex profile detect";
    assert_eq!(confined_hint(&root()), hint);
    let v = |confined| {
        verification(
            confined,
            Some(failed("sh setup.sh", 6, "")),
            Some(failed("sh check.sh", 1, "")),
            Some(failed("sh tests/{test}.sh", 1, "")),
        )
    };
    let (_, dropped) = apply_verification(&proposed(), &v(true), &root());
    let reasons: Vec<(&str, &str)> = dropped
        .iter()
        .map(|d| (d.key.as_str(), d.reason.as_str()))
        .collect();
    assert_eq!(
        reasons[0],
        ("setup", format!("exit 6 after 1s\n{hint}").as_str())
    );
    assert_eq!(
        reasons[1],
        ("check", format!("exit 1 after 1s\n{hint}").as_str())
    );
    assert!(!reasons[2].1.contains("it ran confined"), "{reasons:?}");
    let (_, dropped) = apply_verification(&proposed(), &v(false), &root());
    assert!(
        dropped
            .iter()
            .all(|d| !d.reason.contains("it ran confined")),
        "{dropped:?}"
    );
}

#[test]
fn edit_value_parsing() {
    let stored = RepoProfile::default();
    let (edited, _) = apply_edit(&stored, "check", Some("cargo test")).unwrap();
    assert_eq!(edited.check.as_deref(), Some("cargo test"));
    let (edited, _) = apply_edit(&stored, "modules", Some(r#"["crates/*"]"#)).unwrap();
    assert_eq!(edited.modules, strings(&["crates/*"]));
    let (edited, _) = apply_edit(&stored, "check_timeout_secs", Some("600")).unwrap();
    assert_eq!(edited.check_timeout_secs, Some(600));
    let (edited, _) = apply_edit(&stored, "env.RUST_LOG", Some("debug")).unwrap();
    assert_eq!(
        edited.env.get("RUST_LOG").map(String::as_str),
        Some("debug")
    );
    let with_single = RepoProfile {
        single_test: Some("cargo test -- --exact {test}".into()),
        ..Default::default()
    };
    let (edited, _) = apply_edit(&with_single, "single_test", None).unwrap();
    assert_eq!(edited.single_test, None);
    assert_eq!(
        apply_edit(&stored, "lint", Some("x")),
        Err(format!("unknown key lint; one of {}", EDIT_KEYS.join(", ")))
    );
}

#[test]
fn edit_rejects_invalid_values_with_the_rule() {
    let stored = RepoProfile::default();
    let reason = config::reserved_env::reserved_env("PATH").unwrap();
    for (key, value, message) in [
        (
            "modules",
            r#"["../x/*"]"#,
            "modules: ../x/* must not contain ..".to_string(),
        ),
        (
            "check_timeout_secs",
            "5",
            "check_timeout_secs: must be between 10 and 14400".to_string(),
        ),
        (
            "single_test",
            "cargo test",
            "single_test: must contain {test}".to_string(),
        ),
        (
            "test_passed",
            "ok",
            "test_passed: must contain {test}".to_string(),
        ),
        (
            "env.PATH",
            "/x",
            format!("env: key PATH may not be set by a profile: {reason}"),
        ),
        (
            "env.1X",
            "y",
            "env: key 1X must match [A-Za-z_][A-Za-z0-9_]*".to_string(),
        ),
    ] {
        assert_eq!(apply_edit(&stored, key, Some(value)), Err(message), "{key}");
    }
}

#[test]
fn edit_of_a_command_key_needs_reverification_and_of_modules_does_not() {
    let stored = RepoProfile {
        check: Some("cargo test".into()),
        ..Default::default()
    };
    assert!(
        apply_edit(&stored, "check", Some("cargo test --all"))
            .unwrap()
            .1
    );
    assert!(apply_edit(&stored, "setup", Some("cargo fetch")).unwrap().1);
    assert!(apply_edit(&stored, "check", None).unwrap().1);
    // The same value changes nothing, so nothing re-runs.
    assert!(!apply_edit(&stored, "check", Some("cargo test")).unwrap().1);
    assert!(
        !apply_edit(&stored, "modules", Some(r#"["crates/*"]"#))
            .unwrap()
            .1
    );
    assert!(
        !apply_edit(&stored, "generated", Some(r#"["Cargo.lock"]"#))
            .unwrap()
            .1
    );
}

fn shown_profile() -> RepoProfile {
    let mut profile = RepoProfile {
        languages: strings(&["rust"]),
        protected: strings(&[".cursor/**"]),
        setup: Some("cargo fetch".into()),
        check: Some("cargo build --workspace".into()),
        single_test: Some("cargo test --workspace -- --exact {test}".into()),
        test_passed: Some("test {test} ... ok".into()),
        sample_test: Some("store::tests::round_trip".into()),
        ..Default::default()
    };
    profile.env.insert("RUST_LOG".into(), "debug".into());
    profile
}

#[test]
fn show_text_is_exact() {
    let v = verification(
        true,
        Some(passed("cargo fetch", 3)),
        Some(passed("cargo build --workspace", 214)),
        Some(passed("cargo test --workspace -- --exact {test}", 12)),
    );
    let dropped = vec![DroppedCommand {
        key: "check".into(),
        command: "cargo test --all".into(),
        reason: format!(
            "exit 101 after 30s\n{}",
            confined_hint(Path::new("/work/app"))
        ),
        tail: "error: could not compile\n\nerror: aborting".into(),
    }];
    let hint = confined_hint(Path::new("/work/app"));
    let expected = format!(
        r#"languages = ["rust"]
protected = [".cursor/**"]
setup = "cargo fetch"
check = "cargo build --workspace"
single_test = "cargo test --workspace -- --exact {{test}}"
test_passed = "test {{test}} ... ok"
sample_test = "store::tests::round_trip"
output_filter = "failures-only"

[env]
RUST_LOG = "debug"

# protected: built-in .claude/**, .mcp.json, .codex/**, **/CLAUDE.md, **/AGENTS.md + .cursor/**
# verification 2026-09-21 14:13 (confined)
#   setup        ok     3s   cargo fetch
#   check        ok   214s   cargo build --workspace
#   single_test  ok    12s   cargo test --workspace -- --exact {{test}}  (sample: store::tests::round_trip)
# dropped
#   check: exit 101 after 30s: cargo test --all
#     {hint}
#     error: could not compile
#
#     error: aborting
"#
    );
    assert_eq!(show_text(&shown_profile(), Some(&v), &dropped), expected);

    // Unconfined, with a failed row, and nothing dropped; no verification at all prints
    // only the profile and the protected line.
    let mut v = verification(
        false,
        None,
        Some(failed("cargo build --workspace", 1, "")),
        None,
    );
    v.at = 0;
    let profile = RepoProfile {
        check: Some("cargo build --workspace".into()),
        ..Default::default()
    };
    assert_eq!(
        show_text(&profile, Some(&v), &[]),
        "check = \"cargo build --workspace\"\noutput_filter = \"failures-only\"\n\n\
         # protected: built-in .claude/**, .mcp.json, .codex/**, **/CLAUDE.md, **/AGENTS.md + no extras\n\
         # verification 1970-01-01 00:00 (unconfined)\n\
         #   check        fail   1s   cargo build --workspace\n"
    );
    assert_eq!(
        show_text(&profile, None, &[]),
        "check = \"cargo build --workspace\"\noutput_filter = \"failures-only\"\n\n\
         # protected: built-in .claude/**, .mcp.json, .codex/**, **/CLAUDE.md, **/AGENTS.md + no extras\n"
    );
}

fn preflight(root: &Path) -> Preflight {
    Preflight {
        root: root.to_path_buf(),
        project: root.to_path_buf(),
        git_common_dir: root.join(".git"),
        base_branch: "main".into(),
        base_sha: "0".repeat(40),
        protected_files: Vec::new(),
    }
}

#[test]
fn confine_spec_takes_the_users_tables_for_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("app");
    let other = dir.path().canonicalize().unwrap().join("other");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    let key = root.display().to_string();
    let mut config = config::Orchestrator {
        worker_sandbox: true,
        ..Default::default()
    };
    config
        .cache_dirs
        .insert(key.clone(), strings(&["~/.cargo"]));
    config
        .cache_dirs
        .insert(other.display().to_string(), strings(&["/elsewhere"]));
    config.confined_network.insert(key.clone(), true);
    config
        .confined_unix_sockets
        .insert(key.clone(), strings(&["/tmp/db.sock"]));
    config.confined_localhost_ports.insert(key, vec![5432]);
    let repo_dir = dir.path().join("data/repos/app-12345678");
    let socket = dir.path().join("daemon.sock");
    let pre = preflight(&root);

    let spec = confine_spec(&config, &repo_dir, &pre, &socket);
    if !crate::run::confine::available() {
        assert_eq!(spec, None, "this platform cannot confine");
        return;
    }
    let spec = spec.expect("confined on a platform that can confine");
    assert_eq!(spec.data_dir, repo_dir);
    assert_eq!(spec.common_dir, root.join(".git"));
    assert_eq!(spec.cache_dirs, strings(&["~/.cargo"]));
    assert!(spec.network);
    assert_eq!(spec.unix_sockets, strings(&["/tmp/db.sock"]));
    assert_eq!(spec.localhost_ports, vec![5432]);
    assert_eq!(spec.daemon_socket, socket);

    // Another repository's tables never apply; nothing listed means nothing granted.
    let bare = confine_spec(&config::Orchestrator::default(), &repo_dir, &pre, &socket)
        .expect("workers are sandboxed by default");
    assert!(bare.cache_dirs.is_empty() && !bare.network, "{bare:?}");
    assert!(bare.unix_sockets.is_empty() && bare.localhost_ports.is_empty());
    config.worker_sandbox = false;
    assert_eq!(confine_spec(&config, &repo_dir, &pre, &socket), None);
}

#[test]
fn a_record_of_another_command_does_not_verify_the_proposed_one() {
    // A verification carried forward (an edit's stored record) names the commands it
    // ran; a passing record of a different command verifies nothing.
    let v = verification(
        false,
        Some(passed("sh old-setup.sh", 1)),
        Some(passed("sh old-check.sh", 1)),
        Some(passed("sh old/{test}.sh", 1)),
    );
    let (profile, dropped) = apply_verification(&proposed(), &v, &root());
    assert_eq!(
        (&profile.setup, &profile.check, &profile.single_test),
        (&None, &None, &None)
    );
    let keys: Vec<(&str, &str)> = dropped
        .iter()
        .map(|d| (d.key.as_str(), d.reason.as_str()))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("setup", "it was not verified"),
            ("check", "it was not verified"),
            (
                "single_test",
                "it was not verified; test_passed and sample_test are dropped with it"
            ),
        ]
    );
}

/// Task 10 re-review m2: a checkout pinned as the standalone checkout of another
/// repository is not ours, so no git command runs in it.
#[test]
fn pinned_as_ours_requires_the_pins_git_dir_to_be_the_repositorys() {
    let dir = tempfile::tempdir().unwrap();
    let (ours, other) = (dir.path().join("ours"), dir.path().join("other"));
    for repo in [&ours, &other] {
        std::fs::create_dir_all(repo.join("git")).unwrap();
        std::fs::write(repo.join("git/HEAD"), "ref: refs/heads/main\n").unwrap();
    }
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    let pin_as = |repo: &Path| crate::worktree::pinned::PinAs {
        repo: Some(repo.join("git")),
        ..Default::default()
    };
    crate::worktree::pinned::pin(dir.path(), &checkout, pin_as(&other));
    assert!(!super::verify::pinned_as_ours(&checkout, &ours));
    assert!(super::verify::pinned_as_ours(&checkout, &other));
    crate::worktree::pinned::unpin(&checkout);
}
