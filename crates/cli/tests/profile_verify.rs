//! M8b.10, decision 9: `profile::verify` runs a proposal's commands in a fresh standalone
//! checkout (M8a's `prepare_scratch_in`, `salvage`, `remove_checkout`), confined as a run
//! in the repository would be, on a real repository from `init_repo`. No agent runs
//! here; the commands are plain shell.
//!
//! The confinement tests' payloads are harmless: one tries to write a file named for
//! this test's pid in `$HOME`, which the sandbox must deny (if it does not, the test
//! deletes exactly that file and fails); one runs `curl` with the network off.

mod support;

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use daemon::profile::confined_hint;
use daemon::profile::proposal::apply_verification;
use daemon::profile::verify::SALVAGE_PREFIX;
use daemon::run::git::{checkout_repo_dir, task_tmp};
use proto::RepoProfile;

use support::profile_rig::{COMMAND_TIMEOUT, Rig, check};
use support::run_harness::git_in;
use support::tempdir;

/// The child half of `the_engine_environment_applies` finds its directory here.
const CHILD_DIR: &str = "ANTHREX_M8B10_CHILD_DIR";
/// Agent binaries that exist nowhere: nothing here may reach a real agent.
const NO_AGENT_BIN: &str = "/nonexistent/anthrex-test/agent";

#[test]
fn verification_never_touches_the_checkout_and_salvages_dirt() {
    let rig = Rig::new();
    let head = git_in(&rig.repo, &["rev-parse", "HEAD"]);
    let verified = rig.verify(RepoProfile {
        single_test: Some("sh tests/{test}.sh".into()),
        test_passed: Some("PASS {test}".into()),
        sample_test: Some("t_ok".into()),
        ..check("printf changed > README && printf new > untracked.txt")
    });
    let v = &verified.verification;
    assert!(v.check.as_ref().is_some_and(|c| c.ok), "{v:?}");
    assert!(v.single_test.as_ref().is_some_and(|c| c.ok), "{v:?}");

    // The user's checkout: clean, same HEAD, no linked worktree.
    assert_eq!(
        git_in(&rig.repo, &["status", "--porcelain", "--ignored"]),
        ""
    );
    assert_eq!(git_in(&rig.repo, &["rev-parse", "HEAD"]), head);
    assert!(!rig.pre.git_common_dir.join("worktrees").exists());
    assert_eq!(
        git_in(&rig.repo, &["worktree", "list", "--porcelain"])
            .lines()
            .filter(|line| line.starts_with("worktree "))
            .count(),
        1
    );
    // The checkout, its repository and its TMPDIR are gone.
    let repo = checkout_repo_dir(&rig.repo_dir, &rig.checkout());
    assert!(!rig.checkout().exists(), "{}", rig.checkout().display());
    assert!(!repo.exists(), "{}", repo.display());
    assert!(!task_tmp(&repo).exists());
    assert_eq!(
        repo,
        rig.repo_dir.join("tasks/.profile-verify"),
        "the checkout's repository is in anthrex's data directory"
    );

    // The dirt was salvaged into a ref, and only there.
    assert_eq!(verified.salvaged.len(), 1, "{verified:?}");
    let reference = &verified.salvaged[0];
    assert!(reference.starts_with(SALVAGE_PREFIX), "{reference}");
    let refs = git_in(
        &rig.repo,
        &["for-each-ref", "--format=%(refname)", SALVAGE_PREFIX],
    );
    assert_eq!(refs, *reference);
    assert_eq!(
        git_in(&rig.repo, &["show", &format!("{reference}:README")]),
        "changed"
    );
    assert_eq!(
        git_in(&rig.repo, &["show", &format!("{reference}:untracked.txt")]),
        "new"
    );
    assert_eq!(
        git_in(&rig.repo, &["rev-parse", &format!("{reference}^")]),
        head
    );

    // A second verification starts fresh at HEAD, not from the first one's files.
    let verified = rig.verify(check("test \"$(cat README)\" = readme"));
    assert!(
        verified.verification.check.as_ref().is_some_and(|c| c.ok),
        "{verified:?}"
    );
    assert!(verified.salvaged.is_empty(), "{verified:?}");
}

