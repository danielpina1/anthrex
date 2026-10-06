//! Task M9.7.6 (DH §1.2, decision 6): the base fetch's `contains` steps run with
//! `GIT_DIR` (and the other repository variables) set in the daemon's own environment,
//! and those have no effect: every git call is scrubbed (AGENTS.md rule 11).
//!
//! Alone in its own test binary, for the reason `worktree_env.rs` gives at length: it
//! changes the process environment, which is sound only with no other thread of the
//! binary spawning a process (or otherwise reading `environ`) at the same time. Keep
//! this file to this one test. Its other checks are `src/host/tests_git_contains.rs`.

use std::path::Path;
use std::process::Command;

use daemon::host::{CodeHost, Contains, FetchOutcome, FetchReq, GhHost, HostRepo, SystemRunner};

const RUN: &str = "r1a2b";
const NO_GH: &str = "/nonexistent/anthrex-test/gh";
const SCRUBBED: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_COMMON_DIR",
    "GIT_INDEX_FILE",
    "GIT_PREFIX",
];

/// The test's own setup git: identity, signing and hooks given per command, and the
/// repository variables removed, whatever the environment holds.
fn git(dir: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=anthrex test",
            "-c",
            "user.email=test@anthrex.invalid",
            "-c",
            "commit.gpgSign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args);
    for key in SCRUBBED {
        command.env_remove(key);
    }
    let out = command.output().unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, message: &str) -> String {
    git(dir, &["commit", "-q", "--allow-empty", "-m", message]);
    git(dir, &["rev-parse", "HEAD"])
}

#[test]
fn the_contains_steps_ignore_the_daemons_git_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let (bare, work, user, decoy) = (
        tmp.path().join("remote.git"),
        tmp.path().join("work"),
        tmp.path().join("user"),
        tmp.path().join("decoy.git"),
    );
    for dir in [&bare, &work, &decoy] {
        std::fs::create_dir_all(dir).unwrap();
    }
    git(&bare, &["init", "-q", "--bare"]);
    git(&decoy, &["init", "-q", "--bare"]);
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["remote", "add", "origin", &bare.to_string_lossy()]);
    commit(&work, "a");
    git(&work, &["push", "-q", "origin", "main"]);
    let stage = format!("anthrex/{RUN}/stage-1");
    git(&work, &["checkout", "-q", "-b", &stage]);
    let h2 = commit(&work, "h2 (anthrex's fix)");
    git(&work, &["push", "-q", "origin", &stage]);
    git(&work, &["checkout", "-q", "main"]);
    // The user's commit on the stage branch, and a squash merge, from another clone.
    git(
        tmp.path(),
        &["clone", "-q", &bare.to_string_lossy(), "user"],
    );
    git(
        &user,
        &["checkout", "-q", "-b", "s", &format!("origin/{stage}")],
    );
    let u2 = commit(&user, "u2");
    git(
        &user,
        &["push", "-q", "origin", &format!("HEAD:refs/heads/{stage}")],
    );
    git(&user, &["checkout", "-q", "main"]);
    git(&user, &["merge", "-q", "--squash", "s"]);
    let squash = commit(&user, "squash of stage 1");
    git(&user, &["push", "-q", "origin", "main"]);

    let host = GhHost::new(SystemRunner::new(NO_GH, "git"), NO_GH, "git");
    let into = format!("refs/anthrex/{RUN}/remote/stage-1");
    let req = FetchReq {
        repo: HostRepo {
            host: "github.com".to_string(),
            owner: "o".to_string(),
            name: "r".to_string(),
            remote: "origin".to_string(),
            root: work.clone(),
        },
        run_id: RUN.to_string(),
        branch: "main".to_string(),
        into: format!("refs/anthrex/{RUN}/remote/base"),
        adopt: None,
        parents_of: Some(squash.clone()),
        contains: Some(Contains {
            stage: 1,
            branch: stage.clone(),
            into: into.clone(),
            head: h2,
            merged: u2.clone(),
            pr: None,
        }),
        deadline: None,
    };

    let previous: Vec<(&str, Option<std::ffi::OsString>)> = SCRUBBED
        .iter()
        .map(|key| (*key, std::env::var_os(key)))
        .collect();
    // SAFETY: this is the only test in this binary (see the module doc comment), so no
    // other thread spawns a process or reads the environment while it changes.
    unsafe {
        std::env::set_var("GIT_DIR", &decoy);
        std::env::set_var("GIT_WORK_TREE", tmp.path());
        std::env::set_var("GIT_COMMON_DIR", &decoy);
        std::env::set_var("GIT_INDEX_FILE", tmp.path().join("decoy-index"));
        std::env::set_var("GIT_PREFIX", "decoy/");
    }
    let answer = host.fetch(&req);
    // SAFETY: as above.
    unsafe {
        for (key, value) in &previous {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }

    assert_eq!(
        answer,
        Ok(FetchOutcome::Fetched {
            sha: squash,
            parents: Some(1),
            contains: Some(true),
        })
    );
    assert_eq!(
        git(&work, &["rev-parse", &into]),
        u2,
        "the work repository's ref"
    );
    let decoy_refs = git(&decoy, &["for-each-ref", "--format=%(refname)"]);
    assert_eq!(decoy_refs, "", "nothing reached the decoy repository");
}