#[test]
fn a_hanging_verification_command_times_out_and_is_dropped() {
    let rig = Rig::new();
    let confine = rig.confine(&config::Orchestrator::default());
    let verified = rig.verify_with(check("sleep 60"), confine, Duration::from_secs(2));
    let v = &verified.verification;
    let c = v.check.as_ref().expect("the check ran");
    assert!(c.timed_out && !c.ok && c.code.is_none(), "{c:?}");
    let (profile, dropped) = apply_verification(&check("sleep 60"), v, &rig.pre.root);
    assert_eq!(profile.check, None);
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert!(
        dropped[0].reason.starts_with("timed out after "),
        "{dropped:?}"
    );
    assert!(!rig.checkout().exists());
}

#[test]
fn env_and_worktree_are_substituted() {
    let rig = Rig::new();
    let mut profile = check("echo \"target=$CARGO_TARGET_DIR\"");
    profile
        .env
        .insert("CARGO_TARGET_DIR".into(), "{worktree}/target".into());
    let verified = rig.verify(profile);
    let c = verified.verification.check.expect("the check ran");
    assert!(c.ok, "{c:?}");
    let expected = format!("target={}/target", rig.checkout().display());
    assert!(
        c.tail.lines().any(|line| line == expected),
        "{expected} not in {:?}",
        c.tail
    );
}

#[test]
fn the_engine_environment_applies() {
    let dir = tempdir();
    let root = dir.path().canonicalize().unwrap();
    // The call runs in a child copy of this test binary, alone, whose command (and only
    // that command) carries the variables: this process's environment never changes.
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "engine_environment_child",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIR, &root)
        .env("CLAUDE_CODE_ENTRYPOINT", "cli")
        .env("CLAUDE_CODE_SSE_PORT", "1")
        .env("ANTHROPIC_API_KEY", "sk-ant-not-a-real-key")
        .env("ANTHREX_SOCKET", root.join("no-daemon.sock"))
        .env("ANTHREX_M8B10_KEPT", "kept")
        .env("ANTHREX_CLAUDE_BIN", NO_AGENT_BIN)
        .env("ANTHREX_CODEX_BIN", NO_AGENT_BIN)
        .env("ANTHREX_DECIDER_BIN", NO_AGENT_BIN)
        .env("M8B10_MARKER", "inherited")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let recorded = std::fs::read_to_string(root.join("env.txt")).unwrap();
    let (confined, lines) = recorded.split_once('\n').unwrap();
    let lines: Vec<&str> = lines.lines().collect();
    // The recording works: the child's own marker reached the check ...
    assert!(lines.contains(&"M8B10_MARKER=inherited"), "{lines:?}");
    // ... and no agent variable or API credential did.
    for line in &lines {
        assert!(
            !line.starts_with("CLAUDE_CODE_") && !line.starts_with("ANTHROPIC_API_KEY="),
            "{line} reached a verification command"
        );
    }
    // Confined, every `ANTHREX_*` goes (M8a F1c round 3, N1).
    if confined == "confined" {
        assert!(
            !lines.iter().any(|line| line.starts_with("ANTHREX_")),
            "{lines:?}"
        );
    } else {
        println!("unconfined on this platform: ANTHREX_* is kept, as M8a keeps it");
    }
}

/// The child half of `the_engine_environment_applies`; does nothing unless that test
/// started it.
#[test]
#[ignore = "run by the_engine_environment_applies"]
fn engine_environment_child() {
    let Some(dir) = std::env::var_os(CHILD_DIR) else {
        return;
    };
    let root = PathBuf::from(dir);
    let rig = Rig::in_dir(&root);
    let verified = rig.verify(check(
        "env | grep -E '^(CLAUDE|ANTHROPIC|ANTHREX_|M8B10_)'; true",
    ));
    let c = verified.verification.check.expect("the check ran");
    assert!(c.ok, "{c:?}");
    let confined = if verified.verification.confined {
        "confined"
    } else {
        "unconfined"
    };
    std::fs::write(root.join("env.txt"), format!("{confined}\n{}", c.tail)).unwrap();
}

#[test]
fn verification_uses_the_users_confinement_not_the_proposal() {
    if !cfg!(target_os = "macos") {
        println!("skipped: only macOS can confine a command");
        return;
    }
    let rig = Rig::new();
    let cache = rig.root.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let mut config = config::Orchestrator::default();
    config.cache_dirs.insert(
        rig.pre.root.display().to_string(),
        vec![cache.display().to_string()],
    );
    let confine = rig.confine(&config);
    assert!(confine.is_some(), "workers are sandboxed by default");

    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is set"));
    let pwned = home.join(format!("pwned-{}", std::process::id()));
    assert!(!pwned.exists(), "{} is already there", pwned.display());
    // A proposal cannot widen the confinement: `RepoProfile` has no confinement key, and
    // an `env` naming `HOME`/`TMPDIR` at the home directory changes nothing.
    let mut home_write = check(&format!(
        "printf x > \"$HOME/pwned-{pid}\"; printf x > \"$TMPDIR/pwned-{pid}\"; \
         printf x > '{home}/pwned-{pid}'",
        pid = std::process::id(),
        home = home.display()
    ));
    for key in ["HOME", "TMPDIR"] {
        home_write
            .env
            .insert(key.into(), home.display().to_string());
    }
    let verified = rig.verify_with(home_write.clone(), confine.clone(), COMMAND_TIMEOUT);
    let written = pwned.exists();
    if written {
        std::fs::remove_file(&pwned).unwrap();
    }
    assert!(
        !written,
        "a confined verification wrote {}",
        pwned.display()
    );
    let (profile, dropped) = apply_verification(&home_write, &verified.verification, &rig.pre.root);
    assert_eq!(profile.check, None, "{dropped:?}");
    assert!(
        dropped[0].reason.contains(&confined_hint(&rig.pre.root)),
        "{dropped:?}"
    );

    let cache_write = check(&format!("printf x > '{}/ok'", cache.display()));
    let verified = rig.verify_with(cache_write.clone(), confine, COMMAND_TIMEOUT);
    let (profile, dropped) =
        apply_verification(&cache_write, &verified.verification, &rig.pre.root);
    assert!(dropped.is_empty(), "{dropped:?}");
    assert_eq!(profile.check, cache_write.check);
    assert_eq!(std::fs::read_to_string(cache.join("ok")).unwrap(), "x");
}

#[test]
fn a_confined_setup_without_network_is_dropped_with_the_hint() {
    if !cfg!(target_os = "macos") {
        println!("skipped: only macOS can confine a command");
        return;
    }
    let rig = Rig::new();
    let proposed = RepoProfile {
        setup: Some("curl -s https://example.com".into()),
        ..check("true")
    };
    let verified = rig.verify(proposed.clone());
    assert!(verified.verification.confined, "{verified:?}");
    let (profile, dropped) = apply_verification(&proposed, &verified.verification, &rig.pre.root);
    assert_eq!(profile.setup, None);
    assert_eq!(profile.check.as_deref(), Some("true"));
    assert_eq!(dropped.len(), 1, "{dropped:?}");
    assert_eq!(dropped[0].key, "setup");
    assert!(dropped[0].reason.starts_with("exit "), "{dropped:?}");
    assert!(
        dropped[0]
            .reason
            .ends_with(&format!("\n{}", confined_hint(&rig.pre.root))),
        "{dropped:?}"
    );
}

#[test]
fn single_test_exit_zero_without_its_line_is_not_ok() {
    let rig = Rig::new();
    let proposed = RepoProfile {
        single_test: Some("sh tests/{test}.sh".into()),
        test_passed: Some("DONE {test}".into()),
        sample_test: Some("t_ok".into()),
        ..Default::default()
    };
    let verified = rig.verify(proposed.clone());
    let c = verified.verification.single_test.clone().expect("it ran");
    assert_eq!((c.ok, c.code), (false, Some(0)), "{c:?}");
    assert!(c.tail.contains("PASS t_ok"), "{c:?}");
    let (profile, dropped) = apply_verification(&proposed, &verified.verification, &rig.pre.root);
    assert_eq!(profile.single_test, None);
    assert!(
        dropped[0].reason.starts_with("exit 0 after "),
        "{dropped:?}"
    );
}

#[test]
fn reserved_env_keys_of_a_proposal_never_reach_a_command() {
    let rig = Rig::new();
    let mut proposed =
        check("env | grep -E '^(TMPDIR|HOME|GIT_DIR|ANTHREX_SOCKET|M8B10_OK)='; true");
    for key in ["TMPDIR", "HOME", "GIT_DIR", "ANTHREX_SOCKET"] {
        assert!(config::reserved_env::reserved_env(key).is_some(), "{key}");
        proposed.env.insert(key.into(), "/nonexistent/m8b10".into());
    }
    proposed.env.insert("M8B10_OK".into(), "kept".into());
    let verified = rig.verify(proposed);
    let c = verified.verification.check.expect("the check ran");
    assert!(c.ok, "{c:?}");
    assert!(c.tail.lines().any(|line| line == "M8B10_OK=kept"), "{c:?}");
    assert!(!c.tail.contains("/nonexistent/m8b10"), "{c:?}");
}
